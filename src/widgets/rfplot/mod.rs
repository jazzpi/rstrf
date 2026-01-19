use std::collections::HashMap;

use chrono::{DateTime, Utc};
use cosmic::{
    Element, Task,
    iced::{Length, Padding, Rectangle, event::Status, mouse, widget as iw},
    widget::{container, slider, text},
};
use glam::Vec2;
use ndarray::Array1;
use plotters_iced::ChartWidget;
use rstrf::{orbit::Satellite, spectrogram::Spectrogram, util::minmax};

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 17.0;
const POWER_MIN: f32 = -100.0;
const POWER_MAX: f32 = 60.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

mod colormap;
mod coord;
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

#[derive(Debug, Clone)]
pub enum Message {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(coord::PlotArea),
    ZoomDelta(coord::PlotArea, f32),
    ZoomDeltaX(f32),
    ZoomDeltaY(f32),
    UpdateMinPower(f32),
    UpdateMaxPower(f32),
    SetSatellites(Vec<Satellite>),
    SetSatellitePredictions(Option<Predictions>),
}

pub enum MouseInteraction {
    Idle,
    Panning(coord::PlotArea),
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
    satellite_predictions: Option<Predictions>,
}

#[derive(Clone)]
pub struct Predictions {
    times: Array1<f64>,
    frequencies: HashMap<u64, Array1<f64>>,
    zenith_angles: HashMap<u64, Array1<f64>>,
}

impl std::fmt::Debug for Predictions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Predictions")
            .field("times", &minmax(&self.times))
            .field("frequencies", &self.frequencies.len())
            .field("zenith_angles", &self.zenith_angles.len())
            .finish()
    }
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
            satellite_predictions: None,
        }
    }

    #[must_use]
    pub fn update(&mut self, message: Message) -> Task<Message> {
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
            Message::ZoomDelta(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;

                let old_data = plot_pos.data_normalized(&self.controls);
                let prev_zoom = self.controls.zoom;
                self.controls.zoom = (prev_zoom + Vec2::splat(delta))
                    .clamp(Vec2::splat(ZOOM_MIN), Vec2::splat(ZOOM_MAX));
                let new_data = plot_pos.data_normalized(&self.controls);
                self.controls.center += old_data.0 - new_data.0;
            }
            Message::ZoomDeltaX(delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                self.controls.zoom.x = (self.controls.zoom.x + delta).clamp(ZOOM_MIN, ZOOM_MAX);
            }
            Message::ZoomDeltaY(delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                self.controls.zoom.y = (self.controls.zoom.y + delta).clamp(ZOOM_MIN, ZOOM_MAX);
            }
            Message::UpdateMinPower(min_power) => {
                self.controls.power_bounds.0 = min_power.min(self.controls.power_bounds.1);
            }
            Message::UpdateMaxPower(max_power) => {
                self.controls.power_bounds.1 = max_power.max(self.controls.power_bounds.0);
            }
            Message::SetSatellites(satellites) => {
                self.satellites = satellites;
                // TODO: clear previous predictions here?
                log::debug!("Using {} satellites", self.satellites.len());
                let satellites = self.satellites.clone();
                let start_time = self.spectrogram.start_time;
                let length_s = self.spectrogram.length().num_milliseconds() as f64 / 1000.0;
                return cosmic::task::future(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        predict_satellites(satellites, start_time, length_s)
                    })
                    .await;
                    match result {
                        Ok(predictions) => Message::SetSatellitePredictions(Some(predictions)),
                        Err(e) => {
                            log::error!("Failed to predict satellite passes: {}", e);
                            Message::SetSatellitePredictions(None)
                        }
                    }
                });
            }
            Message::SetSatellitePredictions(predictions) => {
                self.satellite_predictions = predictions;
            }
        }
        Task::none()
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
            .and_then(|p| Some(coord::Screen(Vec2::new(p.x, p.y))));
        if let mouse::Event::WheelScrolled { delta } = event {
            let delta = match delta {
                mouse::ScrollDelta::Lines { x: _, y } => y,
                mouse::ScrollDelta::Pixels { x: _, y } => y,
            };
            if let Some(pos) = pos {
                return (
                    Status::Captured,
                    Some(Message::ZoomDelta(pos.plot(&bounds), delta)),
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
                        *state = MouseInteraction::Panning(pos.plot(&bounds));
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
                    let pos =
                        coord::Screen(Vec2::new(position.x - bounds.x, position.y - bounds.y))
                            .plot(&bounds);
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

fn predict_satellites(
    satellites: Vec<Satellite>,
    start_time: DateTime<Utc>,
    length_s: f64,
) -> Predictions {
    let times = ndarray::Array1::linspace(
        0.0, length_s, 1000, // TODO: number of points
    );
    // TODO: Make this configurable
    const SITE: rstrf::orbit::Site = rstrf::orbit::Site {
        latitude: 78.2244_f64.to_radians(),
        longitude: 15.3952_f64.to_radians(),
        altitude: 0.474,
    };
    // TODO: Parallelize predictions?
    let (frequencies, zenith_angles) = satellites
        .iter()
        .map(|sat| {
            let id = sat.norad_id();
            let (freq, za) = sat.predict_pass(start_time, times.view(), SITE);
            ((id, freq), (id, za))
        })
        .unzip();
    Predictions {
        times,
        frequencies,
        zenith_angles,
    }
}
