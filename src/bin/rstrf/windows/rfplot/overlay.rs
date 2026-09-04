//! This module contains the plot overlay for RFPlot. It draws everything that isn't the spectrogram
//! itself (like axes and overlays). It is also responsible for the user interaction with the plot
//! (like panning/zooming).

use chrono::Duration;
use copy_range::CopyRange;
use iced::{
    Rectangle, Task,
    event::Status,
    keyboard::{self, key::Named},
    mouse,
    widget::canvas,
};
use itertools::{Itertools, izip};
use ndarray::s;
use plotters::coord::types::RangedCoordf32;
use plotters::coord::{
    combinators::WithKeyPoints,
    ranged1d::{KeyPointHint, NoDefaultFormatting, ValueFormatter},
};
use plotters::prelude::*;
use plotters_iced2::Chart;
use rstrf::{
    chart::{ReferenceMode, ReferencedTicks, datetime_referenced_ticks},
    coord::{
        DataAbsoluteToDataNormalized, DataAbsoluteToScreen, DataNormalizedToDataAbsolute,
        PlotAreaToDataAbsolute, ScreenToPlotArea, data_absolute, plot_area, screen,
    },
    signal,
    util::{clip_line, is_modifier, sec_to_duration},
};

use rfd::AsyncFileDialog;

use crate::{app::AppShared, windows::rfplot::MarkAction};

use super::marks::signals_filename;

use super::{
    DisplayMsg, MarksMsg, MouseState, PlotChart, PredictionsMsg, RectAction, State, ViewMsg,
};

/// Maximum cursor-to-mark distance (in screen pixels) for a right-click to delete a mark. Marks
/// render as radius-5 circles, so this gives a comfortable grab radius around them.
const DELETE_TOLERANCE_PX: f32 = 15.0;

const ORANGE: RGBColor = RGBColor(255, 165, 0);

fn prediction_color(classification: Option<sgp4::Classification>) -> RGBColor {
    match classification {
        Some(sgp4::Classification::Secret) => RED,
        Some(sgp4::Classification::Classified) => ORANGE,
        Some(sgp4::Classification::Unclassified) | None => GREEN,
    }
}

fn clamp_line_to_plot(
    bounds: &data_absolute::Rectangle,
    points: impl Iterator<Item = data_absolute::Point>,
) -> impl Iterator<Item = data_absolute::Point> {
    points
        .tuple_windows()
        .filter_map(|(a, b)| clip_line(&bounds.0, a.0, b.0))
        .flat_map(|(a, b)| vec![a, b])
        .map(data_absolute::Point)
}

/// We want to use `with_key_points()`, which creates a `WithKeyPoints<RangedCoordf32>`. That
/// doesn't impl `ValueFormatter<f32>`, which breaks the `configure_mesh()` call. Due to the orphan
/// rule we can't `impl` it ourselves, so we wrap the `WithKeyPoints` in a newtype.
struct FmtWithKeyPoints(WithKeyPoints<RangedCoordf32>);

impl Ranged for FmtWithKeyPoints {
    type FormatOption = NoDefaultFormatting;
    type ValueType = f32;

    fn map(&self, value: &f32, limit: (i32, i32)) -> i32 {
        self.0.map(value, limit)
    }

    fn key_points<Hint: KeyPointHint>(&self, hint: Hint) -> Vec<f32> {
        self.0.key_points(hint)
    }

    fn range(&self) -> std::ops::Range<f32> {
        self.0.range()
    }
}

impl ValueFormatter<f32> for FmtWithKeyPoints {
    fn format_ext(&self, value: &f32) -> String {
        RangedCoordf32::format(value)
    }
}

