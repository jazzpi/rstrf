//! This module contains the plot overlay for RFPlot. It draws everything that isn't the spectrogram
//! itself (like axes and overlays). It is also responsible for the user interaction with the plot
//! (like panning/zooming).

use std::cell::Cell;

use chrono::{DateTime, Duration, Utc};
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
use plotters::prelude::*;
use plotters_iced2::Chart;
use rstrf::{
    coord::{
        DataAbsoluteToDataNormalized, DataNormalizedToDataAbsolute, PlotAreaToDataAbsolute,
        ScreenToPlotArea, data_absolute, plot_area, screen,
    },
    orbit::{self, Site},
    signal,
    util::clip_line,
};
use serde::{Deserialize, Serialize};

use crate::{app::AppShared, windows::rfplot::MarkAction};
use rstrf::async_cache::AsyncCache;

use super::{MouseState, RFPlot, RectAction, SharedState, control};

/// All inputs that determine the satellite pass predictions.
///
/// To avoid having to explicitly keep track of when the predictions are stale, we use this as the
/// key for an `AsyncCache`, and check the cached predictions against the current key on every
/// `update()` call.
///
/// This involves creating a copy of the key & comparing it, so we don't want the key to be too big.
/// Thus, we don't include the full `Satellite` structs and instead just include the satellite IDs.
/// That breaks the automatic staleness detection if the satellites are changed (e.g. new TLEs
/// loaded or transmitters modified), and these cases need to be handled manually (via
/// `Message::RefreshCache`). This is a bit annoying, but keeping the full satellite data in the key
/// comes with a severe performance penalty for large catalogs.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PredictionKey {
    satellites: Vec<u64>,
    start_time: DateTime<Utc>,
    length: Duration,
    site: Site,
}

fn prediction_key(shared: &SharedState, app: &AppShared) -> Option<PredictionKey> {
    let spectrogram = shared.spectrogram.as_ref()?;
    let site = app.config.site.as_ref()?.clone();
    let satellites = app.active_satellite_ids();
    if satellites.is_empty() {
        return None;
    }
    Some(PredictionKey {
        satellites,
        start_time: spectrogram.start_time(),
        length: spectrogram.length(),
        site,
    })
}

#[derive(Debug, Clone)]
pub enum Message {
    MarkTrackpoints,
    MarkSignals,
    AddTrackPoint(data_absolute::Point),
    AddSignal(data_absolute::Point),
    ClearAll,
    FindSignals,
    FoundSignals(Vec<data_absolute::Point>),
    UpdateCrosshair(Option<plot_area::Point>),
    SpectrogramUpdated,
    /// Force a prediction cache check without any other side effects.
    RefreshCache,
    PredictionsReady(PredictionKey, orbit::Predictions),
    PredictionFailed,
    TogglePredictions,
    ToggleGrid,
    ToggleCrosshair,
    ToggleAbsoluteAxes,
    DeleteInRect(data_absolute::Rectangle),
    UpdateRectPreview(Option<plot_area::Point>),
    SaveSignals,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Overlay {
    #[serde(skip)]
    prediction_cache: AsyncCache<PredictionKey, orbit::Predictions>,
    show_predictions: bool,
    show_grid: bool,
    show_crosshair: bool,
    absolute_axes: bool,
    track_points: Vec<data_absolute::Point>,
    signals: Vec<data_absolute::Point>,
    #[serde(skip)]
    crosshair: Option<data_absolute::Point>,
    #[serde(skip)]
    rect_preview: Option<plot_area::Point>,
    #[serde(skip)]
    mouse_state: Cell<MouseState>,
    #[serde(skip)]
    modifiers: Cell<keyboard::Modifiers>,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            prediction_cache: AsyncCache::default(),
            show_predictions: true,
            show_grid: Default::default(),
            show_crosshair: Default::default(),
            absolute_axes: true,
            track_points: Default::default(),
            signals: Default::default(),
            crosshair: Default::default(),
            rect_preview: Default::default(),
            mouse_state: Cell::new(MouseState::Idle),
            modifiers: Cell::new(keyboard::Modifiers::default()),
        }
    }
}

