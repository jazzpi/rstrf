use glam::Vec2;
use iced::{
    Element, Length, Task,
    alignment::Vertical,
    widget::{self, Row, slider, text},
};
use rstrf::coord::{
    DataNormalizedToDataAbsolute, PlotAreaToDataNormalized, data_normalized, plot_area,
};
use serde::{Deserialize, Serialize};

use crate::{
    panes::rfplot,
    widgets::{Icon, icon_button, toolbar},
};

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 8.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

const SIGMA_MIN: f32 = 0.1;
const SIGMA_MAX: f32 = 20.0;

const TRACK_BW_MIN: f32 = 1e3;
const TRACK_BW_MAX: f32 = 100e3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Controls {
    log_scale: Vec2,
    center: data_normalized::Point,
    /// Possible power range
    power_bounds: (f32, f32),
    /// Current power range for display
    power_range: (f32, f32),
    /// Threshold for signal detection
    signal_sigma: f32,
    /// Bandwidth around track points
    track_bw: f32,
    #[serde(default)]
    show_controls: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(plot_area::Vector),
    ZoomDelta(plot_area::Point, f32),
    ZoomDeltaX(plot_area::Point, f32),
    ZoomDeltaY(plot_area::Point, f32),
    ResetView,
    UpdateMinPower(f32),
    UpdateMaxPower(f32),
    UpdateSignalSigma(f32),
    UpdateTrackBW(f32),
    ToggleControls,
}

impl Controls {
    pub fn set_power_bounds(&mut self, bounds: (f32, f32)) {
        self.power_bounds = bounds;
        self.power_range = if self.power_range == (0.0, 0.0) {
            bounds
        } else {
            (
                self.power_range.0.clamp(bounds.0, bounds.1),
                self.power_range.1.clamp(bounds.0, bounds.1),
            )
        };
    }

    pub fn size(&self) -> data_normalized::Size {
        data_normalized::Size::new(
            1.0 / 2.0_f32.powf(self.log_scale.x),
            1.0 / 2.0_f32.powf(self.log_scale.y),
        )
    }

    pub fn bounds(&self) -> data_normalized::Rectangle {
        let size = self.size();
        data_normalized::Rectangle::new(
            data_normalized::Point::new(
                self.center.0.x - size.0.width / 2.0,
                self.center.0.y - size.0.height / 2.0,
            ),
            size,
        )
    }

    pub fn data_normalized(&self) -> PlotAreaToDataNormalized {
        PlotAreaToDataNormalized::new(&self.bounds())
    }

    pub fn power_range(&self) -> (f32, f32) {
        self.power_range
    }

    pub fn signal_sigma(&self) -> f32 {
        self.signal_sigma
    }

    pub fn track_bw(&self) -> f32 {
        self.track_bw
    }

