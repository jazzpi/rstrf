use iced::{
    Element, Length, Task,
    alignment::Vertical,
    widget::{self, Row, slider, text},
};
use rstrf::{
    colormap::Colormap,
    coord::{DataNormalizedToDataAbsolute, data_absolute, data_normalized, plot_area},
    spectrogram::Spectrogram,
};
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;

use crate::{
    widgets::{Icon, ToolbarButton, toolbar},
    windows::rfplot::{self, viewport::Viewport},
};

const ZOOM_MIN: f32 = 0.0;

const SIGMA_MIN: f32 = 0.1;
const SIGMA_MAX: f32 = 20.0;

const TRACK_BW_MIN: f32 = 1e3;
const TRACK_BW_MAX: f32 = 100e3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Controls {
    viewport: Viewport,
    /// Possible power range
    power_bounds: (f32, f32),
    /// Current power range for display
    power_range: (f32, f32),
    /// Threshold for signal detection
    signal_sigma: f32,
    /// Bandwidth around track points
    track_bw: f32,
    show_controls: bool,
    colormap: Colormap,
    average_plotting: bool,
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
    ZoomToRect(data_normalized::Rectangle),
    UpdateMinPower(f32),
    UpdateMaxPower(f32),
    UpdateSignalSigma(f32),
    UpdateTrackBW(f32),
    SetControlsVisible(bool),
    UpdateColormap(Colormap),
    UpdateAveragePlotting(bool),
}

impl Controls {
    pub fn set_spectrogram(&mut self, spec: &Spectrogram) {
        self.set_power_bounds(spec.power_bounds);
        let data = spec.bounds();
        self.viewport.set_data_bounds(data.0.width, data.0.height);
    }

    fn set_power_bounds(&mut self, bounds: (f32, f32)) {
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
        self.viewport.size()
    }

