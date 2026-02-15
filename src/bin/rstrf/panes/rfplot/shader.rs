//! This module contains the WGPU shader implementation for the RFPlot widget. The shader is
//! responsible for rendering the spectrogram itself.
use std::collections::HashMap;

use glam::Vec2;
use iced::{
    Rectangle, mouse,
    wgpu::{self, util::DeviceExt},
    widget::shader,
};
use rstrf::{colormap::Colormap, spectrogram::Spectrogram};
use uuid::Uuid;

use super::{Controls, Message, MouseInteraction, RFPlot};

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Uniforms {
    x_bounds: Vec2,
    y_bounds: Vec2,
    power_bounds: Vec2,
    nslices: u32,
    nchan: u32,
}

#[allow(dead_code)] // Keep buffers accessible for later features
struct Buffers {
    uniform: wgpu::Buffer,
    colormap: wgpu::Buffer,
    spectrogram: wgpu::Buffer,
}

struct BindGroups {
    uniform: wgpu::BindGroup,
    spectrogram: wgpu::BindGroup,
}

struct PrimitiveData {
    buffers: Buffers,
    bind_groups: BindGroups,
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
                buffers: &[],
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
        let uniforms = Uniforms {
            x_bounds: (bounds.0.x, bounds.0.x + bounds.0.width).into(),
            y_bounds: (bounds.0.y, bounds.0.y + bounds.0.height).into(),
            power_bounds: primitive.controls.power_range().into(),
            nslices: spectrogram.nslices as u32,
            nchan: spectrogram.nchan as u32,
        };

        queue.write_buffer(
            &primitive_data.buffers.uniform,
            0,
            bytemuck::bytes_of(&uniforms),
        );
        if primitive_data.spectrogram_id != spectrogram.id {
            (
                primitive_data.buffers.spectrogram,
                primitive_data.bind_groups.spectrogram,
            ) = Self::create_spectrogram_buffers(device, &self.pipeline, spectrogram);
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
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(format!("{prefix}.buffer.uniform").as_str()),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let colormap_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(format!("{prefix}.buffer.colormap").as_str()),
            contents: bytemuck::cast_slice(colormap.buffer()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout = pipeline.get_bind_group_layout(0);
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(format!("{prefix}.bind_group.uniform").as_str()),
            layout: &uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: colormap_buffer.as_entire_binding(),
                },
            ],
        });

        let (spectrogram_buffer, spectrogram_bind_group) =
            Self::create_spectrogram_buffers(device, pipeline, spectrogram);

        PrimitiveData {
            buffers: Buffers {
                uniform: uniform_buffer,
                colormap: colormap_buffer,
                spectrogram: spectrogram_buffer,
            },
            bind_groups: BindGroups {
                uniform: uniform_bind_group,
                spectrogram: spectrogram_bind_group,
            },
            spectrogram_id: *id,
            colormap,
        }
    }

    fn create_spectrogram_buffers(
        device: &wgpu::Device,
        pipeline: &wgpu::RenderPipeline,
        spectrogram: &Spectrogram,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(format!("spectrogram.{}.buffer.spectrogram", spectrogram.id).as_str()),
            contents: bytemuck::cast_slice(spectrogram.data.as_slice().unwrap()),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let bind_group_layout = pipeline.get_bind_group_layout(1);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(format!("spectrogram.{}.bind_group.spectrogram", spectrogram.id).as_str()),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        (buffer, bind_group)
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
        pass.set_bind_group(0, &primitive_data.bind_groups.uniform, &[]);
        pass.set_bind_group(1, &primitive_data.bind_groups.spectrogram, &[]);
        pass.draw(0..6, 0..1);
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
        _viewport: &shader::Viewport,
    ) {
        pipeline.update_buffers(device, queue, self);
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
            self.shared.spectrogram.clone(), // TODO
        )
    }
}