impl State {
    fn build_chart<DB: DrawingBackend>(
        &self,
        mut chart: ChartBuilder<DB>,
        app: &AppShared,
    ) -> Result<(), String> {
        let Some(spectrogram) = &self.spectrogram else {
            return Err("No spectrogram loaded".to_string());
        };
        let view_norm = self.viewport.bounds();
        let bounds = view_norm * DataNormalizedToDataAbsolute::new(&spectrogram.bounds());
        let x = CopyRange::from_std(bounds.0.x..(bounds.0.x + bounds.0.width));
        let y = CopyRange::from_std(bounds.0.y..(bounds.0.y + bounds.0.height));

        // Let plotters pick some nice numbers for the ticks
        const NUM_TICKS: usize = 11;
        let mut y_ticks = ReferencedTicks {
            ticks: RangedCoordf32::from(y.into_std()).key_points(NUM_TICKS),
            reference: bounds.0.y + bounds.0.height / 2.0,
            mode: ReferenceMode::Center,
        };
        let x_ticks = if self.display.absolute_axes {
            y_ticks.snap(y, spectrogram.freq);
            datetime_referenced_ticks(x, spectrogram.start_time(), NUM_TICKS)
        } else {
            RangedCoordf32::from(x.into_std()).key_points(NUM_TICKS)
        };

        let mut chart = chart
            .x_label_area_size(self.plot_area_margin)
            .y_label_area_size(self.plot_area_margin)
            .build_cartesian_2d(
                FmtWithKeyPoints(x.into_std().with_key_points(x_ticks)),
                FmtWithKeyPoints(y.into_std().with_key_points(y_ticks.ticks)),
            )
            .map_err(|e| format!("Failed to build chart: {:?}", e))?;

        let mut mesh = chart.configure_mesh();
        let mut frame = mesh
            .max_light_lines(0)
            .axis_style(WHITE)
            .label_style(&WHITE)
            .bold_line_style(WHITE.mix(0.4));

        let start_time = spectrogram.start_time();
        let duration = sec_to_duration(x.end - x.start);
        let (x_tick_format, x_axis_date_format, x_axis_label) = if duration > Duration::days(1)
            || (start_time + sec_to_duration(x.start)).date_naive()
                != (start_time + sec_to_duration(x.end)).date_naive()
        {
            ("%d %H:%M", "%Y-%m", "DD HH:MM")
        } else {
            ("%H:%M", "%Y-%m-%d", "HH:MM")
        };
        let x_formatter = |v: &f32| {
            let t = start_time + sec_to_duration(*v);
            format!("{}", t.format(x_tick_format))
        };
        let y_formatter = |v: &f32| format!("{:.1}", (v - y_ticks.reference) / 1000.0);
        if self.display.absolute_axes {
            frame = frame
                .x_label_formatter(&x_formatter)
                .y_label_formatter(&y_formatter)
                .x_desc(format!(
                    "Time - {} [{}]",
                    (start_time + sec_to_duration(x.start)).format(x_axis_date_format),
                    x_axis_label
                ))
                .y_desc(format!(
                    "Frequency - {:.1} [kHz]",
                    (spectrogram.freq + y_ticks.reference) / 1000.0
                ));
        } else {
            frame = frame
                .y_label_formatter(&|v| format!("{:.1}", v / 1000.0))
                .x_desc("Time [s]")
                .y_desc("Frequency offset [kHz]");
        }
        if !self.display.show_grid {
            frame = frame.disable_mesh();
        }

        frame
            .draw()
            .map_err(|e| format!("Failed to draw mesh: {:?}", e))?;

        // Thicken the plot border on any side where the view has hit the edge of the data, so
        // panning/zooming limits are visible at a glance.
        const EDGE_EPS: f32 = 1e-4;
        const THICK_STROKE: u32 = 3;
        let at_left = view_norm.0.x <= EDGE_EPS;
        let at_right = view_norm.0.x + view_norm.0.width >= 1.0 - EDGE_EPS;
        let at_bottom = view_norm.0.y <= EDGE_EPS;
        let at_top = view_norm.0.y + view_norm.0.height >= 1.0 - EDGE_EPS;
        for (at_edge, from, to) in [
            (at_left, (x.start, y.start), (x.start, y.end)),
            (at_right, (x.end, y.start), (x.end, y.end)),
            (at_bottom, (x.start, y.start), (x.end, y.start)),
            (at_top, (x.start, y.end), (x.end, y.end)),
        ] {
            if at_edge {
                chart
                    .draw_series(LineSeries::new(
                        [from, to],
                        ShapeStyle {
                            color: WHITE.into(),
                            filled: true,
                            stroke_width: THICK_STROKE,
                        },
                    ))
                    .map_err(|e| format!("Failed to draw edge indicator: {:?}", e))?;
            }
        }

        if self.display.show_predictions
            && let Some((_, predictions)) = self.prediction_cache.get_stored()
        {
            let time = &predictions.times;
            for prediction in predictions.iter_satellites() {
                let (id, passes) = prediction;
                let color = prediction_color(app.satellite_classification(id));
                log::trace!("Plotting {} passes for satellite {}", passes.len(), id);
                for pass in passes {
                    let time = time.slice(s![pass.time_range.clone()]);
                    // First, check only x to find possibly visible time frames
                    let visible_x = time.iter().map(|&t| x.contains(&(t as f32))).collect_vec();
                    for freq in pass.frequencies.iter() {
                        let first_visible =
                            izip!(visible_x.iter(), freq.iter()).position(|(&visible, &f)| {
                                visible && y.contains(&(f as f32 - spectrogram.freq))
                            });
                        let Some(first_visible) = first_visible else {
                            continue;
                        };

                        chart
                            .draw_series(LineSeries::new(
                                izip!(time.iter(), freq.iter())
                                    .map(|(&t, &f)| (t as f32, (f as f32 - spectrogram.freq))),
                                &color,
                            ))
                            .map_err(|e| {
                                format!("Could not draw line for satellite {}: {:?}", id, e)
                            })?
                            .label(format!("{:06}", id));

                        let first_time = (time[first_visible] as f32).max(x.start);
                        let first_freq = freq[first_visible] as f32 - spectrogram.freq;
                        chart
                            .draw_series(vec![Text::new(
                                format!("{:06}", id),
                                (first_time, first_freq),
                                ("sans-serif", 12).into_font().color(&color),
                            )])
                            .map_err(|e| {
                                format!("Could not draw label for satellite {}: {:?}", id, e)
                            })?;
                    }
                }
            }
        }

        chart
            .draw_series(self.marks.track_points().iter().filter_map(|pos| {
                if bounds.contains(*pos) {
                    Some(Circle::new(pos.into(), 5, YELLOW.filled()))
                } else {
                    None
                }
            }))
            .map_err(|e| format!("Could not draw track points: {:?}", e))?;
        chart
            .draw_series(LineSeries::new(
                clamp_line_to_plot(
                    &bounds,
                    self.marks.track_points().iter().map(|pos| {
                        data_absolute::Point::new(pos.0.x, pos.0.y + self.detection.track_bw / 2.0)
                    }),
                )
                .map(|v| v.into()),
                &YELLOW,
            ))
            .map_err(|e| {
                format!(
                    "Could not draw lines connecting track points (above): {:?}",
                    e
                )
            })?;
        chart
            .draw_series(LineSeries::new(
                clamp_line_to_plot(
                    &bounds,
                    self.marks.track_points().iter().map(|pos| {
                        data_absolute::Point::new(pos.0.x, pos.0.y - self.detection.track_bw / 2.0)
                    }),
                )
                .map(|v| v.into()),
                &YELLOW,
            ))
            .map_err(|e| {
                format!(
                    "Could not draw lines connecting track points (below): {:?}",
                    e
                )
            })?;

        chart
            .draw_series(self.marks.signals().iter().filter_map(|pos| {
                if bounds.contains(*pos) {
                    Some(Circle::new(pos.into(), 5, WHITE.filled()))
                } else {
                    None
                }
            }))
            .map_err(|e| format!("Could not draw track points: {:?}", e))?;
        if self.display.show_crosshair
            && let Some(crosshair) = &self.interaction.crosshair.get()
            && bounds.contains(*crosshair)
        {
            let style = ShapeStyle {
                color: WHITE.mix(0.5),
                filled: false,
                stroke_width: 1,
            };
            // Vertical line
            chart
                .draw_series(LineSeries::new(
                    vec![
                        data_absolute::Point::new(crosshair.0.x, bounds.0.y),
                        data_absolute::Point::new(crosshair.0.x, bounds.0.y + bounds.0.height),
                    ]
                    .into_iter()
                    .map(|p| p.into()),
                    style,
                ))
                .map_err(|e| format!("Could not draw crosshair vertical line: {:?}", e))?;
            // Horizontal line
            chart
                .draw_series(LineSeries::new(
                    vec![
                        data_absolute::Point::new(bounds.0.x, crosshair.0.y),
                        data_absolute::Point::new(bounds.0.x + bounds.0.width, crosshair.0.y),
                    ]
                    .into_iter()
                    .map(|p| p.into()),
                    style,
                ))
                .map_err(|e| format!("Could not draw crosshair horizontal line: {:?}", e))?;
            let crosshair_norm =
                *crosshair * DataAbsoluteToDataNormalized::new(&spectrogram.bounds());
            let dim = spectrogram.data().dim();
            let power = spectrogram.data()[(
                ((crosshair_norm.0.x * (dim.0 as f32)).floor() as usize).clamp(0, dim.0 - 1),
                ((crosshair_norm.0.y * (dim.1 as f32)).floor() as usize).clamp(0, dim.1 - 1),
            )];
            let crosshair_pos = plot_area::Point::new(0.01, 0.99)
                * PlotAreaToDataAbsolute::new(&self.viewport.bounds(), &spectrogram.bounds());
            let crosshair_text = if self.display.absolute_axes {
                let t = spectrogram.start_time() + sec_to_duration(crosshair.0.x);
                format!(
                    "t = {}\nf = {:.01} kHz\nP = {:.01} dB",
                    t.format("%Y-%m-%d %H:%M:%S"),
                    (crosshair.0.y + spectrogram.freq) / 1000.0,
                    power
                )
            } else {
                format!(
                    "t = {:.01} s\nf = {:.01} kHz\nP = {:.01} dB",
                    crosshair.0.x,
                    crosshair.0.y / 1000.0,
                    power
                )
            };
            chart
                .draw_series(vec![Text::new(
                    crosshair_text,
                    crosshair_pos.into(),
                    ("sans-serif", 12).into_font().color(&WHITE),
                )])
                .expect("Could not draw crosshair label");
        }

        if let MouseState::DrawingRect {
            action,
            corner1,
            corner2,
        } = self.interaction.mouse_state.get()
        {
            let pa_to_da =
                PlotAreaToDataAbsolute::new(&self.viewport.bounds(), &spectrogram.bounds());
            let c1: (f32, f32) = (corner1 * pa_to_da).into();
            let c2: (f32, f32) = (corner2 * pa_to_da).into();
            let (fill_color, border_color) = match action {
                RectAction::Delete => (RED.mix(0.25), RED.mix(1.0)),
                RectAction::Zoom => (CYAN.mix(0.15), CYAN.mix(1.0)),
                RectAction::MarkCentroid => (YELLOW.mix(0.15), YELLOW.mix(1.0)),
            };
            chart
                .draw_series(std::iter::once(plotters::element::Rectangle::new(
                    [c1, c2],
                    fill_color.filled(),
                )))
                .map_err(|e| format!("Could not draw rect fill: {:?}", e))?;
            chart
                .draw_series(std::iter::once(plotters::element::Rectangle::new(
                    [c1, c2],
                    ShapeStyle {
                        color: border_color,
                        filled: false,
                        stroke_width: 1,
                    },
                )))
                .map_err(|e| format!("Could not draw rect border: {:?}", e))?;
        }

        Ok(())
    }