    pub fn bounds(&self) -> data_normalized::Rectangle {
        self.viewport.bounds()
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

    pub fn colormap(&self) -> Colormap {
        self.colormap
    }

    pub fn average_plotting(&self) -> bool {
        self.average_plotting
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

    pub fn view(&self, state: &super::State) -> Element<'_, rfplot::Message> {
        let colormaps = Colormap::iter()
            .map(|c| ToolbarButton::LabeledIcon {
                icon: Icon::Colormap(c),
                label: c.into(),
                tooltip: c.into(),
                msg: Message::UpdateColormap(c).into(),
                enabled: true,
                style: widget::button::primary,
            })
            .collect();
        let buttons = toolbar([
            ToolbarButton::Icon {
                icon: Icon::Sliders,
                tooltip: "Toggle controls",
                msg: Message::SetControlsVisible(!self.show_controls).into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::ZoomReset,
                tooltip: "Reset view & clear marks",
                msg: Message::ResetView.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::TogglePredictions,
                tooltip: "Toggle predictions",
                msg: rfplot::overlay::Message::TogglePredictions.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::Grid,
                tooltip: "Toggle grid",
                msg: rfplot::overlay::Message::ToggleGrid.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::ToggleAbsolute,
                tooltip: "Toggle absolute/relative axes",
                msg: rfplot::overlay::Message::ToggleAbsoluteAxes.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::Crosshair,
                tooltip: "Toggle crosshair",
                msg: rfplot::overlay::Message::ToggleCrosshair.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::MarkTrackpoint,
                tooltip: "Mark track points",
                msg: rfplot::overlay::Message::MarkTrackpoints.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::MarkSignal,
                tooltip: "Mark signals",
                msg: rfplot::overlay::Message::MarkSignals.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::Delete,
                tooltip: "Clear signals & track points",
                msg: rfplot::overlay::Message::ClearAll.into(),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::Save,
                tooltip: "Save signals to out.dat",
                msg: rfplot::overlay::Message::SaveSignals.into(),
                enabled: !state.marks.signals.is_empty(),
                style: widget::button::primary,
            },
            ToolbarButton::Icon {
                icon: Icon::Screenshot,
                tooltip: "Save screenshot",
                msg: rfplot::Message::CaptureScreenshot(None),
                enabled: true,
                style: widget::button::primary,
            },
            ToolbarButton::Submenu {
                toplevel: Box::new(ToolbarButton::Icon {
                    icon: Icon::Colormap(state.controls.colormap),
                    tooltip: "Colormap",
                    msg: rfplot::Message::Nop,
                    enabled: true,
                    style: widget::button::primary,
                }),
                submenu: colormaps,
            },
        ]);
        let mut result = widget::column![buttons].spacing(8);
        if self.show_controls
            && let Some(spectrogram) = &state.spectrogram
        {
            let bounds = self.bounds() * DataNormalizedToDataAbsolute::new(&spectrogram.bounds());
            let zoom_max = self.viewport.zoom_max();
            let log_scale = self.viewport.log_scale();
            result = result.push(
                widget::grid![
                    Self::control(
                        "Zoom Time",
                        slider(ZOOM_MIN..=zoom_max.x, log_scale.x, |z| {
                            Message::UpdateZoomX(z).into()
                        })
                        .step(0.01f32)
                        .width(Length::Fill),
                        format!("{:.0} s", bounds.0.width),
                    ),
                    Self::control(
                        "Zoom Freq",
                        slider(ZOOM_MIN..=zoom_max.y, log_scale.y, |z| {
                            Message::UpdateZoomY(z).into()
                        })
                        .step(0.01f32)
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
                        .step(0.1f32)
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
                        .step(0.1f32)
                        .width(Length::Fill),
                        format!("{:.1} dB", self.power_range.1),
                    ),
                    Self::control(
                        "Signal Thresh",
                        slider(SIGMA_MIN..=SIGMA_MAX, self.signal_sigma, |s| {
                            Message::UpdateSignalSigma(s).into()
                        })
                        .step(0.1f32)
                        .width(Length::Fill),
                        format!("{:.1}", self.signal_sigma),
                    ),
                    Self::control(
                        "Track BW",
                        slider(TRACK_BW_MIN..=TRACK_BW_MAX, self.track_bw, |b| {
                            Message::UpdateTrackBW(b).into()
                        })
                        .step(100.0f32)
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

    pub fn update(&mut self, message: Message) -> Task<rfplot::Message> {
        match message {
            Message::UpdateZoomX(zoom_x) => self.viewport.set_zoom_x(zoom_x),
            Message::UpdateZoomY(zoom_y) => self.viewport.set_zoom_y(zoom_y),
            Message::PanningDelta(delta) => self.viewport.pan_by(delta),
            Message::ZoomDelta(plot_pos, delta) => self.viewport.zoom_at(plot_pos, delta),
            Message::ZoomDeltaX(plot_pos, delta) => self.viewport.zoom_x_at(plot_pos, delta),
            Message::ZoomDeltaY(plot_pos, delta) => self.viewport.zoom_y_at(plot_pos, delta),
            Message::ResetView => self.viewport.reset(),
            Message::ZoomToRect(rect) => self.viewport.set_view_from_rect_dn(&rect),
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
            Message::SetControlsVisible(visible) => self.show_controls = visible,
            Message::UpdateColormap(colormap) => self.colormap = colormap,
            Message::UpdateAveragePlotting(average) => self.average_plotting = average,
        }
        Task::none()
    }

    /// Set the view to show `rect`, clamping zoom to the allowed range.
    pub fn set_view_from_rect_da(
        &mut self,
        rect: &data_absolute::Rectangle,
        spec_bounds: &data_absolute::Rectangle,
    ) {
        self.viewport.set_view_from_rect_da(rect, spec_bounds);
    }

    /// Override the displayed power range. Clamps to current power_bounds.
    pub fn set_power_range(&mut self, zmin: Option<f32>, zmax: Option<f32>) {
        if let Some(z) = zmin {
            self.power_range.0 = z.clamp(self.power_bounds.0, self.power_bounds.1);
        }
        if let Some(z) = zmax {
            self.power_range.1 = z.clamp(self.power_bounds.0, self.power_bounds.1);
        }
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            viewport: Viewport::default(),
            power_bounds: (0.0, 0.0),
            power_range: (0.0, 0.0),
            signal_sigma: 5.0,
            track_bw: 10e3,
            show_controls: true,
            colormap: Default::default(),
            average_plotting: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(c: &mut Controls, msg: Message) {
        let _ = c.update(msg);
    }

    #[test]
    fn set_power_bounds_initializes_range() {
        let mut c = Controls::default();
        c.set_power_bounds((-50.0, -10.0));
        assert_eq!(c.power_range(), (-50.0, -10.0));
    }

    #[test]
    fn set_power_bounds_clamps_existing_range() {
        let mut c = Controls::default();
        c.set_power_bounds((-50.0, -10.0));
        c.set_power_bounds((-30.0, -20.0));
        let (lo, hi) = c.power_range();
        assert!(lo >= -30.0 && lo <= -20.0, "lo={}", lo);
        assert!(hi >= -30.0 && hi <= -20.0, "hi={}", hi);
    }

    #[test]
    fn update_min_power_cannot_exceed_max() {
        let mut c = Controls::default();
        c.set_power_bounds((-50.0, -10.0));
        update(&mut c, Message::UpdateMaxPower(-20.0));
        update(&mut c, Message::UpdateMinPower(-10.0));
        let (lo, hi) = c.power_range();
        assert!(lo <= hi, "lo={} > hi={}", lo, hi);
    }

    #[test]
    fn set_controls_visible_changes_visibility() {
        let mut c = Controls::default();
        assert!(c.show_controls);
        update(&mut c, Message::SetControlsVisible(false));
        assert!(!c.show_controls);
        update(&mut c, Message::SetControlsVisible(true));
        assert!(c.show_controls);
    }
}