impl Overlay {
    fn build_chart<DB: DrawingBackend>(
        &self,
        mut chart: ChartBuilder<DB>,
        shared: &SharedState,
    ) -> Result<(), String> {
        let Some(spectrogram) = &shared.spectrogram else {
            return Err("No spectrogram loaded".to_string());
        };
        let bounds =
            shared.controls.bounds() * DataNormalizedToDataAbsolute::new(&spectrogram.bounds());
        let x = CopyRange::from_std(bounds.0.x..(bounds.0.x + bounds.0.width));
        let y = CopyRange::from_std(bounds.0.y..(bounds.0.y + bounds.0.height));
        let mut chart = chart
            .x_label_area_size(shared.plot_area_margin)
            .y_label_area_size(shared.plot_area_margin)
            .build_cartesian_2d(x.into_std(), y.into_std())
            .map_err(|e| format!("Failed to build chart: {:?}", e))?;

        let mut mesh = chart.configure_mesh();
        let mut frame = mesh
            .max_light_lines(0)
            .axis_style(WHITE)
            .label_style(&WHITE)
            .bold_line_style(WHITE.mix(0.4));

        let plot_center_freq = bounds.0.y + bounds.0.height / 2.0;
        let start_time = spectrogram.start_time();
        let x_formatter = |v: &f32| {
            let t = start_time + Duration::seconds(*v as i64);
            format!("{}", t.format("%H:%M"))
        };
        let y_formatter = |v: &f32| format!("{:.1}", (v - plot_center_freq) / 1000.0);
        if self.absolute_axes {
            frame = frame
                .x_label_formatter(&x_formatter)
                .y_label_formatter(&y_formatter)
                .x_desc(format!("Time - {} [HH:MM]", start_time.format("%Y-%m-%d")))
                .y_desc(format!(
                    "Frequency - {:.1} [kHz]",
                    (spectrogram.freq + plot_center_freq) / 1000.0
                ));
        } else {
            frame = frame
                .y_label_formatter(&|v| format!("{:.1}", v / 1000.0))
                .x_desc("Time [s]")
                .y_desc("Frequency offset [kHz]");
        }
        if !self.show_grid {
            frame = frame.disable_mesh();
        }

        frame
            .draw()
            .map_err(|e| format!("Failed to draw mesh: {:?}", e))?;

        if self.show_predictions
            && let Some((_, predictions)) = self.prediction_cache.get_stored()
        {
            let time = &predictions.times;
            for prediction in predictions.iter_satellites() {
                let (id, passes) = prediction;
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
                                &GREEN,
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
                                ("sans-serif", 12).into_font().color(&GREEN),
                            )])
                            .map_err(|e| {
                                format!("Could not draw label for satellite {}: {:?}", id, e)
                            })?;
                    }
                }
            }
        }

        chart
            .draw_series(self.track_points.iter().filter_map(|pos| {
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
                    self.track_points.iter().map(|pos| {
                        data_absolute::Point::new(
                            pos.0.x,
                            pos.0.y + shared.controls.track_bw() / 2.0,
                        )
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
                    self.track_points.iter().map(|pos| {
                        data_absolute::Point::new(
                            pos.0.x,
                            pos.0.y - shared.controls.track_bw() / 2.0,
                        )
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
            .draw_series(self.signals.iter().filter_map(|pos| {
                if bounds.contains(*pos) {
                    Some(Circle::new(pos.into(), 5, WHITE.filled()))
                } else {
                    None
                }
            }))
            .map_err(|e| format!("Could not draw track points: {:?}", e))?;
        if self.show_crosshair
            && let Some(crosshair) = &self.crosshair
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
                * PlotAreaToDataAbsolute::new(&shared.controls.bounds(), &spectrogram.bounds());
            chart
                .draw_series(vec![Text::new(
                    format!(
                        "t = {:.01} s\nf = {:.01} kHz\nP = {:.01} dB",
                        crosshair.0.x,
                        crosshair.0.y / 1e3,
                        power
                    ),
                    crosshair_pos.into(),
                    ("sans-serif", 12).into_font().color(&WHITE),
                )])
                .expect("Could not draw crosshair label");
        }

        if let MouseState::DrawingRect {
            action,
            corner1,
            corner2,
        } = self.mouse_state.get()
        {
            let pa_to_da =
                PlotAreaToDataAbsolute::new(&shared.controls.bounds(), &spectrogram.bounds());
            let c1: (f32, f32) = (corner1 * pa_to_da).into();
            let c2: (f32, f32) = (corner2 * pa_to_da).into();
            let (fill_color, border_color) = match action {
                RectAction::Delete => (RED.mix(0.25), RED.mix(1.0)),
                RectAction::Zoom => (CYAN.mix(0.15), CYAN.mix(1.0)),
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
        shared: &SharedState,
    ) -> (Status, Option<super::Message>) {
        use control::Message as CMessage;
        let Some(cursor_pos) = cursor.position() else {
            return (Status::Ignored, None);
        };
        let pos = screen::Point::new(cursor_pos.x - bounds.x, cursor_pos.y - bounds.y);
        let plot_pos = pos * ScreenToPlotArea::new(&screen::Size(bounds.size()));
        let modifiers = self.modifiers.get();
        if let mouse::Event::WheelScrolled { delta } = event {
            let delta = match delta {
                mouse::ScrollDelta::Lines { x: _, y } => y,
                mouse::ScrollDelta::Pixels { x: _, y } => y,
            };
            let x_axis = Rectangle {
                x: bounds.x,
                y: bounds.y + bounds.height,
                width: bounds.width,
                height: shared.plot_area_margin,
            };
            let y_axis = Rectangle {
                x: bounds.x - shared.plot_area_margin,
                y: bounds.y,
                width: shared.plot_area_margin,
                height: bounds.height,
            };
            if cursor.is_over(bounds) {
                if modifiers.shift() {
                    return (
                        Status::Captured,
                        Some(CMessage::ZoomDeltaX(plot_pos, *delta).into()),
                    );
                } else if modifiers.control() {
                    return (
                        Status::Captured,
                        Some(CMessage::ZoomDeltaY(plot_pos, *delta).into()),
                    );
                }
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDelta(plot_pos, *delta).into()),
                );
            } else if cursor.is_over(y_axis) {
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDeltaY(plot_pos, *delta).into()),
                );
            } else if cursor.is_over(x_axis) {
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDeltaX(plot_pos, *delta).into()),
                );
            }
        }

        match self.mouse_state.get() {
            MouseState::Idle => match event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if cursor.is_over(bounds) {
                        self.mouse_state.set(MouseState::Panning(plot_pos));
                        return (Status::Captured, None);
                    }
                }
                mouse::Event::CursorMoved { position: _ } => {
                    if cursor.is_over(bounds) {
                        return (
                            Status::Captured,
                            Some(Message::UpdateCrosshair(Some(plot_pos)).into()),
                        );
                    } else {
                        return (
                            Status::Captured,
                            Some(Message::UpdateCrosshair(None).into()),
                        );
                    }
                }
                _ => {}
            },
            MouseState::Panning(prev_pos) => match event {
                mouse::Event::ButtonReleased(mouse::Button::Left) => {
                    self.mouse_state.set(MouseState::Idle);
                }
                mouse::Event::CursorMoved { position: _ } => {
                    let delta = plot_pos - prev_pos;
                    self.mouse_state.set(MouseState::Panning(plot_pos));
                    return (Status::Captured, Some(CMessage::PanningDelta(delta).into()));
                }
                _ => {}
            },
            MouseState::DrawingRect {
                action, corner1, ..
            } => match event {
                mouse::Event::CursorMoved { .. } => {
                    self.mouse_state.set(MouseState::DrawingRect {
                        action,
                        corner1,
                        corner2: plot_pos,
                    });
                    return (
                        Status::Captured,
                        Some(Message::UpdateRectPreview(Some(plot_pos)).into()),
                    );
                }
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    self.mouse_state.set(MouseState::Idle);
                    if let Some(spectrogram) = &shared.spectrogram {
                        let pa_to_da = PlotAreaToDataAbsolute::new(
                            &shared.controls.bounds(),
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
                            RectAction::Delete => Message::DeleteInRect(rect).into(),
                            RectAction::Zoom => control::Message::ZoomToRect(
                                rect * DataAbsoluteToDataNormalized::new(&spectrogram.bounds()),
                            )
                            .into(),
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
                    let Some(spectrogram) = &shared.spectrogram else {
                        return (Status::Captured, None);
                    };
                    let da_pos = plot_pos
                        * PlotAreaToDataAbsolute::new(
                            &shared.controls.bounds(),
                            &spectrogram.bounds(),
                        );
                    let msg = match kind {
                        MarkAction::Trackpoint => Message::AddTrackPoint(da_pos).into(),
                        MarkAction::Signal => Message::AddSignal(da_pos).into(),
                    };
                    return (Status::Captured, Some(msg));
                }
            }
        };

        (Status::Captured, None)
    }

    fn handle_keyboard(
        &self,
        event: &keyboard::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shared: &SharedState,
    ) -> (Status, Option<super::Message>) {
        let keyboard::Event::KeyReleased { key, .. } = event else {
            return (Status::Ignored, None);
        };
        let modifiers = self.modifiers.get();

        // Some keys should work regardless of cursor position...
        let pan = if modifiers.shift() { 0.5 } else { 1.0 };
        match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::Escape) => match self.mouse_state.get() {
                MouseState::Idle => (),
                MouseState::Panning(_) => (),
                MouseState::DrawingRect { .. } => {
                    self.mouse_state.set(MouseState::Idle);
                    return (
                        Status::Captured,
                        Some(Message::UpdateRectPreview(None).into()),
                    );
                }
                MouseState::Marking(_) => self.mouse_state.set(MouseState::Idle),
            },
            keyboard::Key::Character("s") => {
                return (Status::Captured, Some(Message::MarkTrackpoints.into()));
            }
            keyboard::Key::Character("d") if modifiers.shift() => {
                return (Status::Captured, Some(Message::MarkSignals.into()));
            }
            keyboard::Key::Character("r") => {
                return (Status::Captured, Some(control::Message::ResetView.into()));
            }
            keyboard::Key::Character("f") => {
                return (Status::Captured, Some(Message::FindSignals.into()));
            }
            keyboard::Key::Character("p") => {
                return (Status::Captured, Some(Message::TogglePredictions.into()));
            }
            keyboard::Key::Named(Named::ArrowLeft) => {
                return (
                    Status::Captured,
                    Some(control::Message::PanningDelta(plot_area::Vector::new(pan, 0.0)).into()),
                );
            }
            keyboard::Key::Named(Named::ArrowRight) => {
                return (
                    Status::Captured,
                    Some(control::Message::PanningDelta(plot_area::Vector::new(-pan, 0.0)).into()),
                );
            }
            keyboard::Key::Named(Named::ArrowUp) => {
                return (
                    Status::Captured,
                    Some(control::Message::PanningDelta(plot_area::Vector::new(0.0, -pan)).into()),
                );
            }
            keyboard::Key::Named(Named::ArrowDown) => {
                return (
                    Status::Captured,
                    Some(control::Message::PanningDelta(plot_area::Vector::new(0.0, pan)).into()),
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
                    && matches!(self.mouse_state.get(), MouseState::Idle)
                    && shared.spectrogram.is_some() =>
            {
                self.mouse_state.set(MouseState::DrawingRect {
                    action: RectAction::Delete,
                    corner1: plot_pos,
                    corner2: plot_pos,
                });
                (Status::Captured, None)
            }
            keyboard::Key::Character("z")
                if matches!(self.mouse_state.get(), MouseState::Idle)
                    && shared.spectrogram.is_some() =>
            {
                self.mouse_state.set(MouseState::DrawingRect {
                    action: RectAction::Zoom,
                    corner1: plot_pos,
                    corner2: plot_pos,
                });
                (Status::Captured, None)
            }
            _ => (Status::Ignored, None),
        }
    }

    /// Checks whether the prediction cache is stale for the current inputs. If so, starts an async
    /// recomputation. Called at the top of every `update()` so any incoming message acts as a
    /// trigger.
    fn check_cache(&mut self, shared: &SharedState, app: &AppShared) -> Task<Message> {
        let Some(key) = prediction_key(shared, app) else {
            self.prediction_cache.reset();
            return Task::none();
        };
        self.prediction_cache.request(key, |key| {
            let length_s = key.length.num_milliseconds() as f64 / 1000.0;
            let satellites = app.active_satellites();
            Task::future(async move {
                let key_for_msg = key.clone();
                let result = tokio::task::spawn_blocking(move || {
                    orbit::predict_satellites(&satellites, key.start_time, length_s, &key.site)
                })
                .await;
                match result {
                    Ok(predictions) => Message::PredictionsReady(key_for_msg, predictions),
                    Err(e) => {
                        log::error!("Failed to predict satellite passes: {}", e);
                        Message::PredictionFailed
                    }
                }
            })
        })
    }

    pub fn update(
        &mut self,
        message: Message,
        shared: &SharedState,
        app: &AppShared,
    ) -> Task<Message> {
        let msg_task = match message {
            Message::MarkTrackpoints => {
                if matches!(self.mouse_state.get(), MouseState::Idle) {
                    self.mouse_state
                        .set(MouseState::Marking(MarkAction::Trackpoint));
                }
                Task::none()
            }
            Message::MarkSignals => {
                if matches!(self.mouse_state.get(), MouseState::Idle) {
                    self.mouse_state
                        .set(MouseState::Marking(MarkAction::Signal));
                }
                Task::none()
            }
            Message::AddTrackPoint(pos) => {
                log::debug!("Adding track point at position: {:?}", pos);
                match self
                    .track_points
                    .binary_search_by(|p| p.0.x.partial_cmp(&pos.0.x).unwrap())
                {
                    Ok(idx) => self.track_points[idx] = pos,
                    Err(idx) => self.track_points.insert(idx, pos),
                }
                Task::none()
            }
            Message::AddSignal(pos) => {
                log::debug!("Manually adding signal at position: {:?}", pos);
                self.signals.push(pos);
                Task::none()
            }
            Message::ClearAll => {
                self.track_points.clear();
                self.signals.clear();
                Task::none()
            }
            Message::FindSignals => {
                if self.track_points.len() < 2 {
                    Task::none()
                } else {
                    let Some(spectrogram) = &shared.spectrogram else {
                        log::error!("No spectrogram loaded, cannot find signals");
                        return Task::none();
                    };
                    let spectrogram = spectrogram.clone();
                    let track_points = self.track_points.clone();
                    let sigma = shared.controls.signal_sigma();
                    let track_bw = shared.controls.track_bw();
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
                            Message::FoundSignals(signals)
                        })
                        .await
                        .unwrap()
                    })
                }
            }
            Message::FoundSignals(signals) => {
                self.signals = signals;
                Task::none()
            }
            Message::UpdateCrosshair(plot_pos) => {
                self.crosshair = shared.spectrogram.as_ref().and_then(|spectrogram| {
                    plot_pos.map(|p| {
                        p * PlotAreaToDataAbsolute::new(
                            &shared.controls.bounds(),
                            &spectrogram.bounds(),
                        )
                    })
                });
                Task::none()
            }
            Message::SpectrogramUpdated => {
                self.track_points.clear();
                self.signals.clear();
                self.crosshair = None;
                Task::none()
            }
            Message::RefreshCache => {
                self.prediction_cache.reset();
                Task::none()
            }
            Message::PredictionsReady(key, predictions) => {
                log::debug!("Using {} satellite predictions", predictions.n_satellites());
                self.prediction_cache.store(key, predictions);
                Task::none()
            }
            Message::PredictionFailed => {
                log::error!("Prediction failed");
                Task::none()
            }
            Message::TogglePredictions => {
                self.show_predictions = !self.show_predictions;
                Task::none()
            }
            Message::ToggleGrid => {
                self.show_grid = !self.show_grid;
                Task::none()
            }
            Message::ToggleCrosshair => {
                self.show_crosshair = !self.show_crosshair;
                Task::none()
            }
            Message::ToggleAbsoluteAxes => {
                self.absolute_axes = !self.absolute_axes;
                Task::none()
            }
            Message::DeleteInRect(rect) => {
                self.rect_preview = None;
                self.track_points.retain(|p| !rect.contains(*p));
                self.signals.retain(|p| !rect.contains(*p));
                Task::none()
            }
            Message::UpdateRectPreview(corner2) => {
                self.rect_preview = corner2;
                Task::none()
            }
            Message::SaveSignals => {
                let Some(spectrogram) = &shared.spectrogram else {
                    log::warn!("No spectrogram loaded, cannot save signals");
                    return Task::none();
                };
                let start_mjd =
                    spectrogram.start_time().timestamp_millis() as f64 / 86_400_000.0 + 40587.0;
                let center_freq = spectrogram.freq as f64;
                let site_id = app.site_id;
                let mut output = String::new();
                for sig in &self.signals {
                    let mjd = start_mjd + sig.0.x as f64 / 86400.0;
                    let freq = center_freq + sig.0.y as f64;
                    output.push_str(&format!("{mjd:.6} {freq:.6} 5.000000 {site_id}\n"));
                }
                match std::fs::write("out.dat", &output) {
                    Ok(()) => log::info!("Wrote {} signals to out.dat", self.signals.len()),
                    Err(e) => log::error!("Failed to write out.dat: {e}"),
                }
                Task::none()
            }
        };

        let cache_task = self.check_cache(shared, app);
        Task::batch([cache_task, msg_task])
    }
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Self) -> bool {
        self.track_points == other.track_points
            && self.signals == other.signals
            && self.crosshair == other.crosshair
            && self.rect_preview == other.rect_preview
            && self.absolute_axes == other.absolute_axes
    }
}

impl Chart<super::Message> for RFPlot {
    type State = ();

    fn build_chart<DB: DrawingBackend>(&self, _state: &Self::State, chart: ChartBuilder<DB>) {
        match self.overlay.build_chart(chart, &self.shared) {
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
            x: bounds.x + self.shared.plot_area_margin,
            y: bounds.y,
            width: bounds.width - self.shared.plot_area_margin,
            height: bounds.height - self.shared.plot_area_margin,
        };
        match event {
            canvas::Event::Mouse(event) => {
                self.overlay
                    .handle_mouse(event, bounds, cursor, &self.shared)
            }
            canvas::Event::Keyboard(event) => {
                if let keyboard::Event::ModifiersChanged(modifiers) = event {
                    self.overlay.modifiers.set(*modifiers);
                    return (Status::Ignored, None);
                }
                self.overlay
                    .handle_keyboard(event, bounds, cursor, &self.shared)
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
            match self.overlay.mouse_state.get() {
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
