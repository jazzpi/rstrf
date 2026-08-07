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
//!
//! The pyramid is built by a compute shader in [mipmap.wgsl](mipmap.wgsl), one dispatch per level,
//! each reading the level below it. Switching between the two modes re-runs those dispatches, so
//! no pixel data crosses the bus. On adapters without compute support the same pyramid is built on
//! the CPU instead.
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

const MIPMAP_FACTOR: usize = 4;
/// Due to the mipmap, the size of the `spec_data` buffer must increase by
/// sum(1/4^n for n=0..inf) = 4/3
const MIPMAP_BUFFER_FACTOR: f64 = 4.0 / 3.0;

fn mipmap_buffer_size(normal_size: usize) -> usize {
    (normal_size as f64 * MIPMAP_BUFFER_FACTOR).ceil() as usize
}

/// Channels per slice at `level`, where level 0 is the raw data.
fn mipmap_level_nchan(nchan: usize, level: u32) -> usize {
    nchan / MIPMAP_FACTOR.pow(level)
}

/// Number of mipmap levels above level 0. The chain stops once a level would have no full group
/// of `MIPMAP_FACTOR` input channels left.
fn mipmap_level_count(nchan: usize) -> u32 {
    let mut n = nchan;
    let mut levels = 0;
    while n >= MIPMAP_FACTOR {
        n /= MIPMAP_FACTOR;
        levels += 1;
    }
    levels
}

/// Offset of `level` within one chunk's mipmap block, in channels. Multiply by the chunk's slice
/// count to get the offset in `f32` elements.
fn mipmap_level_offset_chan(nchan: usize, level: u32) -> usize {
    (0..level).map(|i| mipmap_level_nchan(nchan, i)).sum()
}

/// Total `f32` elements occupied by levels 1.. of one chunk.
fn mipmap_levels_len(nslices: usize, nchan: usize) -> usize {
    let past_last = mipmap_level_offset_chan(nchan, mipmap_level_count(nchan) + 1);
    nslices * (past_last - nchan)
}

/// Slices per GPU chunk, bounded by the largest storage buffer the device will bind. Returns 0 if
/// a single slice does not fit.
fn chunk_len(limits: &wgpu::Limits, spectrogram: &Spectrogram) -> usize {
    let max_buf_size =
        (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size) as usize;
    let slice_size = mipmap_buffer_size(spectrogram.nchan * std::mem::size_of::<f32>());
    spectrogram
        .nslices
        .min(max_buf_size / slice_size)
        .min(limits.max_compute_workgroups_per_dimension as usize)
}

/// Invocations per workgroup in `mipmap.wgsl`. A multiple of every common SIMD width, and well
/// under the 256 that `max_compute_invocations_per_workgroup` guarantees.
const WORKGROUP_SIZE: u32 = 64;

/// Stride between consecutive `MipParams` in the uniform buffer. Dynamic offsets must be a
/// multiple of `min_uniform_buffer_offset_alignment`; 256 is the value in `Limits::default()`, and
/// any device requiring less is satisfied by it too, since the requirement is always a power of
/// two no greater than 256.
const MIP_PARAMS_STRIDE: usize = 256;

/// One dispatch's worth of parameters. Must match `struct MipParams` in `mipmap.wgsl`, including
/// the padding: WGSL rounds uniform struct sizes up to a multiple of 16.
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct MipParams {
    src_offset: u32,
    dst_offset: u32,
    nchan_in: u32,
    nslices: u32,
    average: u32,
    _padding: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<MipParams>() == 32);
const _: () = assert!(std::mem::size_of::<MipParams>() <= MIP_PARAMS_STRIDE);

