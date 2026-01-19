use std::ops::{Add, Sub};

use cosmic::{
    Element,
    iced::{Length, Padding, Rectangle, event::Status, mouse, widget as iw},
    widget::{container, slider, text},
};
use duplicate::duplicate_item;
use glam::Vec2;
use plotters_iced::ChartWidget;
use rstrf::{orbit::Satellite, spectrogram::Spectrogram};

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 17.0;
const POWER_MIN: f32 = -100.0;
const POWER_MAX: f32 = 60.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

mod colormap;
mod plotter;
mod shader;

#[derive(Debug, Clone, Copy)]
struct Controls {
    zoom: Vec2,
    center: Vec2,
    power_bounds: (f32, f32),
}

impl Controls {
    fn scale(&self) -> Vec2 {
        Vec2::new(
            1.0 / 2.0_f32.powf(self.zoom.x),
            1.0 / 2.0_f32.powf(self.zoom.y),
        )
    }

    fn bounds(&self) -> (Vec2, Vec2) {
        let half_scale = self.scale() / 2.0;
        (
            Vec2::new(self.center.x - half_scale.x, self.center.x + half_scale.x),
            Vec2::new(self.center.y - half_scale.y, self.center.y + half_scale.y),
        )
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            zoom: Vec2::new(ZOOM_MIN, ZOOM_MIN),
            center: Vec2::new(0.5, 0.5),
            power_bounds: (POWER_MIN, POWER_MAX),
        }
    }
}

#[duplicate_item(name; [ScreenCoords]; [PlotRelativeCoords]; [PlotAbsoluteCoords])]
#[derive(Debug, Clone, Copy)]
pub struct name(Vec2);
#[duplicate_item(name; [ScreenCoords]; [PlotRelativeCoords]; [PlotAbsoluteCoords])]
impl Sub for name {
    type Output = name;

    fn sub(self, rhs: Self) -> Self::Output {
        name(self.0 - rhs.0)
    }
}
#[duplicate_item(name; [ScreenCoords]; [PlotRelativeCoords]; [PlotAbsoluteCoords])]
impl Add for name {
    type Output = name;

    fn add(self, rhs: Self) -> Self::Output {
        name(self.0 + rhs.0)
    }
}

impl ScreenCoords {
    fn plot_relative(&self, bounds: &Rectangle) -> PlotRelativeCoords {
        let x = self.0.x / bounds.width;
        let y = 1.0 - self.0.y / bounds.height;
        PlotRelativeCoords(Vec2::new(x, y))
    }

    fn plot_absolute(&self, bounds: &Rectangle, controls: &Controls) -> PlotAbsoluteCoords {
        let norm_x = self.0.x / bounds.width;
        let norm_y = 1.0 - self.0.y / bounds.height;
        let center = controls.center;
        let scale = controls.scale();
        let x = center.x + (norm_x - 0.5) * scale.x;
        let y = center.y + (norm_y - 0.5) * scale.y;
        PlotAbsoluteCoords(Vec2::new(x, y))
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(PlotRelativeCoords),
    ZoomDelta(PlotAbsoluteCoords, f32),
    ZoomDeltaX(f32),
    ZoomDeltaY(f32),
    UpdateMinPower(f32),
    UpdateMaxPower(f32),
    SetSatellites(Vec<Satellite>),
}

pub enum MouseInteraction {
    Idle,
    Panning(PlotRelativeCoords),
}

impl Default for MouseInteraction {
    fn default() -> Self {
        MouseInteraction::Idle
    }
}

pub struct RFPlot {
    controls: Controls,
    spectrogram: Spectrogram,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    plot_area_margin: f32,
    satellites: Vec<Satellite>,
}

impl RFPlot {
    pub fn new(spectrogram: Spectrogram) -> Self {
        Self {
            controls: Controls {
                power_bounds: spectrogram.power_bounds,
                ..Controls::default()
            },
            spectrogram,
            plot_area_margin: 50.0,
            satellites: Vec::new(),
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
                self.controls.center -= delta.0 * self.controls.scale();
            }
            Message::ZoomDelta(pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let prev_scale = self.controls.scale();
                let prev_zoom = self.controls.zoom;
                self.controls.zoom = (prev_zoom + Vec2::splat(delta))
                    .max(Vec2::splat(ZOOM_MIN))
                    .min(Vec2::splat(ZOOM_MAX));

                let vec = pos.0 - self.controls.center;
                let new_scale = self.controls.scale();
                self.controls.center += vec * (prev_scale - new_scale) * 2.0;
            }
            Message::ZoomDeltaX(delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                self.controls.zoom.x = (self.controls.zoom.x + delta).max(ZOOM_MIN).min(ZOOM_MAX);
            }
            Message::ZoomDeltaY(delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                self.controls.zoom.y = (self.controls.zoom.y + delta).max(ZOOM_MIN).min(ZOOM_MAX);
            }
            Message::UpdateMinPower(min_power) => {
                self.controls.power_bounds.0 = min_power.min(self.controls.power_bounds.1);
            }
            Message::UpdateMaxPower(max_power) => {
                self.controls.power_bounds.1 = max_power.max(self.controls.power_bounds.0);
            }
            Message::SetSatellites(satellites) => {
                self.satellites = satellites;
                log::debug!("Using {} satellites", self.satellites.len());
            }
        }
    }