    fn control<'a>(
        label: &'static str,
        control: impl Into<Element<'a, rfplot::Message>>,
        value: impl Into<String>,
    ) -> Row<'a, rfplot::Message> {
        widget::row![
            text(label).width(Length::FillPortion(3)),
            widget::container(control).width(Length::FillPortion(5)),
            text(value.into()).width(Length::FillPortion(2)),
        ]
        .spacing(4)
        .align_y(Vertical::Center)
    }

    pub fn view(&self, shared: &super::SharedState) -> Element<'_, rfplot::Message> {
        let buttons = toolbar([
            icon_button(
                Icon::Sliders,
                "Toggle controls",
                Message::ToggleControls.into(),
                widget::button::primary,
            ),
            icon_button(
                Icon::ZoomReset,
                "Reset view",
                Message::ResetView.into(),
                widget::button::primary,
            ),
            icon_button(
                Icon::TogglePredictions,
                "Toggle predictions",
                rfplot::overlay::Message::TogglePredictions.into(),
                widget::button::primary,
            ),
            icon_button(
                Icon::Grid,
                "Toggle grid",
                rfplot::overlay::Message::ToggleGrid.into(),
                widget::button::primary,
            ),
            icon_button(
                Icon::Crosshair,
                "Toggle crosshair",
                rfplot::overlay::Message::ToggleCrosshair.into(),
                widget::button::primary,
            ),
        ]);
        let mut result = widget::column![buttons].spacing(8);
        if self.show_controls
            && let Some(spectrogram) = &shared.spectrogram
        {
            let bounds = self.bounds() * DataNormalizedToDataAbsolute::new(&spectrogram.bounds());
            result = result.push(
                widget::grid![
                    Self::control(
                        "Zoom Time",
                        slider(ZOOM_MIN..=ZOOM_MAX, self.log_scale.x, |z| {
                            Message::UpdateZoomX(z).into()
                        })
                        .step(0.01)
                        .width(Length::Fill),
                        format!("{:.0} s", bounds.0.width),
                    ),
                    Self::control(
                        "Zoom Freq",
                        slider(ZOOM_MIN..=ZOOM_MAX, self.log_scale.y, |z| {
                            Message::UpdateZoomY(z).into()
                        })
                        .step(0.01)
                        .width(Length::Fill),
                        format!("{:.0} kHz", bounds.0.height / 1000.0),
                    ),
                    Self::control(
                        "Min Power",
                        slider(
                            self.power_bounds.0..=self.power_bounds.1,
                            self.power_range.0,
                            |p| Message::UpdateMinPower(p).into(),
                        )
                        .step(0.1)
                        .width(Length::Fill),
                        format!("{:.1} dB", self.power_range.0),
                    ),
                    Self::control(
                        "Max Power",
                        slider(
                            self.power_bounds.0..=self.power_bounds.1,
                            self.power_range.1,
                            |p| Message::UpdateMaxPower(p).into(),
                        )
                        .step(0.1)
                        .width(Length::Fill),
                        format!("{:.1} dB", self.power_range.1),
                    ),
                    Self::control(
                        "Signal Thresh",
                        slider(SIGMA_MIN..=SIGMA_MAX, self.signal_sigma, |s| {
                            Message::UpdateSignalSigma(s).into()
                        })
                        .step(0.1)
                        .width(Length::Fill),
                        format!("{:.1}", self.signal_sigma),
                    ),
                    Self::control(
                        "Track BW",
                        slider(TRACK_BW_MIN..=TRACK_BW_MAX, self.track_bw, |b| {
                            Message::UpdateTrackBW(b).into()
                        })
                        .step(100.0)
                        .width(Length::Fill),
                        format!("{:.1} kHz", self.track_bw / 1000.0),
                    ),
                ]
                .columns(2)
                .spacing(8)
                .height(Length::Shrink),
            );
        }
        widget::container(result)
            .padding(8)
            .width(Length::Fill)
            .style(widget::container::bordered_box)
            .into()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UpdateZoomX(zoom_x) => {
                self.log_scale.x = zoom_x;
            }
            Message::UpdateZoomY(zoom_y) => {
                self.log_scale.y = zoom_y;
            }
            Message::PanningDelta(delta) => {
                self.center -= delta * self.data_normalized();
            }
            Message::ZoomDelta(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;

                let old_data = plot_pos * self.data_normalized();
                let prev_zoom = self.log_scale;
                self.log_scale = (prev_zoom + Vec2::splat(delta))
                    .clamp(Vec2::splat(ZOOM_MIN), Vec2::splat(ZOOM_MAX));
                let new_data = plot_pos * self.data_normalized();
                self.center += old_data - new_data;
            }
            Message::ZoomDeltaX(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let old_x = (plot_pos * self.data_normalized()).0.x;
                self.log_scale.x = (self.log_scale.x + delta).clamp(ZOOM_MIN, ZOOM_MAX);
                let new_x = (plot_pos * self.data_normalized()).0.x;
                self.center.0.x += old_x - new_x;
            }
            Message::ZoomDeltaY(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let old_y = (plot_pos * self.data_normalized()).0.y;
                self.log_scale.y = (self.log_scale.y + delta).clamp(ZOOM_MIN, ZOOM_MAX);
                let new_y = (plot_pos * self.data_normalized()).0.y;
                self.center.0.y += old_y - new_y;
            }
            Message::ResetView => {
                self.log_scale = Vec2::new(ZOOM_MIN, ZOOM_MIN);
                self.center = data_normalized::Point::new(0.5, 0.5);
            }
            Message::UpdateMinPower(min_power) => {
                self.power_range.0 = min_power.min(self.power_range.1);
            }
            Message::UpdateMaxPower(max_power) => {
                self.power_range.1 = max_power.max(self.power_range.0);
            }
            Message::UpdateSignalSigma(sigma) => {
                self.signal_sigma = sigma;
            }
            Message::UpdateTrackBW(bw) => {
                self.track_bw = bw;
            }
            Message::ToggleControls => self.show_controls = !self.show_controls,
        }
        self.snap_to_bounds();
        Task::none()
    }

    /// Ensure that the current view bounds are within [0, 1] in both axes.
    fn snap_to_bounds(&mut self) {
        let bounds = self.bounds().0;
        let dx = if bounds.x < 0.0 {
            -bounds.x
        } else if bounds.x + bounds.width > 1.0 {
            1.0 - (bounds.x + bounds.width)
        } else {
            0.0
        };
        let dy = if bounds.y < 0.0 {
            -bounds.y
        } else if bounds.y + bounds.height > 1.0 {
            1.0 - (bounds.y + bounds.height)
        } else {
            0.0
        };
        self.center.0.x += dx;
        self.center.0.y += dy;
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            log_scale: Vec2::new(ZOOM_MIN, ZOOM_MIN),
            center: data_normalized::Point::new(0.5, 0.5),
            power_bounds: (0.0, 0.0),
            power_range: (0.0, 0.0),
            signal_sigma: 5.0,
            track_bw: 10e3,
            show_controls: true,
        }
    }
}
