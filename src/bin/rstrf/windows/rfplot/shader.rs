//! This module contains the WGPU shader implementation for the RFPlot widget. The shader is
//! responsible for rendering the spectrogram itself.
//!
//! The shader code is in [shader.wgsl](shader.wgsl).
//!
//! The shader uses four buffers:
//! - `color_map`: storage buffer containing the colormap
//! - `uniforms`: uniform buffer containing bounds, viewport information, buffer sizes etc.
//! - `spec_data`: storage buffer containing the spectrogram data itself (`array<f32>`), **mipmapped
//!   on the frequency axis**.
//! - `x_ranges`: storage buffer containing the x coordinates of each slice (`array<vec2<f32>>`, for
//!   left and right edge)
//!
//! The spectrogram data is chunked to not exceed `max_storage_buffer_binding_size` (128MiB by
//! default, which iced currently leaves it at). There are two binding groups:
//! - `bind_group(0)`: contains the colormap buffer and is switched per primitive (i.e. per RFPlot)
//! - `bind_group(1)`: contains the other three buffers and is switched per spectrogram chunk.
//!
//! # Vertex shader
//! Each slice is rendered as a quad (i.e. two triangles). This allows rendering each slice at the
//! correct position & length, and properly rendering gaps in the recording (although sub-pixel gaps
//! are eliminated via pixel snapping).
//!
//! The vertex shader gets the corner coordinates ([01], [01]). It transforms the coordinates to the
//! correct slice position using the `x_ranges` buffer & view bounds, and sets the u/v coordinates
//! for texture mapping.
//!
//! The transformation can leave slices off-screen, which should then be clipped before the fragment
//! shader ever runs.
//!
//! # Fragment shader
//! The fragment shader is essentially a custom texture sampler. Typical spectrograms can be hours
//! long and contain tens of thousands of channels. That means that when the view is zoomed out,
//! many samples alias to the same pixel. The fragment shader is designed to avoid aliasing
//! artifacts, and also to bring out narrow-band signals.
//!
//! To avoid extreme supersampling when zoomed out, there is a mipmap. Since we zoom independently
//! on the x and y axes, we cannot really use an isotropic mipmap. A log-2 anisotropic mipmap would
//! be ideal, but cost 4x the VRAM. Since a typical usecase is about 1h at 1Hz integration time, but
//! with maybe 80000 channels, the supersampling on the frequency axis is an order of magnitude
//! worse than on the time axis. So we use a mipmap only on the frequency axis. To keep the VRAM
//! cost at ~33% increase, we use a log-4 mipmap.
//!
//! That leaves the fragment shader with up to 4x supersampling, but that's an acceptable
//! performance penalty. There are two modes for the supersampling (and mipmap generation):
//! - max-hold: take the maximum of the samples
//! - average: take the average of the samples
//!
//! Max-hold is the default, since it helps with bringing out narrow-band signals. To achieve
//! max-hold on the x axis (where each slice is its own quad), the fragment shader has a depth
//! output and renders pixels from slices with higher power over lower-power pixels.
//!
//! Depending on the recording setup, there can be many narrow-band interference signals that
//! visually clutter a zoomed out max-hold plot. To avoid this, there is also an "average" plotting
//! mode, where the fragment shader computes the average over its samples instead of the maximum.
//! There is still max-hold on the x axis, but typically there is less aliasing there.
use std::{collections::HashMap, sync::Arc};

use glam::{Vec2, vec2};
use iced::{
    Rectangle, Size, mouse,
    wgpu::{self, util::DeviceExt},
    widget::shader,
};
use itertools::{Itertools, izip};
use rstrf::{colormap::Colormap, spectrogram::Spectrogram};
use uuid::Uuid;

use super::{Controls, Message, RFPlot};

const MIPMAP_FACTOR: f64 = 4.0;
/// Due to the mipmap, the size of the `spec_data` buffer must increase by
/// sum(1/4^n for n=0..inf) = 4/3
const MIPMAP_BUFFER_FACTOR: f64 = 4.0 / 3.0;

