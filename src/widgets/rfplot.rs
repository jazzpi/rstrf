use cosmic::Element;
use cosmic::iced::advanced::Shell;
use cosmic::iced::event::Status;
use cosmic::iced::mouse;
use cosmic::iced::mouse::Cursor;
use cosmic::iced::wgpu;
use cosmic::iced::wgpu::util::DeviceExt;
use cosmic::iced::widget::shader::Event;
use cosmic::iced::widget::{column, row, shader, slider, text};
use cosmic::iced::{Length, Rectangle};
use glam::Vec2;
use rs_trf::spectrogram::Spectrogram;

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 17.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Uniforms {
    x_bounds: Vec2,
    y_bounds: Vec2,
    nslices: u32,
    nchan: u32,
}

struct FragmentShaderPipeline {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
}

impl FragmentShaderPipeline {
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

#[derive(Debug, Clone, Copy)]
struct Controls {
    zoom: Vec2,
    center: Vec2,
}

impl Controls {
    fn scale(&self) -> Vec2 {
        Vec2::new(
            1.0 / 2.0_f32.powf(self.zoom.x), // / ZOOM_PIXELS_FACTOR,
            1.0 / 2.0_f32.powf(self.zoom.y), // / ZOOM_PIXELS_FACTOR,
        )
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            zoom: Vec2::new(ZOOM_MIN, ZOOM_MIN),
            center: Vec2::new(0.5, 0.5),
        }
    }
}

#[derive(Debug)]
pub struct FragmentShaderPrimitive {
    controls: Controls,
    spectrogram: Spectrogram,
}

impl FragmentShaderPrimitive {
    fn new(controls: Controls, spectrogram: Spectrogram) -> Self {
        Self {
            controls,
            spectrogram,
        }
    }
}