    fn handle_mouse(
        &self,
        event: &mouse::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<super::Message>) {
        let Some(cursor_pos) = cursor.position() else {
            return (Status::Ignored, None);
        };
        let pos = screen::Point::new(cursor_pos.x - bounds.x, cursor_pos.y - bounds.y);
        let plot_pos = pos * ScreenToPlotArea::new(&screen::Size(bounds.size()));
        let modifiers = self.interaction.modifiers.get();
        if let mouse::Event::WheelScrolled { delta } = event {
            let delta = match delta {
                mouse::ScrollDelta::Lines { x: _, y } => y,
                mouse::ScrollDelta::Pixels { x: _, y } => y,
            };
            let x_axis = Rectangle {
                x: bounds.x,
                y: bounds.y + bounds.height,
                width: bounds.width,
                height: self.plot_area_margin,
            };
            let y_axis = Rectangle {
                x: bounds.x - self.plot_area_margin,
                y: bounds.y,
                width: self.plot_area_margin,
                height: bounds.height,
            };
            if cursor.is_over(bounds) {
                if modifiers.shift() {
                    return (
                        Status::Captured,
                        Some(ViewMsg::ZoomDeltaX(plot_pos, *delta).into()),
                    );
                } else if modifiers.control() {
                    return (
                        Status::Captured,
                        Some(ViewMsg::ZoomDeltaY(plot_pos, *delta).into()),
                    );
                }
                return (
                    Status::Captured,
                    Some(ViewMsg::ZoomDelta(plot_pos, *delta).into()),
                );
            } else if cursor.is_over(y_axis) {
                return (
                    Status::Captured,
                    Some(ViewMsg::ZoomDeltaY(plot_pos, *delta).into()),
                );
            } else if cursor.is_over(x_axis) {
                return (
                    Status::Captured,
                    Some(ViewMsg::ZoomDeltaX(plot_pos, *delta).into()),
                );
            }
        }

        let update_crosshair =
            matches!(event, mouse::Event::CursorMoved { .. }) && self.display.show_crosshair;
        if update_crosshair {
            if cursor.is_over(bounds)
                && let Some(spectrogram) = &self.spectrogram
            {
                let pos = plot_pos
                    * PlotAreaToDataAbsolute::new(&self.viewport.bounds(), &spectrogram.bounds());
                self.interaction.crosshair.set(Some(pos));
            } else {
                self.interaction.crosshair.set(None);
            }
        }

        match self.interaction.mouse_state.get() {
            MouseState::Idle => match event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if cursor.is_over(bounds) {
                        self.interaction
                            .mouse_state
                            .set(MouseState::Panning(plot_pos));
                        return (Status::Captured, None);
                    }
                }
                mouse::Event::ButtonPressed(mouse::Button::Right) => {
                    if cursor.is_over(bounds)
                        && let Some(spectrogram) = &self.spectrogram
                        && let Some((action, point)) = closest_mark(
                            pos,
                            &DataAbsoluteToScreen::new(
                                &screen::Size(bounds.size()),
                                &self.viewport.bounds(),
                                &spectrogram.bounds(),
                            ),
                            self.marks.track_points(),
                            self.marks.signals(),
                        )
                    {
                        return (
                            Status::Captured,
                            Some(MarksMsg::DeleteMark(action, point).into()),
                        );
                    }
                    return (Status::Captured, None);
                }
                _ => {}
            },
            MouseState::Panning(prev_pos) => match event {
                mouse::Event::ButtonReleased(mouse::Button::Left) => {
                    self.interaction.mouse_state.set(MouseState::Idle);
                }
                mouse::Event::CursorMoved { position: _ } => {
                    let mut delta = plot_pos - prev_pos;
                    self.interaction
                        .mouse_state
                        .set(MouseState::Panning(plot_pos));
                    if modifiers.shift() {
                        delta.0.y = 0.0;
                    }
                    if modifiers.control() {
                        delta.0.x = 0.0;
                    }
                    return (Status::Captured, Some(ViewMsg::PanningDelta(delta).into()));
                }
                _ => {}
            },
            MouseState::DrawingRect {
                action, corner1, ..
            } => match event {
                mouse::Event::CursorMoved { .. } => {
                    self.interaction.mouse_state.set(MouseState::DrawingRect {
                        action,
                        corner1,
                        corner2: plot_pos,
                    });
                    return (Status::Captured, Some(super::Message::Refresh));
                }
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    self.interaction.mouse_state.set(MouseState::Idle);
                    if let Some(spectrogram) = &self.spectrogram {
                        let pa_to_da = PlotAreaToDataAbsolute::new(
                            &self.viewport.bounds(),
                            &spectrogram.bounds(),
                        );
                        let c1 = corner1 * pa_to_da;
                        let c2 = plot_pos * pa_to_da;
                        let rect = data_absolute::Rectangle::new(
                            data_absolute::Point::new(c1.0.x.min(c2.0.x), c1.0.y.min(c2.0.y)),
                            data_absolute::Size::new(
                                (c1.0.x - c2.0.x).abs(),
                                (c1.0.y - c2.0.y).abs(),
                            ),
                        );
                        let msg: super::Message = match action {
                            RectAction::Delete => MarksMsg::DeleteInRect(rect).into(),
                            RectAction::Zoom => ViewMsg::ZoomToRect(
                                rect * DataAbsoluteToDataNormalized::new(&spectrogram.bounds()),
                            )
                            .into(),
                            RectAction::MarkCentroid => MarksMsg::MarkCentroid(rect).into(),
                        };
                        return (Status::Captured, Some(msg));
                    }
                    return (Status::Captured, None);
                }
                _ => {}
            },
            MouseState::Marking(kind) => {
                if matches!(event, mouse::Event::ButtonReleased(mouse::Button::Left))
                    && cursor.is_over(bounds)
                {
                    let Some(spectrogram) = &self.spectrogram else {
                        return (Status::Captured, None);
                    };
                    let da_pos = plot_pos
                        * PlotAreaToDataAbsolute::new(
                            &self.viewport.bounds(),
                            &spectrogram.bounds(),
                        );
                    let msg = match kind {
                        MarkAction::Trackpoint => MarksMsg::AddTrackPoint(da_pos).into(),
                        MarkAction::Signal => MarksMsg::AddSignal(da_pos).into(),
                    };
                    return (Status::Captured, Some(msg));
                } else if matches!(event, mouse::Event::ButtonPressed(mouse::Button::Left))
                    && !cursor.is_over(bounds)
                {
                    self.interaction.mouse_state.set(MouseState::Idle);
                    return (Status::Captured, None);
                }
            }
        };

        let msg = if update_crosshair {
            Some(super::Message::Refresh)
        } else {
            None
        };
        (Status::Captured, msg)
    }

    fn handle_keyboard(
        &self,
        event: &keyboard::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<super::Message>) {
        let keyboard::Event::KeyReleased { key, .. } = event else {
            return (Status::Ignored, None);
        };
        let modifiers = self.interaction.modifiers.get();

        if matches!(self.interaction.mouse_state.get(), MouseState::Marking(_)) && !is_modifier(key)
        {
            self.interaction.mouse_state.set(MouseState::Idle);
        }

        // Some keys should work regardless of cursor position...
        let pan = if modifiers.shift() { 0.5 } else { 1.0 };
        match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                match self.interaction.mouse_state.get() {
                    MouseState::Idle => (),
                    MouseState::Panning(_) => (),
                    MouseState::DrawingRect { .. } => {
                        self.interaction.mouse_state.set(MouseState::Idle);
                        return (Status::Captured, Some(super::Message::Refresh));
                    }
                    MouseState::Marking(_) => self.interaction.mouse_state.set(MouseState::Idle),
                }
            }
            keyboard::Key::Character("s") => {
                return (Status::Captured, Some(MarksMsg::MarkTrackpoints.into()));
            }
            keyboard::Key::Character("d") if modifiers.shift() => {
                return (Status::Captured, Some(MarksMsg::MarkSignals.into()));
            }
            keyboard::Key::Character("r") => {
                return (Status::Captured, Some(ViewMsg::ResetView.into()));
            }
            keyboard::Key::Character("f") => {
                return (Status::Captured, Some(MarksMsg::FindSignals.into()));
            }
            keyboard::Key::Character("p") => {
                return (Status::Captured, Some(DisplayMsg::TogglePredictions.into()));
            }
            keyboard::Key::Named(Named::ArrowLeft) => {
                return (
                    Status::Captured,
                    Some(ViewMsg::PanningDelta(plot_area::Vector::new(pan, 0.0)).into()),
                );
            }
            keyboard::Key::Named(Named::ArrowRight) => {
                return (
                    Status::Captured,
                    Some(ViewMsg::PanningDelta(plot_area::Vector::new(-pan, 0.0)).into()),
                );
            }
            keyboard::Key::Named(Named::ArrowUp) => {
                return (
                    Status::Captured,
                    Some(ViewMsg::PanningDelta(plot_area::Vector::new(0.0, -pan)).into()),
                );
            }
            keyboard::Key::Named(Named::ArrowDown) => {
                return (
                    Status::Captured,
                    Some(ViewMsg::PanningDelta(plot_area::Vector::new(0.0, pan)).into()),
                );
            }
            _ => (),
        };

        // And some should only work when the cursor is over the actual spectrogram
        let Some(pos) = cursor
            .position_in(bounds)
            .map(|pos| screen::Point::new(pos.x, pos.y))
        else {
            return (Status::Ignored, None);
        };
        let plot_pos = pos * ScreenToPlotArea::new(&screen::Size(bounds.size()));

        match key.as_ref() {
            keyboard::Key::Character("d")
                if !modifiers.shift()
                    && matches!(self.interaction.mouse_state.get(), MouseState::Idle)
                    && self.spectrogram.is_some() =>
            {
                self.interaction.mouse_state.set(MouseState::DrawingRect {
                    action: RectAction::Delete,
                    corner1: plot_pos,
                    corner2: plot_pos,
                });
                (Status::Captured, None)
            }
            keyboard::Key::Character("z")
                if matches!(self.interaction.mouse_state.get(), MouseState::Idle)
                    && self.spectrogram.is_some() =>
            {
                self.interaction.mouse_state.set(MouseState::DrawingRect {
                    action: RectAction::Zoom,
                    corner1: plot_pos,
                    corner2: plot_pos,
                });
                (Status::Captured, None)
            }
            keyboard::Key::Character("m")
                if matches!(self.interaction.mouse_state.get(), MouseState::Idle)
                    && self.spectrogram.is_some() =>
            {
                self.interaction.mouse_state.set(MouseState::DrawingRect {
                    action: RectAction::MarkCentroid,
                    corner1: plot_pos,
                    corner2: plot_pos,
                });
                (Status::Captured, None)
            }
            _ => (Status::Ignored, None),
        }
    }

    pub fn update_marks(&mut self, message: MarksMsg, app: &AppShared) -> Task<super::Message> {
        match message {
            MarksMsg::MarkTrackpoints => {
                if matches!(self.interaction.mouse_state.get(), MouseState::Idle) {
                    self.interaction
                        .mouse_state
                        .set(MouseState::Marking(MarkAction::Trackpoint));
                }
                Task::none()
            }
            MarksMsg::MarkSignals => {
                if matches!(self.interaction.mouse_state.get(), MouseState::Idle) {
                    self.interaction
                        .mouse_state
                        .set(MouseState::Marking(MarkAction::Signal));
                }
                Task::none()
            }
            MarksMsg::AddTrackPoint(pos) => {
                log::debug!("Adding track point at position: {:?}", pos);
                self.marks.insert_track_point(pos);
                Task::none()
            }
            MarksMsg::AddSignal(pos) => {
                log::debug!("Manually adding signal at position: {:?}", pos);
                self.marks.signals_mut().push(pos);
                Task::none()
            }
            MarksMsg::DeleteMark(action, point) => {
                log::debug!("Deleting {:?} mark at position: {:?}", action, point);
                self.marks.remove(action, point);
                Task::none()
            }
            MarksMsg::DeleteInRect(rect) => {
                self.marks.retain(|p| !rect.contains(*p));
                Task::none()
            }
            MarksMsg::MarkCentroid(rect) => {
                let centroid = self
                    .spectrogram
                    .as_ref()
                    .and_then(|spec| signal::centroid(spec, rect));
                self.marks.signals_mut().extend(centroid);
                Task::none()
            }
            MarksMsg::ClearAll => {
                self.marks.clear();
                Task::none()
            }
            MarksMsg::FindSignals => {
                if self.marks.track_points().len() < 2 {
                    Task::none()
                } else {
                    let Some(spectrogram) = &self.spectrogram else {
                        log::error!("No spectrogram loaded, cannot find signals");
                        return Task::none();
                    };
                    let spectrogram = spectrogram.clone();
                    let track_points = self.marks.track_points().to_vec();
                    let sigma = self.detection.signal_sigma;
                    let track_bw = self.detection.track_bw;
                    Task::future(async move {
                        tokio::task::spawn_blocking(move || {
                            let signals = signal::find_signals(
                                &spectrogram,
                                &track_points,
                                track_bw,
                                signal::SignalDetectionMethod::FitTrace { sigma },
                            );
                            let signals = match signals {
                                Err(e) => {
                                    log::error!("Error finding signals: {}", e);
                                    Vec::new()
                                }
                                Ok(signals) => {
                                    log::info!("Found {} signal peaks", signals.len());
                                    signals
                                }
                            };
                            MarksMsg::FoundSignals(signals).into()
                        })
                        .await
                        .unwrap()
                    })
                }
            }
            MarksMsg::FoundSignals(signals) => {
                *self.marks.signals_mut() = signals;
                Task::none()
            }
            MarksMsg::SaveSignals => {
                let Some(spectrogram) = &self.spectrogram else {
                    log::error!("No spectrogram loaded, cannot save signals");
                    return Task::none();
                };
                let Some(site_id) = app.site_id else {
                    log::error!("No site configured, cannot save signals");
                    return Task::none();
                };
                let start_time = spectrogram.start_time();
                let start_mjd = start_time.timestamp_millis() as f64 / 86_400_000.0 + 40587.0;
                let center_freq = spectrogram.freq as f64;
                let suggested = signals_filename(start_time, center_freq, self.marks.signals())
                    .unwrap_or_else(|| "out.dat".to_owned());
                let mut output = String::new();
                for sig in self.marks.signals() {
                    let mjd = start_mjd + sig.0.x as f64 / 86400.0;
                    let freq = center_freq + sig.0.y as f64;
                    output.push_str(&format!("{mjd:.6} {freq:.6} 5.000000 {site_id}\n"));
                }
                Task::future(async move {
                    let path = AsyncFileDialog::new()
                        .set_file_name(suggested.as_str())
                        .save_file()
                        .await
                        .map(|f| f.path().to_path_buf());
                    MarksMsg::WriteSignals(output, path).into()
                })
            }
            MarksMsg::WriteSignals(_, None) => Task::none(),
            MarksMsg::WriteSignals(output, Some(path)) => {
                let n = output.lines().count();
                match std::fs::write(&path, &output) {
                    Ok(()) => log::info!("Wrote {n} signals to {path:?}"),
                    Err(e) => log::error!("Failed to write {path:?}: {e}"),
                }
                Task::none()
            }
            MarksMsg::SpectrogramUpdated => {
                self.marks.clear();
                self.interaction.crosshair.set(None);
                self.check_cache(app)
            }
            MarksMsg::UpdateSignalSigma(sigma) => {
                self.detection.signal_sigma = sigma;
                Task::none()
            }
            MarksMsg::UpdateTrackBW(bw) => {
                self.detection.track_bw = bw;
                Task::none()
            }
        }
    }

    pub fn update_predictions(
        &mut self,
        message: PredictionsMsg,
        app: &AppShared,
    ) -> Task<super::Message> {
        match message {
            PredictionsMsg::RefreshCache => {
                self.prediction_cache.reset();
                self.check_cache(app)
            }
            PredictionsMsg::PredictionsReady(key, predictions) => {
                log::debug!("Using {} satellite predictions", predictions.n_satellites());
                self.prediction_cache.store(key, predictions);
                Task::none()
            }
            PredictionsMsg::PredictionFailed => {
                log::error!("Prediction failed");
                Task::none()
            }
        }
    }
}