fn mipmap_buffer_size(normal_size: usize) -> usize {
    (normal_size as f64 * MIPMAP_BUFFER_FACTOR).ceil() as usize
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Uniforms {
    power_bounds: Vec2,
    time_bounds: Vec2,
    freq_bounds: Vec2,
    pixel_height: f32,
    viewport_width: f32,
    nslices: u32,
    nchan: u32,
    average: u32,
    buf_offset: u32,
}

const _: () = assert!(std::mem::size_of::<Uniforms>() % std::mem::size_of::<Vec2>() == 0);

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

struct DepthTarget {
    view: wgpu::TextureView,
    size: Size<u32>,
}

struct SpectrogramChunk {
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    nslices: u32,
}

struct Buffers {
    colormap: wgpu::Buffer,
    colormap_bind: wgpu::BindGroup,
    spectrogram: Vec<SpectrogramChunk>,
}

struct PrimitiveData {
    buffers: Buffers,
    spectrogram_id: Uuid,
    colormap: Colormap,
    depth: DepthTarget,
}

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    instances: HashMap<Uuid, PrimitiveData>,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spectrogram.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader.wgsl"
            ))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spectrogram.pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vec2>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<u32>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![1 => Uint32],
                    },
                ],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            instances: HashMap::new(),
        }
    }
}

