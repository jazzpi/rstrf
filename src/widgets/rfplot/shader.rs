//! This module contains the WGPU shader implementation for the RFPlot widget. The shader is
//! responsible for rendering the spectrogram itself.
use cosmic::iced::{
    Rectangle, mouse,
    wgpu::{self, util::DeviceExt},
    widget::shader,
};
use glam::Vec2;
use rstrf::spectrogram::Spectrogram;

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

struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
}

impl Pipeline {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, spec: Spectrogram) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FragmentShaderPipeline shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader.wgsl"
            ))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FragmentShaderPipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
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

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader_quad uniform buffer"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let spec_data = spec.data();
        let spec_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("spectrogram buffer"),
            contents: bytemuck::cast_slice(spec_data.as_slice().unwrap()),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let colormap_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("colormap buffer"),
            contents: bytemuck::cast_slice(&super::colormap::MAGMA),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let uniform_bind_group_layout = pipeline.get_bind_group_layout(0);
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader_quad uniform bind group"),
            layout: &uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: spec_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: colormap_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            pipeline,
            uniform_buffer,
            uniform_bind_group,
        }
    }

    fn update(&mut self, queue: &wgpu::Queue, uniforms: &Uniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }

    fn render(
        &self,
        target: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        viewport: Rectangle<u32>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fill color test"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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

        pass.set_pipeline(&self.pipeline);
        pass.set_viewport(
            viewport.x as f32,
            viewport.y as f32,
            viewport.width as f32,
            viewport.height as f32,
            0.0,
            1.0,
        );
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);

        pass.draw(0..6, 0..1);
    }
}

#[derive(Debug)]
pub struct Primitive {
    controls: Controls,
    spectrogram: Spectrogram,
}

impl Primitive {
    fn new(controls: Controls, spectrogram: Spectrogram) -> Self {
        Self {
            controls,
            spectrogram,
        }
    }
}

impl shader::Primitive for Primitive {
    fn prepare(
        &self,
        device: &cosmic::iced::wgpu::Device,
        queue: &cosmic::iced::wgpu::Queue,
        format: cosmic::iced::wgpu::TextureFormat,
        storage: &mut shader::Storage,
        _bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        if !storage.has::<Pipeline>() {
            storage.store(Pipeline::new(
                device,
                format,
                self.spectrogram.clone(), // TODO
            ));
        }

        let pipeline = storage.get_mut::<Pipeline>().unwrap();

        let spec_data = self.spectrogram.data();
        let (nslices, nchan) = spec_data.dim();

        let (x_bounds, y_bounds) = self.controls.bounds();

        pipeline.update(
            queue,
            &Uniforms {
                x_bounds,
                y_bounds,
                power_bounds: self.controls.power_bounds.into(),
                nslices: nslices as u32,
                nchan: nchan as u32,
            },
        );
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let pipeline = storage.get::<Pipeline>().unwrap();
        pipeline.render(target, encoder, *clip_bounds);
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
            self.controls,
            self.spectrogram.clone(), // TODO
        )
    }
}