/// Finds the mark (track point or signal) nearest to `pos`, measured in screen pixels via
/// `da_to_screen`, and returns it tagged with which collection it belongs to. Returns `None` if
/// there are no marks, or the nearest is farther than [`DELETE_TOLERANCE_PX`].
fn closest_mark(
    pos: screen::Point,
    da_to_screen: &DataAbsoluteToScreen,
    track_points: &[data_absolute::Point],
    signals: &[data_absolute::Point],
) -> Option<(MarkAction, data_absolute::Point)> {
    track_points
        .iter()
        .map(|p| (MarkAction::Trackpoint, p))
        .chain(signals.iter().map(|p| (MarkAction::Signal, p)))
        .map(|(action, &point)| {
            let offset = point * *da_to_screen - pos;
            let dist = offset.0.x.hypot(offset.0.y);
            (action, point, dist)
        })
        .filter(|(_, _, dist)| *dist <= DELETE_TOLERANCE_PX)
        .min_by(|(_, _, a), (_, _, b)| a.total_cmp(b))
        .map(|(action, point, _)| (action, point))
}

impl Chart<super::Message> for PlotChart<'_> {
    type State = ();

    fn build_chart<DB: DrawingBackend>(&self, _state: &Self::State, chart: ChartBuilder<DB>) {
        match self.state.build_chart(chart, self.app) {
            Ok(()) => (),
            Err(e) => log::error!("Error building chart: {:?}", e),
        }
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<super::Message>) {
        let bounds = Rectangle {
            x: bounds.x + self.state.plot_area_margin,
            y: bounds.y,
            width: bounds.width - self.state.plot_area_margin,
            height: bounds.height - self.state.plot_area_margin,
        };
        match event {
            canvas::Event::Mouse(event) => self.state.handle_mouse(event, bounds, cursor),
            canvas::Event::Keyboard(event) => {
                if let keyboard::Event::ModifiersChanged(modifiers) = event {
                    self.state.interaction.modifiers.set(*modifiers);
                    return (Status::Ignored, None);
                }
                self.state.handle_keyboard(event, bounds, cursor)
            }
            _ => {
                log::debug!("{:?}", event);
                (Status::Ignored, None)
            }
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            match self.state.interaction.mouse_state.get() {
                MouseState::Idle => mouse::Interaction::Idle,
                MouseState::Panning(_) => mouse::Interaction::Grabbing,
                MouseState::DrawingRect { .. } | MouseState::Marking(_) => {
                    mouse::Interaction::Crosshair
                }
            }
        } else {
            mouse::Interaction::Idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstrf::coord::{data_absolute, data_normalized};

    fn pt(x: f32, y: f32) -> data_absolute::Point {
        data_absolute::Point::new(x, y)
    }

    /// A `DataAbsoluteToScreen` transform that maps data-absolute coordinates 1:1 onto screen
    /// pixels, so marks in these tests can be positioned directly in pixel space.
    fn identity_da_to_screen() -> DataAbsoluteToScreen {
        DataAbsoluteToScreen::new(
            &screen::Size::new(100.0, 100.0),
            &data_normalized::Rectangle::new(
                data_normalized::Point::new(0.0, 0.0),
                data_normalized::Size::new(1.0, 1.0),
            ),
            // y is flipped (screen y grows downward) so an identity x/y mapping needs a
            // negative-height bounds anchored at the top.
            &data_absolute::Rectangle::new(pt(0.0, 100.0), data_absolute::Size::new(100.0, -100.0)),
        )
    }

    fn sp(x: f32, y: f32) -> screen::Point {
        screen::Point::new(x, y)
    }

    #[test]
    fn identity_transform_maps_data_to_pixels() {
        // Sanity check that the test fixture really is an identity x/y mapping.
        let screen = pt(30.0, 40.0) * identity_da_to_screen();
        assert!((screen.0.x - 30.0).abs() < 1e-3, "x = {}", screen.0.x);
        assert!((screen.0.y - 40.0).abs() < 1e-3, "y = {}", screen.0.y);
    }

    #[test]
    fn closest_mark_none_when_no_marks() {
        assert_eq!(
            closest_mark(sp(50.0, 50.0), &identity_da_to_screen(), &[], &[]),
            None
        );
    }

    #[test]
    fn closest_mark_returns_nearest_track_point() {
        let track_points = [pt(10.0, 10.0), pt(50.0, 50.0)];
        // Cursor 5 px from the second point, far from the first.
        assert_eq!(
            closest_mark(sp(53.0, 54.0), &identity_da_to_screen(), &track_points, &[]),
            Some((MarkAction::Trackpoint, pt(50.0, 50.0)))
        );
    }

    #[test]
    fn closest_mark_picks_nearest_across_collections() {
        let track_points = [pt(10.0, 10.0)];
        let signals = [pt(12.0, 12.0)];
        // Cursor nearer the signal than the track point.
        assert_eq!(
            closest_mark(
                sp(13.0, 13.0),
                &identity_da_to_screen(),
                &track_points,
                &signals
            ),
            Some((MarkAction::Signal, pt(12.0, 12.0)))
        );
    }

    #[test]
    fn closest_mark_none_when_all_outside_tolerance() {
        let track_points = [pt(0.0, 0.0)];
        // ~70 px away, well beyond DELETE_TOLERANCE_PX.
        assert_eq!(
            closest_mark(sp(50.0, 50.0), &identity_da_to_screen(), &track_points, &[]),
            None
        );
    }

    #[test]
    fn closest_mark_tolerance_is_inclusive() {
        let signals = [pt(DELETE_TOLERANCE_PX, 0.0)];
        // Exactly DELETE_TOLERANCE_PX away → still deleted.
        assert_eq!(
            closest_mark(sp(0.0, 0.0), &identity_da_to_screen(), &[], &signals),
            Some((MarkAction::Signal, pt(DELETE_TOLERANCE_PX, 0.0)))
        );
    }

    #[test]
    fn prediction_color_maps_classification_to_expected_rgb() {
        assert_eq!(prediction_color(None), GREEN);
        assert_eq!(
            prediction_color(Some(sgp4::Classification::Unclassified)),
            GREEN
        );
        assert_eq!(
            prediction_color(Some(sgp4::Classification::Classified)),
            ORANGE
        );
        assert_eq!(prediction_color(Some(sgp4::Classification::Secret)), RED);
    }
}