impl Pipeline {
    fn update_buffers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        primitive: &Primitive,
        viewport_bounds: &Rectangle,
        physical_size: Size<u32>,
    ) {
        let Some(spectrogram) = &primitive.spectrogram else {
            return;
        };

        let is_new_entry = !self.instances.contains_key(&primitive.id);
        let primitive_data = self.instances.entry(primitive.id).or_insert_with_key(|id| {
            Self::create_buffers(
                device,
                queue,
                &self.pipeline,
                id,
                spectrogram,
                primitive.controls.colormap(),
                physical_size,
                primitive.controls.average_plotting(),
            )
        });

        let bounds = primitive.controls.bounds();
        let pixel_height = bounds.0.height / viewport_bounds.height * spectrogram.nchan as f32;
        let max_level = (spectrogram.nchan as f32 / 256.0)
            .log(MIPMAP_FACTOR as f32)
            .floor() as u32;
        let mipmap_level = if pixel_height.is_finite() {
            pixel_height.log(MIPMAP_FACTOR as f32).floor() as u32
        } else {
            0
        }
        .min(max_level);
        let mipmap_stride = MIPMAP_FACTOR.powi(mipmap_level as i32) as usize;
        let nchan = (spectrogram.nchan / mipmap_stride) as u32;
        let pixel_height = pixel_height / mipmap_stride as f32;
        log::trace!(
            "Updating buffers for primitive {} (mipmap level {}, {} channels, pixel height {})",
            primitive.id,
            mipmap_level,
            nchan,
            pixel_height
        );

        let xmin = bounds.0.x;
        let xmax = bounds.0.x + bounds.0.width;
        let vmin = bounds.0.y;
        let vmax = bounds.0.y + bounds.0.height;

        let mut buf_offset_chan = 0;
        for i in 0..mipmap_level {
            buf_offset_chan += spectrogram.nchan / MIPMAP_FACTOR.powi(i as i32) as usize;
        }

        for chunk in primitive_data.buffers.spectrogram.iter_mut() {
            let uniforms = Uniforms {
                power_bounds: primitive.controls.power_range().into(),
                time_bounds: vec2(xmin, xmax),
                freq_bounds: vec2(vmin, vmax),
                nslices: chunk.nslices,
                nchan,
                pixel_height,
                viewport_width: viewport_bounds.width,
                average: primitive.controls.average_plotting() as u32,
                buf_offset: buf_offset_chan as u32 * chunk.nslices,
            };
            queue.write_buffer(&chunk.uniform, 0, bytemuck::bytes_of(&uniforms));
        }

        // TODO: do we need to track primitive ID & spectrogram ID separately?
        if is_new_entry {
            // Buffers were already created inside create_buffers; just signal upload done.
            if let Some(notify) = &primitive.gpu_notify {
                notify.notify_one();
            }
        } else if primitive_data.spectrogram_id != spectrogram.id {
            primitive_data.buffers.spectrogram = Self::create_spectrogram_buffers(
                device,
                queue,
                &self.pipeline,
                spectrogram,
                primitive.controls.average_plotting(),
            );
            primitive_data.spectrogram_id = spectrogram.id;
            if let Some(notify) = &primitive.gpu_notify {
                notify.notify_one();
            }
        }
        if primitive_data.depth.size != physical_size {
            primitive_data.depth = Self::create_depth_target(
                device,
                physical_size,
                &format!("spectrogram.{}", primitive.id),
            );
        }

        if primitive_data.colormap != primitive.controls.colormap() {
            queue.write_buffer(
                &primitive_data.buffers.colormap,
                0,
                bytemuck::cast_slice(primitive.controls.colormap().buffer()),
            );
            primitive_data.colormap = primitive.controls.colormap();
        }
    }

    fn create_buffers(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        id: &Uuid,
        spectrogram: &Spectrogram,
        colormap: Colormap,
        physical_size: Size<u32>,
        average: bool,
    ) -> PrimitiveData {
        let prefix = format!("spectrogram.{}", id);
        let colormap_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(format!("{prefix}.buffer.colormap").as_str()),
            contents: bytemuck::cast_slice(colormap.buffer()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let colormap_bind_group_layout = pipeline.get_bind_group_layout(0);
        let colormap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(format!("{prefix}.bind_group.colormap").as_str()),
            layout: &colormap_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: colormap_buffer.as_entire_binding(),
            }],
        });

        let spectrogram_id = spectrogram.id;
        let spectrogram =
            Self::create_spectrogram_buffers(device, queue, pipeline, spectrogram, average);

        PrimitiveData {
            buffers: Buffers {
                colormap: colormap_buffer,
                colormap_bind: colormap_bind_group,
                spectrogram,
            },
            spectrogram_id,
            colormap,
            depth: Self::create_depth_target(device, physical_size, &prefix),
        }
    }

    fn create_spectrogram_buffers(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        spectrogram: &Spectrogram,
        average: bool,
    ) -> Vec<SpectrogramChunk> {
        let limits = device.limits();
        let max_buf_size =
            (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size) as usize;
        let data = spectrogram.data.as_slice().unwrap();
        let slice_size = mipmap_buffer_size(spectrogram.nchan * std::mem::size_of::<f32>());
        let max_chunk_len = max_buf_size / slice_size;
        let chunk_len = spectrogram.nslices.min(max_chunk_len);
        if chunk_len == 0 {
            log::error!(
                "Spectrogram is too large to render ({} bytes per slice, max buffer size is {})",
                slice_size,
                max_buf_size
            );
            return Vec::new();
        }

        let prefix = format!("spectrogram.{}", spectrogram.id);
        let start_time = spectrogram.start_time();
        let timestamps = spectrogram
            .timestamps
            .iter()
            .map(|t| (*t - start_time).as_seconds_f32());
        let length = spectrogram.length().as_seconds_f32();
        let x_ranges = izip!(timestamps, spectrogram.lengths.iter())
            .map(|(t, len)| {
                let left = t / length;
                let right = (t + len) / length;
                vec2(left, right)
            })
            .collect_vec();

        izip!(
            data.chunks(chunk_len * spectrogram.nchan),
            x_ranges.chunks(chunk_len),
        )
        .enumerate()
        .map(|(i, (chunk, x_ranges_chunk))| {
            let prefix = format!("{}.chunk{}", prefix, i);
            log::debug!(
                "Creating chunk {} ({} bytes)",
                prefix,
                std::mem::size_of_val(chunk)
            );
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(format!("{prefix}.buffer.vertex").as_str()),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                contents: bytemuck::cast_slice(&[
                    vec2(0.0, 0.0),
                    vec2(1.0, 0.0),
                    vec2(0.0, 1.0),
                    vec2(1.0, 0.0),
                    vec2(1.0, 1.0),
                    vec2(0.0, 1.0),
                ]),
            });
            let nslices = (chunk.len() / spectrogram.nchan) as u64;
            let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(format!("{prefix}.buffer.instance").as_str()),
                contents: bytemuck::cast_slice(&(0..nslices as u32).collect::<Vec<_>>()),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });

            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(format!("{prefix}.buffer.uniform").as_str()),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let spectrogram_buffer = Self::create_spectrogram_buffer(
                device,
                chunk,
                nslices as usize,
                spectrogram.nchan,
                &format!("{prefix}.buffer.spectrogram"),
                average,
            );

            let x_ranges_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(format!("{prefix}.buffer.x_ranges").as_str()),
                contents: bytemuck::cast_slice(x_ranges_chunk),
                usage: wgpu::BufferUsages::STORAGE,
            });

            let bind_group_layout = pipeline.get_bind_group_layout(1);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(format!("{prefix}.bind_group.spectrogram").as_str()),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: spectrogram_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: x_ranges_buffer.as_entire_binding(),
                    },
                ],
            });
            // `create_buffer_init` parks a host-visible staging copy of `chunk` in the queue's
            // pending writes until the next submit. Flush + wait here so that staging memory is
            // reclaimed before we build the next chunk, capping the transient overhead at one chunk
            // instead of holding a full second copy of the whole spectrogram in RAM until the next
            // frame happens to be rendered.
            queue.submit(std::iter::empty());
            let _ = device.poll(wgpu::PollType::wait_indefinitely());

            SpectrogramChunk {
                uniform: uniform_buffer,
                vertices: vertex_buffer,
                instances: instance_buffer,
                bind_group,
                nslices: nslices as u32,
            }
        })
        .collect()
    }

    fn create_spectrogram_buffer(
        device: &wgpu::Device,
        data: &[f32],
        nslices: usize,
        mut nchan: usize,
        label: &str,
        average: bool,
    ) -> wgpu::Buffer {
        let data_size = std::mem::size_of_val(data);
        let unpadded_size = mipmap_buffer_size(data_size);
        // Adapted from wgpu::util::DeviceExt::create_buffer_init()
        let align_mask = wgpu::COPY_BUFFER_ALIGNMENT - 1;
        let padded_size =
            ((unpadded_size as u64 + align_mask) & !align_mask).max(wgpu::COPY_BUFFER_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: padded_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        {
            // Need to drop the mapped views before unmapping
            let mut view = buffer.slice(..).get_mapped_range_mut();
            let floats: &mut [f32] = bytemuck::cast_slice_mut(&mut view[..]);
            floats[..data.len()].copy_from_slice(bytemuck::cast_slice(data));
            // TODO: Can we compute the mipmaps in a compute shader instead?
            let mut prev_mipmap = data;
            let mut mipmap;
            let mut mipmap_offset = data.len();
            while nchan >= MIPMAP_FACTOR as usize {
                log::debug!(
                    "Computing mipmap for {} slices, {} channels (offset {})",
                    nslices,
                    nchan / MIPMAP_FACTOR as usize,
                    mipmap_offset
                );
                // TODO: We need to recompute the mipmap if the average/max-hold mode changes...
                mipmap = Self::compute_mipmap(prev_mipmap, nslices, nchan, average);
                nchan /= MIPMAP_FACTOR as usize;
                floats[mipmap_offset..(mipmap_offset + mipmap.len())].copy_from_slice(&mipmap);
                mipmap_offset += mipmap.len();
                prev_mipmap = &mipmap;
            }
        }
        buffer.unmap();
        buffer
    }

    fn compute_mipmap(data: &[f32], nslices: usize, nchan_in: usize, average: bool) -> Vec<f32> {
        let stride = MIPMAP_FACTOR as usize;
        let mipmap_size = data.len() / stride;
        let mut mipmap = Vec::with_capacity(mipmap_size);
        let nchan_out = nchan_in / stride;
        for x in 0..nslices {
            for y in 0..nchan_out {
                let mut agg = if average { 0.0 } else { f32::MIN };
                for dy in 0..stride {
                    let y_idx = y * stride + dy;
                    let val = data[x * nchan_in + y_idx];
                    if average {
                        agg += val;
                    } else {
                        agg = agg.max(val);
                    }
                }
                if average {
                    mipmap.push(agg / stride as f32);
                } else {
                    mipmap.push(agg);
                }
            }
        }
        mipmap
    }

    fn create_depth_target(
        device: &wgpu::Device,
        size: Size<u32>,
        name_prefix: &str,
    ) -> DepthTarget {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("{}.depth", name_prefix)),
            size: wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        DepthTarget { view, size }
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
        id: &Uuid,
    ) {
        let Some(primitive_data) = self.instances.get(id) else {
            return;
        };
        let depth = &primitive_data.depth;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(format!("spectrogram.pipeline.pass.{}", id).as_str()),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_viewport(
            clip_bounds.x as f32,
            clip_bounds.y as f32,
            clip_bounds.width as f32,
            clip_bounds.height as f32,
            0.0,
            1.0,
        );

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &primitive_data.buffers.colormap_bind, &[]);
        for chunk in &primitive_data.buffers.spectrogram {
            pass.set_vertex_buffer(0, chunk.vertices.slice(..));
            pass.set_vertex_buffer(1, chunk.instances.slice(..));
            pass.set_bind_group(1, &chunk.bind_group, &[]);
            pass.draw(0..6, 0..chunk.nslices);
        }
    }
}