impl shader::Primitive for FragmentShaderPrimitive {
    fn prepare(
        &self,
        device: &cosmic::iced::wgpu::Device,
        queue: &cosmic::iced::wgpu::Queue,
        format: cosmic::iced::wgpu::TextureFormat,
        storage: &mut shader::Storage,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        if !storage.has::<FragmentShaderPipeline>() {
            storage.store(FragmentShaderPipeline::new(
                device,
                format,
                self.spectrogram.clone(), // TODO
            ));
        }

        let pipeline = storage.get_mut::<FragmentShaderPipeline>().unwrap();

        let spec_data = self.spectrogram.data();
        let (nslices, nchan) = spec_data.dim();

        let center = self.controls.center;
        let scale = self.controls.scale();
        let x_bounds = Vec2::new(center.x - scale.x / 2f32, center.x + scale.x / 2f32);
        let y_bounds = Vec2::new(center.y - scale.y / 2f32, center.y + scale.y / 2f32);

        pipeline.update(
            queue,
            &Uniforms {
                x_bounds,
                y_bounds,
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
        let pipeline = storage.get::<FragmentShaderPipeline>().unwrap();
        pipeline.render(target, encoder, *clip_bounds);
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(Vec2),
    ZoomDelta(Vec2, f32),
}

pub enum MouseInteraction {
    Idle,
    Panning(Vec2),
}

impl Default for MouseInteraction {
    fn default() -> Self {
        MouseInteraction::Idle
    }
}

pub struct RFPlot {
    controls: Controls,
    spectrogram: Spectrogram,
}

impl RFPlot {
    pub fn new(spectrogram: Spectrogram) -> Self {
        Self {
            controls: Controls::default(),
            spectrogram,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::UpdateZoomX(zoom_x) => {
                self.controls.zoom.x = zoom_x;
            }
            Message::UpdateZoomY(zoom_y) => {
                self.controls.zoom.y = zoom_y;
            }
            Message::PanningDelta(delta) => {
                let scale = self.controls.scale();
                self.controls.center.x -= delta.x * scale.x;
                self.controls.center.y -= delta.y * scale.y;
            }
            Message::ZoomDelta(pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let prev_scale = self.controls.scale();
                let prev_zoom = self.controls.zoom;
                self.controls.zoom = (prev_zoom + Vec2::splat(delta))
                    .max(Vec2::splat(ZOOM_MIN))
                    .min(Vec2::splat(ZOOM_MAX));

                let vec = pos - self.controls.center;
                let new_scale = self.controls.scale();
                self.controls.center += vec * (prev_scale - new_scale) * 2.0;
            }
        }
    }

    fn control<'a>(
        label: &'static str,
        control: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        row![text(label), control.into()].spacing(10).into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let controls = row![
            Self::control(
                "Zoom Time",
                slider(ZOOM_MIN..=ZOOM_MAX, self.controls.zoom.x, move |zoom| {
                    Message::UpdateZoomX(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
            Self::control(
                "Zoom Freq",
                slider(ZOOM_MIN..=ZOOM_MAX, self.controls.zoom.y, move |zoom| {
                    Message::UpdateZoomY(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
        ];

        let spectrogram = shader(self).width(Length::Fill).height(Length::Fill);

        column![spectrogram, controls]
            // .align_items(Alignment::Center)
            .padding(10)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Normalize screen coordinates to [0, 1], assuming (0,0) is top-left. This seems to be the
    /// case for scrolling events, regardless of the bounds.x/y.
    fn normalize_scroll_position(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        Vec2::new(pos.x / bounds.width, 1.0 - pos.y / bounds.height)
    }

    /// Normalize screen coordinates to [0, 1], assuming (bounds.x, bounds.y) is top-left. This
    /// seems to be the case for mouse click & move events.
    fn normalize_click_position(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        Vec2::new(
            (pos.x - bounds.x) / bounds.width,
            1.0 - (pos.y - bounds.y) / bounds.height,
        )
    }

    fn screen_scroll_to_uv(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        let norm = self.normalize_scroll_position(pos, bounds);
        let center = self.controls.center;
        let scale = self.controls.scale();
        Vec2::new(
            center.x + (norm.x - 0.5) * scale.x,
            center.y + (norm.y - 0.5) * scale.y,
        )
    }

    fn screen_click_to_uv(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        let norm = self.normalize_click_position(pos, bounds);
        let center = self.controls.center;
        let scale = self.controls.scale();
        Vec2::new(
            center.x + (norm.x - 0.5) * scale.x,
            center.y + (norm.y - 0.5) * scale.y,
        )
    }
}

impl shader::Program<Message> for RFPlot {
    type State = MouseInteraction;
    type Primitive = FragmentShaderPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        FragmentShaderPrimitive::new(
            self.controls,
            self.spectrogram.clone(), // TODO
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: Event,
        bounds: Rectangle,
        cursor: Cursor,
        _shell: &mut Shell<'_, Message>,
    ) -> (Status, Option<Message>) {
        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event {
            if let Some(pos) = cursor.position_in(bounds) {
                let pos = self.screen_scroll_to_uv(Vec2::new(pos.x, pos.y), &bounds);
                let delta = match delta {
                    mouse::ScrollDelta::Lines { x: _, y } => y,
                    mouse::ScrollDelta::Pixels { x: _, y } => y,
                };
                return (Status::Captured, Some(Message::ZoomDelta(pos, delta)));
            }
        }

        match state {
            MouseInteraction::Idle => match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    if let Some(pos) = cursor.position_over(bounds) {
                        *state = MouseInteraction::Panning(
                            self.normalize_click_position(Vec2::new(pos.x, pos.y), &bounds),
                        );
                        log::debug!(
                            "Clicked at pos={:?}, norm={:?}, uv={:?}",
                            pos,
                            self.normalize_click_position(Vec2::new(pos.x, pos.y), &bounds),
                            self.screen_click_to_uv(Vec2::new(pos.x, pos.y), &bounds)
                        );
                        return (Status::Captured, None);
                    }
                }
                _ => {}
            },
            MouseInteraction::Panning(prev_pos) => match event {
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    *state = MouseInteraction::Idle;
                }
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    let pos =
                        self.normalize_click_position(Vec2::new(position.x, position.y), &bounds);
                    let delta = pos - *prev_pos;
                    *state = MouseInteraction::Panning(pos);
                    return (Status::Captured, Some(Message::PanningDelta(delta)));
                }
                _ => {}
            },
        };

        (Status::Ignored, None)
    }
}

// struct FragmentShaderApp {
//     program: RFPlot,
// }

// fn control<'a>(
//     label: &'static str,
//     control: impl Into<Element<'a, Message>>,
// ) -> Element<'a, Message> {
//     row![text(label), control.into()].spacing(10).into()
// }

// impl Sandbox for FragmentShaderApp {
//     type Message = Message;

//     fn new() -> Self {
//         Self {
//             program: RFPlot::new(),
//         }
//     }

//     fn title(&self) -> String {
//         String::from("Fragment Shader Widget - Iced")
//     }

//     fn view(&self) -> Element<'_, Message> {
//         let controls = row![
//             control(
//                 "Max iterations",
//                 slider(
//                     ITERS_MIN..=ITERS_MAX,
//                     self.program.controls.max_iter,
//                     move |iter| { Message::UpdateMaxIterations(iter) }
//                 )
//                 .width(Length::Fill)
//             ),
//             control(
//                 "Zoom",
//                 slider(
//                     ZOOM_MIN..=ZOOM_MAX,
//                     self.program.controls.zoom,
//                     move |zoom| { Message::UpdateZoom(zoom) }
//                 )
//                 .step(0.01)
//                 .width(Length::Fill)
//             ),
//         ];

//         let shader = shader(&self.program)
//             .width(Length::Fill)
//             .height(Length::Fill);

//         column![shader, controls]
//             .align_items(Alignment::Center)
//             .padding(10)
//             .spacing(10)
//             .width(Length::Fill)
//             .height(Length::Fill)
//             .into()
//     }

//     fn update(&mut self, message: Message) {
//         match message {
//             Message::UpdateMaxIterations(max_iter) => {
//                 self.program.controls.max_iter = max_iter;
//             }
//             Message::UpdateZoom(zoom) => {
//                 self.program.controls.zoom = zoom;
//             }
//             Message::PanningDelta(delta) => {
//                 self.program.controls.center -= 2.0 * delta * self.program.controls.scale();
//             }
//             Message::ZoomDelta(pos, bounds, delta) => {
//                 let delta = delta * ZOOM_WHEEL_SCALE;
//                 let prev_scale = self.program.controls.scale();
//                 let prev_zoom = self.program.controls.zoom;
//                 self.program.controls.zoom = (prev_zoom + delta).max(ZOOM_MIN).min(ZOOM_MAX);

//                 let vec = pos - Vec2::new(bounds.width, bounds.height) * 0.5;
//                 let new_scale = self.program.controls.scale();
//                 self.program.controls.center += vec * (prev_scale - new_scale) * 2.0;
//             }
//         }
//     }
// }

// fn main() -> iced::Result {
//     FragmentShaderApp::run(Settings::default())
// }