    fn control<'a>(
        label: &'static str,
        control: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        iw::row![text(label), control.into()].spacing(10).into()
    }

    /// Build the RFPlot widget view.
    ///
    /// The plot itself is implemented as a stack of two layers: the spectrogram itself (see
    /// `shader.rs`) and the overlay (see `plotter.rs`).
    pub fn view(&self) -> Element<'_, Message> {
        let controls = iw::row![
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
            Self::control(
                "Min Power",
                slider(
                    POWER_MIN..=POWER_MAX,
                    self.controls.power_bounds.0,
                    move |power| { Message::UpdateMinPower(power) }
                )
                .step(0.1)
                .width(Length::Fill)
            ),
            Self::control(
                "Max Power",
                slider(
                    POWER_MIN..=POWER_MAX,
                    self.controls.power_bounds.1,
                    move |power| { Message::UpdateMaxPower(power) }
                )
                .step(0.1)
                .width(Length::Fill)
            ),
        ];

        let spectrogram: Element<'_, Message> =
            container(iw::shader(self).width(Length::Fill).height(Length::Fill))
                .padding(Padding {
                    top: 0.0,
                    right: 0.0,
                    bottom: self.plot_area_margin,
                    left: self.plot_area_margin,
                })
                .into();
        let plot_overlay: Element<'_, Message> = ChartWidget::new(self)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let plot_area: Element<'_, Message> = iw::stack![spectrogram, plot_overlay,].into();

        iw::column![plot_area, controls]
            .padding(10)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn handle_mouse(
        &self,
        state: &mut MouseInteraction,
        event: mouse::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<Message>) {
        let pos = cursor
            .position_in(bounds)
            .and_then(|p| Some(ScreenCoords(Vec2::new(p.x, p.y))));
        if let mouse::Event::WheelScrolled { delta } = event {
            let delta = match delta {
                mouse::ScrollDelta::Lines { x: _, y } => y,
                mouse::ScrollDelta::Pixels { x: _, y } => y,
            };
            if let Some(pos) = pos {
                return (
                    Status::Captured,
                    Some(Message::ZoomDelta(
                        pos.plot_absolute(&bounds, &self.controls),
                        delta,
                    )),
                );
            } else if cursor.is_over(Rectangle {
                x: bounds.x - self.plot_area_margin,
                y: bounds.y,
                width: self.plot_area_margin,
                height: bounds.height,
            }) {
                // Zooming over y axis
                return (Status::Captured, Some(Message::ZoomDeltaY(delta)));
            } else if cursor.is_over(Rectangle {
                x: bounds.x,
                y: bounds.y + bounds.height,
                width: bounds.width,
                height: self.plot_area_margin,
            }) {
                // Zooming over x axis
                return (Status::Captured, Some(Message::ZoomDeltaX(delta)));
            }
        }

        match state {
            MouseInteraction::Idle => match event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if let Some(pos) = pos {
                        *state = MouseInteraction::Panning(pos.plot_relative(&bounds));
                        return (Status::Captured, None);
                    }
                }
                _ => {}
            },
            MouseInteraction::Panning(prev_pos) => match event {
                mouse::Event::ButtonReleased(mouse::Button::Left) => {
                    *state = MouseInteraction::Idle;
                }
                mouse::Event::CursorMoved { position } => {
                    // pos might be None if the cursor is outside bounds
                    let pos = ScreenCoords(Vec2::new(position.x - bounds.x, position.y - bounds.y))
                        .plot_relative(&bounds);
                    let delta = pos - *prev_pos;
                    *state = MouseInteraction::Panning(pos);
                    return (Status::Captured, Some(Message::PanningDelta(delta)));
                }
                _ => {}
            },
        };

        (Status::Captured, None)
    }
}