#[derive(Debug)]
pub struct Primitive {
    id: uuid::Uuid,
    controls: Controls,
    spectrogram: Option<Spectrogram>,
    gpu_notify: Option<Arc<tokio::sync::Notify>>,
}

impl Primitive {
    fn new(
        id: uuid::Uuid,
        controls: Controls,
        spectrogram: Option<Spectrogram>,
        gpu_notify: Option<Arc<tokio::sync::Notify>>,
    ) -> Self {
        Self {
            id,
            controls,
            spectrogram,
            gpu_notify,
        }
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &iced::wgpu::Device,
        queue: &iced::wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        pipeline.update_buffers(device, queue, self, bounds, viewport.physical_size());
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        pipeline.render(encoder, target, clip_bounds, &self.id);
    }
}

impl shader::Program<Message> for RFPlot {
    type State = ();
    type Primitive = Primitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        Primitive::new(
            self.id,
            self.shared.controls,
            self.shared.spectrogram.clone(),
            self.gpu_notify.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_HOLD: bool = false;
    const AVERAGE: bool = true;

    /// `data[x][y] = x * 100 + y`, so every value identifies its slice and channel.
    fn ramp(nslices: usize, nchan: usize) -> Vec<f32> {
        (0..nslices)
            .flat_map(|x| (0..nchan).map(move |y| (x * 100 + y) as f32))
            .collect()
    }

    fn assert_close(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len(), "length mismatch");
        for (i, (a, e)) in izip!(actual, expected).enumerate() {
            assert!((a - e).abs() < 1e-3, "bin {}: {} != {}", i, a, e);
        }
    }