/// Parameters for every dispatch of one chunk, laid out at `MIP_PARAMS_STRIDE` intervals so a
/// dynamic offset can select a level.
fn mip_params_bytes(nslices: usize, nchan: usize, average: bool) -> Vec<u8> {
    let levels = mipmap_level_count(nchan);
    let mut bytes = vec![0u8; levels as usize * MIP_PARAMS_STRIDE];
    for level in 1..=levels {
        let params = MipParams {
            src_offset: (nslices * mipmap_level_offset_chan(nchan, level - 1)) as u32,
            dst_offset: (nslices * mipmap_level_offset_chan(nchan, level)) as u32,
            nchan_in: mipmap_level_nchan(nchan, level - 1) as u32,
            nslices: nslices as u32,
            average: average as u32,
            _padding: [0; 3],
        };
        let at = (level as usize - 1) * MIP_PARAMS_STRIDE;
        bytes[at..at + std::mem::size_of::<MipParams>()]
            .copy_from_slice(bytemuck::bytes_of(&params));
    }
    bytes
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

/// GPU-side mipmap state for one chunk. `None` when the CPU built the pyramid, or when the chunk
/// has too few channels for any level.
struct ChunkMipmap {
    /// `MipParams` per level at `MIP_PARAMS_STRIDE` intervals, selected by dynamic offset.
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Level-0 channel count, enough to re-derive every level's dispatch size.
    nchan: usize,
}

struct SpectrogramChunk {
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    /// Retained so the mode toggle can patch mipmap levels in place.
    spectrogram: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    mipmap: Option<ChunkMipmap>,
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
    average: bool,
    depth: DepthTarget,
}

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    /// `None` on adapters without compute support; the CPU path builds the pyramid instead.
    mipmap: Option<wgpu::ComputePipeline>,
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

        // TODO: this is a proxy for the real capability. Compute support is an *adapter* downlevel
        // flag (`DownlevelFlags::COMPUTE_SHADERS`) and iced never hands the adapter to
        // `shader::Pipeline::new`, so we infer it from the compute limits being non-zero — they are
        // literally 0 in wgpu's WebGL2 downlevel limit set. On native, iced requests
        // `Limits::default()` and falls back to `downlevel_defaults()`, both of which carry
        // non-zero compute limits, so a compute-less native adapter would fail at pipeline creation
        // rather than fall back here.
        let mipmap =
            (device.limits().max_compute_workgroup_size_x >= WORKGROUP_SIZE).then(|| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("spectrogram.mipmap.shader"),
                    source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                        "mipmap.wgsl"
                    ))),
                });

                // The layout cannot be auto-derived here: `has_dynamic_offset` has no WGSL
                // spelling, so `layout: None` would produce a binding with dynamic offsets
                // disabled.
                let bind_group_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("spectrogram.mipmap.bind_group_layout"),
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::COMPUTE,
                                ty: wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Uniform,
                                    has_dynamic_offset: true,
                                    min_binding_size: std::num::NonZeroU64::new(
                                        std::mem::size_of::<MipParams>() as u64,
                                    ),
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::COMPUTE,
                                ty: wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                                    has_dynamic_offset: false,
                                    min_binding_size: None,
                                },
                                count: None,
                            },
                        ],
                    });

                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("spectrogram.mipmap.pipeline_layout"),
                    bind_group_layouts: &[&bind_group_layout],
                    push_constant_ranges: &[],
                });

                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("spectrogram.mipmap.pipeline"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some("mipmap_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    cache: None,
                })
            });

        Self {
            pipeline,
            mipmap,
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
                self.mipmap.as_ref(),
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
        let mipmap_stride = MIPMAP_FACTOR.pow(mipmap_level);
        let nchan = mipmap_level_nchan(spectrogram.nchan, mipmap_level) as u32;
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

        let buf_offset_chan = mipmap_level_offset_chan(spectrogram.nchan, mipmap_level);

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
                self.mipmap.as_ref(),
                spectrogram,
                primitive.controls.average_plotting(),
            );
            primitive_data.spectrogram_id = spectrogram.id;
            primitive_data.average = primitive.controls.average_plotting();
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

        let average = primitive.controls.average_plotting();
        if primitive_data.average != average {
            match &self.mipmap {
                Some(compute) => Self::redispatch_mipmaps(
                    device,
                    queue,
                    compute,
                    &primitive_data.buffers.spectrogram,
                    average,
                ),
                None => Self::repatch_mipmaps_cpu(
                    device,
                    queue,
                    spectrogram,
                    &primitive_data.buffers.spectrogram,
                    average,
                ),
            }
            primitive_data.average = average;
        }
    }

    fn create_buffers(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        mipmap_pipeline: Option<&wgpu::ComputePipeline>,
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
        let spectrogram = Self::create_spectrogram_buffers(
            device,
            queue,
            pipeline,
            mipmap_pipeline,
            spectrogram,
            average,
        );

        PrimitiveData {
            buffers: Buffers {
                colormap: colormap_buffer,
                colormap_bind: colormap_bind_group,
                spectrogram,
            },
            spectrogram_id,
            colormap,
            average,
            depth: Self::create_depth_target(device, physical_size, &prefix),
        }
    }

    fn create_spectrogram_buffers(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        mipmap_pipeline: Option<&wgpu::ComputePipeline>,
        spectrogram: &Spectrogram,
        average: bool,
    ) -> Vec<SpectrogramChunk> {
        let limits = device.limits();
        let data = spectrogram.data.as_slice().unwrap();
        let chunk_len = chunk_len(&limits, spectrogram);
        if chunk_len == 0 {
            log::error!(
                "Spectrogram is too large to render ({} bytes per slice, max buffer size is {})",
                mipmap_buffer_size(spectrogram.nchan * std::mem::size_of::<f32>()),
                (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size),
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
                mipmap_pipeline.is_some(),
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

            let mipmap = mipmap_pipeline
                .filter(|_| mipmap_level_count(spectrogram.nchan) > 0)
                .map(|compute| {
                    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(format!("{prefix}.buffer.mip_params").as_str()),
                        contents: &mip_params_bytes(nslices as usize, spectrogram.nchan, average),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                    let layout = compute.get_bind_group_layout(0);
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(format!("{prefix}.bind_group.mipmap").as_str()),
                        layout: &layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &params,
                                    offset: 0,
                                    // One level's worth. The dynamic offset shifts this window, so
                                    // binding the whole buffer would put offset+size out of bounds.
                                    size: std::num::NonZeroU64::new(
                                        std::mem::size_of::<MipParams>() as u64,
                                    ),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: spectrogram_buffer.as_entire_binding(),
                            },
                        ],
                    });
                    ChunkMipmap {
                        params,
                        bind_group,
                        nchan: spectrogram.nchan,
                    }
                });

            let chunk = SpectrogramChunk {
                uniform: uniform_buffer,
                vertices: vertex_buffer,
                instances: instance_buffer,
                spectrogram: spectrogram_buffer,
                bind_group,
                mipmap,
                nslices: nslices as u32,
            };

            // `create_buffer_init` parks a host-visible staging copy of `chunk` in the queue's
            // pending writes until the next submit. Flush + wait here so that staging memory is
            // reclaimed before we build the next chunk, capping the transient overhead at one chunk
            // instead of holding a full second copy of the whole spectrogram in RAM until the next
            // frame happens to be rendered.
            //
            // The mipmap dispatch rides along in the same submit. `unmap()` inside
            // `create_spectrogram_buffer` queued level 0 into pending writes, which flush ahead of
            // command buffers, so level 0 is present before the first dispatch reads it.
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(format!("{prefix}.encoder.mipmap").as_str()),
            });
            if let Some(compute) = mipmap_pipeline {
                log::debug!("{prefix}: computing mipmap on the GPU (average={average})");
                Self::encode_mipmap_pass(compute, &mut encoder, &chunk);
            }
            queue.submit(Some(encoder.finish()));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());

            chunk
        })
        .collect()
    }

    fn create_spectrogram_buffer(
        device: &wgpu::Device,
        data: &[f32],
        nslices: usize,
        nchan: usize,
        label: &str,
        average: bool,
        gpu_mipmap: bool,
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
            if !gpu_mipmap {
                log::debug!("{label}: computing mipmap on the CPU (average={average})");
                let levels = Self::cpu_mipmap_levels(data, nslices, nchan, average);
                floats[data.len()..][..levels.len()].copy_from_slice(&levels);
            }
        }
        buffer.unmap();
        buffer
    }

    fn compute_mipmap_into(
        data: &[f32],
        nslices: usize,
        nchan_in: usize,
        average: bool,
        out: &mut [f32],
    ) {
        let stride = MIPMAP_FACTOR;
        let nchan_out = nchan_in / stride;
        for x in 0..nslices {
            for y in 0..nchan_out {
                let src = x * nchan_in + y * stride;
                out[x * nchan_out + y] = if average {
                    let mut sum = 0.0;
                    for i in 0..stride {
                        sum += data[src + i];
                    }
                    sum / stride as f32
                } else {
                    // Seed from the first sample rather than a sentinel, so no magic constant has
                    // to stay in sync with the compute shader.
                    let mut max = data[src];
                    for i in 1..stride {
                        max = max.max(data[src + i]);
                    }
                    max
                };
            }
        }
    }

    /// Test-only wrapper around `compute_mipmap_into` that allocates its own output buffer.
    #[cfg(test)]
    fn compute_mipmap(data: &[f32], nslices: usize, nchan_in: usize, average: bool) -> Vec<f32> {
        let mut out = vec![0.0; nslices * (nchan_in / MIPMAP_FACTOR)];
        Self::compute_mipmap_into(data, nslices, nchan_in, average, &mut out);
        out
    }

    /// Levels 1.. concatenated, laid out exactly as they sit in `spec_data` after level 0.
    fn cpu_mipmap_levels(data: &[f32], nslices: usize, nchan: usize, average: bool) -> Vec<f32> {
        let stride = MIPMAP_FACTOR;
        let mut out = vec![0.0; mipmap_levels_len(nslices, nchan)];

        // Level 1 reads the caller's data; every level after reads the one before it out of `out`.
        let mut nchan_in = nchan;
        let mut src = 0..0;
        let mut written = 0;
        while nchan_in >= stride {
            let len = nslices * (nchan_in / stride);
            let (done, rest) = out.split_at_mut(written);
            let src_data: &[f32] = if written == 0 {
                data
            } else {
                &done[src.clone()]
            };
            Self::compute_mipmap_into(src_data, nslices, nchan_in, average, &mut rest[..len]);
            src = written..written + len;
            written += len;
            nchan_in /= stride;
        }
        out
    }

    /// One dispatch per mipmap level, reading level k-1 and writing level k. wgpu inserts a memory
    /// barrier before each dispatch because `spec_data` is bound `read_write`, which is an
    /// exclusive usage and therefore never eligible for barrier elision.
    fn encode_mipmap_pass(
        pipeline: &wgpu::ComputePipeline,
        encoder: &mut wgpu::CommandEncoder,
        chunk: &SpectrogramChunk,
    ) {
        let Some(mipmap) = &chunk.mipmap else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("spectrogram.mipmap.pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        for level in 1..=mipmap_level_count(mipmap.nchan) {
            let nchan_out = mipmap_level_nchan(mipmap.nchan, level) as u32;
            let offset = (level as usize - 1) * MIP_PARAMS_STRIDE;
            pass.set_bind_group(0, &mipmap.bind_group, &[offset as u32]);
            pass.dispatch_workgroups(nchan_out.div_ceil(WORKGROUP_SIZE), chunk.nslices, 1);
        }
    }

    /// Rebuild every chunk's pyramid for a new plotting mode. `write_buffer` lands in the queue's
    /// pending writes, which flush ahead of the command buffers in the same submit, so every
    /// dispatch sees its new `average` flag.
    fn redispatch_mipmaps(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::ComputePipeline,
        chunks: &[SpectrogramChunk],
        average: bool,
    ) {
        log::debug!("recomputing mipmap on the GPU (average={average})");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("spectrogram.mipmap.encoder.redispatch"),
        });
        for chunk in chunks {
            let Some(mipmap) = &chunk.mipmap else {
                continue;
            };
            queue.write_buffer(
                &mipmap.params,
                0,
                &mip_params_bytes(chunk.nslices as usize, mipmap.nchan, average),
            );
            Self::encode_mipmap_pass(pipeline, &mut encoder, chunk);
        }
        queue.submit(Some(encoder.finish()));
    }

    /// Recompute levels 1.. for the current plotting mode and upload them over the existing ones.
    /// Level 0 is untouched, and the buffer objects are unchanged, so bind groups stay valid.
    fn repatch_mipmaps_cpu(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spectrogram: &Spectrogram,
        chunks: &[SpectrogramChunk],
        average: bool,
    ) {
        let chunk_len = chunk_len(&device.limits(), spectrogram);
        if chunk_len == 0 {
            return;
        }
        let data = spectrogram.data.as_slice().unwrap();
        log::debug!(
            "spectrogram.{}: recomputing mipmap on the CPU (average={average})",
            spectrogram.id
        );
        for (chunk_data, chunk) in izip!(data.chunks(chunk_len * spectrogram.nchan), chunks) {
            let levels = Self::cpu_mipmap_levels(
                chunk_data,
                chunk.nslices as usize,
                spectrogram.nchan,
                average,
            );
            queue.write_buffer(
                &chunk.spectrogram,
                std::mem::size_of_val(chunk_data) as u64,
                bytemuck::cast_slice(&levels),
            );
        }
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

    /// Level offsets and level sizes must describe the same layout: the offset of level L is the
    /// sum of the sizes of every level below it. `update_buffers` reads with the offsets, the
    /// pyramid builder writes with the sizes.
    #[test]
    fn level_offsets_are_the_running_sum_of_level_sizes() {
        for nchan in [4usize, 10, 63, 1024, 1250, 80000] {
            let mut expected = 0;
            for level in 0..=mipmap_level_count(nchan) {
                assert_eq!(
                    mipmap_level_offset_chan(nchan, level),
                    expected,
                    "nchan={}, level={}",
                    nchan,
                    level
                );
                expected += mipmap_level_nchan(nchan, level);
            }
        }
    }

    #[test]
    fn levels_len_covers_every_level_above_zero() {
        let nslices = 7;
        for nchan in [4usize, 10, 63, 1024, 1250, 80000] {
            let past_last = mipmap_level_offset_chan(nchan, mipmap_level_count(nchan) + 1);
            assert_eq!(
                mipmap_levels_len(nslices, nchan),
                nslices * (past_last - nchan),
                "nchan={}",
                nchan
            );
        }
    }

    /// The 4/3 budget in `mipmap_buffer_size` must hold for the levels we actually build.
    #[test]
    fn levels_fit_within_the_mipmap_buffer_budget() {
        let nslices = 7;
        for nchan in [4usize, 10, 63, 1024, 1250, 80000] {
            let data_len = nslices * nchan;
            let budget = mipmap_buffer_size(data_len * std::mem::size_of::<f32>())
                / std::mem::size_of::<f32>();
            assert!(
                data_len + mipmap_levels_len(nslices, nchan) <= budget,
                "nchan={}: {} + {} > {}",
                nchan,
                data_len,
                mipmap_levels_len(nslices, nchan),
                budget
            );
        }
    }

    #[test]
    fn fewer_than_four_channels_has_no_levels() {
        for nchan in 0..4 {
            assert_eq!(mipmap_level_count(nchan), 0, "nchan={}", nchan);
            assert_eq!(mipmap_levels_len(9, nchan), 0, "nchan={}", nchan);
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

    /// Each level must land at the offset `update_buffers` will read it from, and hold exactly what
    /// aggregating the level below it produces. `nchan = 20` so that level 2 has a partial group.
    #[test]
    fn cpu_mipmap_levels_places_each_level_at_its_offset() {
        for average in [MAX_HOLD, AVERAGE] {
            let (nslices, nchan) = (3usize, 20usize);
            let data = ramp(nslices, nchan);
            let levels = Pipeline::cpu_mipmap_levels(&data, nslices, nchan, average);

            let mut prev = data.clone();
            let mut nchan_in = nchan;
            for level in 1..=mipmap_level_count(nchan) {
                let expected = Pipeline::compute_mipmap(&prev, nslices, nchan_in, average);
                // Offsets are absolute within the chunk; `levels` starts after level 0.
                let start = nslices * mipmap_level_offset_chan(nchan, level) - data.len();
                assert_close(&levels[start..start + expected.len()], &expected);
                prev = expected;
                nchan_in /= 4;
            }
            assert_eq!(levels.len(), mipmap_levels_len(nslices, nchan));
        }
    }

    fn params_at(bytes: &[u8], level: u32) -> MipParams {
        let at = (level as usize - 1) * MIP_PARAMS_STRIDE;
        // `pod_read_unaligned`, not `from_bytes`: a `Vec<u8>` carries no alignment guarantee, and
        // `from_bytes` panics rather than reading unaligned.
        bytemuck::pod_read_unaligned(&bytes[at..at + std::mem::size_of::<MipParams>()])
    }

    /// Each level's dispatch must read the level below it and write where `update_buffers`
    /// expects to find it. These offsets are the same layout the CPU builder writes.
    #[test]
    fn mip_params_offsets_match_the_level_layout() {
        let (nslices, nchan) = (314usize, 80000usize);
        let bytes = mip_params_bytes(nslices, nchan, MAX_HOLD);

        assert_eq!(
            bytes.len(),
            mipmap_level_count(nchan) as usize * MIP_PARAMS_STRIDE
        );

        for level in 1..=mipmap_level_count(nchan) {
            let p = params_at(&bytes, level);
            assert_eq!(
                p.src_offset as usize,
                nslices * mipmap_level_offset_chan(nchan, level - 1),
                "level {} src",
                level
            );
            assert_eq!(
                p.dst_offset as usize,
                nslices * mipmap_level_offset_chan(nchan, level),
                "level {} dst",
                level
            );
            assert_eq!(
                p.nchan_in as usize,
                mipmap_level_nchan(nchan, level - 1),
                "level {} nchan_in",
                level
            );
            assert_eq!(p.nslices as usize, nslices, "level {} nslices", level);
        }
    }

    /// Level 1 reads level 0, which starts at the beginning of the chunk.
    #[test]
    fn mip_params_first_level_reads_from_zero() {
        let bytes = mip_params_bytes(7, 1024, MAX_HOLD);
        let p = params_at(&bytes, 1);
        assert_eq!(p.src_offset, 0);
        assert_eq!(p.dst_offset, 7 * 1024);
        assert_eq!(p.nchan_in, 1024);
    }

    #[test]
    fn mip_params_carries_the_plotting_mode() {
        for average in [MAX_HOLD, AVERAGE] {
            let bytes = mip_params_bytes(7, 1024, average);
            for level in 1..=mipmap_level_count(1024) {
                assert_eq!(params_at(&bytes, level).average, average as u32);
            }
        }
    }

    #[test]
    fn mip_params_is_empty_when_there_are_no_levels() {
        for nchan in 0..4 {
            assert!(
                mip_params_bytes(7, nchan, MAX_HOLD).is_empty(),
                "nchan={}",
                nchan
            );
        }
    }
}
