//! The toolbar and the collapsible controls panel above the plot.

use iced::{
    Element, Length,
    alignment::Vertical,
    widget::{self, Row, slider, text},
};
use rstrf::{colormap::Colormap, coord::DataNormalizedToDataAbsolute};
use strum::IntoEnumIterator;

use crate::{
    widgets::{Icon, ToolbarButton, toolbar},
    windows::rfplot::{self, DisplayMsg, MarksMsg, ViewMsg},
};

const ZOOM_MIN: f32 = 0.0;

const SIGMA_MIN: f32 = 0.1;
const SIGMA_MAX: f32 = 20.0;

const TRACK_BW_MIN: f32 = 1e3;
const TRACK_BW_MAX: f32 = 100e3;

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

pub fn view(state: &super::State) -> Element<'_, rfplot::Message> {
    let colormaps = Colormap::iter()
        .map(|c| ToolbarButton::LabeledIcon {
            icon: Icon::Colormap(c),
            label: c.into(),
            tooltip: c.into(),
            msg: DisplayMsg::UpdateColormap(c).into(),
            enabled: true,
            style: widget::button::primary,
        })
        .collect();
    let buttons = toolbar([
        ToolbarButton::Icon {
            icon: Icon::Sliders,
            tooltip: "Toggle controls",
            msg: DisplayMsg::SetControlsVisible(!state.display.show_controls).into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::ZoomReset,
            tooltip: "Reset view & clear marks",
            msg: ViewMsg::ResetView.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::TogglePredictions,
            tooltip: "Toggle predictions",
            msg: DisplayMsg::TogglePredictions.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::Grid,
            tooltip: "Toggle grid",
            msg: DisplayMsg::ToggleGrid.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::ToggleAbsolute,
            tooltip: "Toggle absolute/relative axes",
            msg: DisplayMsg::ToggleAbsoluteAxes.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::Crosshair,
            tooltip: "Toggle crosshair",
            msg: DisplayMsg::ToggleCrosshair.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::MarkTrackpoint,
            tooltip: "Mark track points",
            msg: MarksMsg::MarkTrackpoints.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::MarkSignal,
            tooltip: "Mark signals",
            msg: MarksMsg::MarkSignals.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::Delete,
            tooltip: "Clear signals & track points",
            msg: MarksMsg::ClearAll.into(),
            enabled: true,
            style: widget::button::primary,
        },
        ToolbarButton::Icon {
            icon: Icon::Save,
            tooltip: "Save signals to out.dat",
            msg: MarksMsg::SaveSignals.into(),
            enabled: !state.marks.signals().is_empty(),
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
                icon: Icon::Colormap(state.display.colormap),
                tooltip: "Colormap",
                msg: rfplot::Message::Nop,
                enabled: true,
                style: widget::button::primary,
            }),
            submenu: colormaps,
        },
    ]);
    let mut result = widget::column![buttons].spacing(8);
    if state.display.show_controls
        && let Some(spectrogram) = &state.spectrogram()
    {
        let bounds =
            state.viewport.bounds() * DataNormalizedToDataAbsolute::new(&spectrogram.bounds());
        let zoom_max = state.viewport.zoom_max();
        let log_scale = state.viewport.log_scale();
        let power_bounds = state.power.bounds();
        let power_range = state.power.range();
        result = result.push(
            widget::grid![
                control(
                    "Zoom Time",
                    slider(ZOOM_MIN..=zoom_max.x, log_scale.x, |z| {
                        ViewMsg::UpdateZoomX(z).into()
                    })
                    .step(0.01f32)
                    .width(Length::Fill),
                    format!("{:.0} s", bounds.0.width),
                ),
                control(
                    "Zoom Freq",
                    slider(ZOOM_MIN..=zoom_max.y, log_scale.y, |z| {
                        ViewMsg::UpdateZoomY(z).into()
                    })
                    .step(0.01f32)
                    .width(Length::Fill),
                    format!("{:.0} kHz", bounds.0.height / 1000.0),
                ),
                control(
                    "Min Power",
                    slider(power_bounds.0..=power_bounds.1, power_range.0, |p| {
                        ViewMsg::UpdateMinPower(p).into()
                    })
                    .step(0.1f32)
                    .width(Length::Fill),
                    format!("{:.1} dB", power_range.0),
                ),
                control(
                    "Max Power",
                    slider(power_bounds.0..=power_bounds.1, power_range.1, |p| {
                        ViewMsg::UpdateMaxPower(p).into()
                    })
                    .step(0.1f32)
                    .width(Length::Fill),
                    format!("{:.1} dB", power_range.1),
                ),
                control(
                    "Signal Thresh",
                    slider(SIGMA_MIN..=SIGMA_MAX, state.detection.signal_sigma, |s| {
                        MarksMsg::UpdateSignalSigma(s).into()
                    })
                    .step(0.1f32)
                    .width(Length::Fill),
                    format!("{:.1}", state.detection.signal_sigma),
                ),
                control(
                    "Track BW",
                    slider(TRACK_BW_MIN..=TRACK_BW_MAX, state.detection.track_bw, |b| {
                        MarksMsg::UpdateTrackBW(b).into()
                    })
                    .step(100.0f32)
                    .width(Length::Fill),
                    format!("{:.1} kHz", state.detection.track_bw / 1000.0),
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