    #[test]
    fn max_hold_takes_maximum_of_each_group() {
        let data = ramp(1, 8);
        let mipmap = Pipeline::compute_mipmap(&data, 1, 8, MAX_HOLD);
        assert_close(&mipmap, &[3.0, 7.0]);
    }

    #[test]
    fn average_takes_mean_of_each_group() {
        let data = ramp(1, 8);
        let mipmap = Pipeline::compute_mipmap(&data, 1, 8, AVERAGE);
        assert_close(&mipmap, &[1.5, 5.5]);
    }

    #[test]
    fn output_length_is_nslices_times_floored_nchan() {
        for (nslices, nchan) in [(1, 8), (3, 10), (7, 63), (5, 4)] {
            let data = ramp(nslices, nchan);
            let mipmap = Pipeline::compute_mipmap(&data, nslices, nchan, MAX_HOLD);
            assert_eq!(
                mipmap.len(),
                nslices * (nchan / 4),
                "nslices={}, nchan={}",
                nslices,
                nchan
            );
        }
    }

    /// Regression: the input row stride is `nchan_in`, not `nchan_out * 4`. When `nchan_in` is not
    /// a multiple of 4, using the latter walks backwards into the previous slice's data.
    #[test]
    fn slices_are_not_mixed_when_nchan_is_not_a_multiple_of_four() {
        let data = ramp(3, 10);

        let max_hold = Pipeline::compute_mipmap(&data, 3, 10, MAX_HOLD);
        assert_close(&max_hold, &[3.0, 7.0, 103.0, 107.0, 203.0, 207.0]);

        let average = Pipeline::compute_mipmap(&data, 3, 10, AVERAGE);
        assert_close(&average, &[1.5, 5.5, 101.5, 105.5, 201.5, 205.5]);
    }

    /// The `nchan_in % 4` channels at the top of each slice have no full group and are dropped.
    #[test]
    fn trailing_partial_group_is_dropped() {
        let data = [0.0, 0.0, 0.0, 0.0, 99.0, 99.0];
        let mipmap = Pipeline::compute_mipmap(&data, 1, 6, MAX_HOLD);
        assert_close(&mipmap, &[0.0]);
    }

    /// Power values are in dB and therefore usually negative, so the max-hold identity must be
    /// below every sample rather than zero.
    #[test]
    fn max_hold_handles_all_negative_values() {
        let data = [-90.0, -80.0, -85.0, -95.0];
        let mipmap = Pipeline::compute_mipmap(&data, 1, 4, MAX_HOLD);
        assert_close(&mipmap, &[-80.0]);
    }

    #[test]
    fn fewer_than_four_channels_produces_no_mipmap() {
        for nchan in 0..4 {
            let data = ramp(2, nchan);
            let mipmap = Pipeline::compute_mipmap(&data, 2, nchan, MAX_HOLD);
            assert!(mipmap.is_empty(), "nchan={}", nchan);
        }
    }

    /// Each level is built from the previous one, so level 2 must equal a direct aggregation over
    /// 16 original channels. Uses `nchan = 20` so the second level also has a partial group.
    #[test]
    fn levels_compose_like_a_direct_aggregation() {
        for average in [MAX_HOLD, AVERAGE] {
            let data = ramp(3, 20);
            let level1 = Pipeline::compute_mipmap(&data, 3, 20, average);
            let level2 = Pipeline::compute_mipmap(&level1, 3, 20 / 4, average);

            // Slice x, bin 0 aggregates original channels 0..16; channels 16..20 are dropped.
            let expected = (0..3)
                .map(|x| {
                    let base = (x * 100) as f32;
                    if average == AVERAGE {
                        base + 7.5
                    } else {
                        base + 15.0
                    }
                })
                .collect_vec();
            assert_close(&level2, &expected);
        }
    }
}
