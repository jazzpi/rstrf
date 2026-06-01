//! This module contains the WGPU shader implementation for the RFPlot widget. The shader is
//! responsible for rendering the spectrogram itself.
use std::collections::HashMap;

use glam::{Vec2, vec2};
use iced::{
    Rectangle, mouse,
    wgpu::{self, util::DeviceExt},
    widget::shader,
};
use itertools::{Itertools, izip};
use rstrf::{colormap::Colormap, spectrogram::Spectrogram};
use uuid::Uuid;

use super::{Controls, Message, MouseInteraction, RFPlot};

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Uniforms {
    power_bounds: Vec2,
    time_bounds: Vec2,
    freq_bounds: Vec2,
    pixel_height: f32,
    nslices: u32,
    nchan: u32,
    _pad: u32,
}

// #[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
// #[repr(C)]
// struct Vertex {
//     corner: Vec2,
// }

// #[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
// #[repr(C)]
// struct Instance {
//     time_idx: u32,
// }

struct SpectrogramChunk {
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    #[allow(dead_code)] // Keep this around in case we need it for future features
    spectrogram: wgpu::Buffer,
    x_ranges: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    nslices: u32,
    slice_offset: u32,
    visible: bool,
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
            depth_stencil: None,
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
        viewport: &shader::Viewport,
    ) {
        let Some(spectrogram) = &primitive.spectrogram else {
            return;
        };

        let primitive_data = self.instances.entry(primitive.id).or_insert_with_key(|id| {
            Self::create_buffers(
                device,
                &self.pipeline,
                id,
                spectrogram,
                primitive.controls.colormap(),
            )
        });

        let bounds = primitive.controls.bounds();
        let pixel_height =
            bounds.0.height / viewport.physical_height() as f32 * spectrogram.nchan as f32;

        let xmin = bounds.0.x;
        let xmax = bounds.0.x + bounds.0.width;
        let vmin = bounds.0.y;
        let vmax = bounds.0.y + bounds.0.height;

        for (i, chunk) in primitive_data.buffers.spectrogram.iter_mut().enumerate() {
            let uniforms = Uniforms {
                power_bounds: primitive.controls.power_range().into(),
                time_bounds: vec2(xmin, xmax),
                freq_bounds: vec2(vmin, vmax),
                nslices: chunk.nslices,
                nchan: spectrogram.nchan as u32,
                pixel_height,
                _pad: 0,
            };
            queue.write_buffer(&chunk.uniform, 0, bytemuck::bytes_of(&uniforms));

            // let left = spectrogram.timestamps[i * chunk.nslices as usize];
            // let width = chunk.nslices as f32 / spectrogram.nslices as f32;
            // let xmin = (left - bounds.0.x) / bounds.0.width;
            // let xmax = ((left + width) - bounds.0.x) / bounds.0.width;
            // chunk.visible = xmax > 0.0 && xmin < 1.0;
            // left += width;
        }

        if primitive_data.spectrogram_id != spectrogram.id {
            primitive_data.buffers.spectrogram =
                Self::create_spectrogram_buffers(device, &self.pipeline, spectrogram);
            primitive_data.spectrogram_id = spectrogram.id;
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
        pipeline: &wgpu::RenderPipeline,
        id: &Uuid,
        spectrogram: &Spectrogram,
        colormap: Colormap,
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

        let spectrogram = Self::create_spectrogram_buffers(device, pipeline, spectrogram);

        PrimitiveData {
            buffers: Buffers {
                colormap: colormap_buffer,
                colormap_bind: colormap_bind_group,
                spectrogram,
            },
            spectrogram_id: *id,
            colormap,
        }
    }

    fn create_spectrogram_buffers(
        device: &wgpu::Device,
        pipeline: &wgpu::RenderPipeline,
        spectrogram: &Spectrogram,
    ) -> Vec<SpectrogramChunk> {
        let limits = device.limits();
        let max_buf_size =
            (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size) as usize;
        let data = spectrogram.data.as_slice().unwrap();
        let chunk_len = (max_buf_size / (std::mem::size_of::<f32>() * spectrogram.nchan))
            .min(data.len() / spectrogram.nchan);
        if chunk_len == 0 {
            log::error!(
                "Spectrogram is too large to render ({} bytes per slice, max buffer size is {})",
                spectrogram.nchan * std::mem::size_of::<f32>(),
                max_buf_size
            );
            return Vec::new();
        }

        let prefix = format!("spectrogram.{}", spectrogram.id);
        let timestamps = spectrogram
            .timestamps
            .iter()
            .map(|t| (*t - spectrogram.start_time).as_seconds_f32());
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

            let spectrogram_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(format!("{prefix}.buffer.spectrogram").as_str()),
                contents: bytemuck::cast_slice(chunk),
                usage: wgpu::BufferUsages::STORAGE,
            });
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
            SpectrogramChunk {
                uniform: uniform_buffer,
                vertices: vertex_buffer,
                instances: instance_buffer,
                spectrogram: spectrogram_buffer,
                x_ranges: x_ranges_buffer,
                bind_group,
                nslices: nslices as u32,
                slice_offset: (i * chunk_len) as u32,
                visible: true,
            }
        })
        .collect()
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
            depth_stencil_attachment: None,
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
            if !chunk.visible {
                continue;
            }
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
}

impl Primitive {
    fn new(id: uuid::Uuid, controls: Controls, spectrogram: Option<Spectrogram>) -> Self {
        Self {
            id,
            controls,
            spectrogram,
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
        _bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        pipeline.update_buffers(device, queue, self, viewport);
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
    type State = MouseInteraction;
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
        )
    }
}
