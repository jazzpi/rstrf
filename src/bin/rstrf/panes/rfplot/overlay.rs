//! This module contains the plot overlay for RFPlot. It draws everything that isn't the spectrogram
//! itself (like axes and overlays). It is also responsible for the user interaction with the plot
//! (like panning/zooming).

use copy_range::CopyRange;
use iced::{Rectangle, Task, event::Status, keyboard, mouse, widget::canvas};
use itertools::{Itertools, izip};
use plotters::prelude::*;
use plotters_iced2::Chart;
use rstrf::{
    coord::{
        DataAbsoluteToDataNormalized, DataNormalizedToDataAbsolute, PlotAreaToDataAbsolute,
        ScreenToDataAbsolute, ScreenToPlotArea, data_absolute, plot_area, screen,
    },
    orbit::{self, SatPrediction, Site},
    signal,
    spectrogram::Spectrogram,
    util::clip_line,
};
use serde::{Deserialize, Serialize};

use crate::{app::AppShared, workspace::WorkspaceShared};

use super::{MouseInteraction, RFPlot, SharedState, control};

#[derive(Debug, Clone)]
pub enum Message {
    AddTrackPoint(data_absolute::Point),
    FindSignals,
    FoundSignals(Vec<data_absolute::Point>),
    UpdateCrosshair(Option<plot_area::Point>),
    UpdatePredictions,
    SetSatellitePredictions(Option<orbit::Predictions>),
    SpectrogramUpdated,
    TogglePredictions,
    ToggleGrid,
    ToggleCrosshair,
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

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Overlay {
    satellites: Vec<orbit::Satellite>,
    #[serde(skip)]
    satellite_predictions: Option<orbit::Predictions>,
    #[serde(default = "yes")]
    show_predictions: bool,
    #[serde(default)]
    show_grid: bool,
    #[serde(default)]
    show_crosshair: bool,
    track_points: Vec<data_absolute::Point>,
    signals: Vec<data_absolute::Point>,
    #[serde(skip)]
    crosshair: Option<data_absolute::Point>,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            satellites: Default::default(),
            satellite_predictions: Default::default(),
            show_predictions: true,
            show_grid: Default::default(),
            track_points: Default::default(),
            signals: Default::default(),
            show_crosshair: Default::default(),
            crosshair: Default::default(),
        }
    }
}

impl Overlay {
    fn build_chart<DB: DrawingBackend>(
        &self,
        _state: &MouseInteraction,
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
            .bold_line_style(WHITE.mix(0.4))
            .y_label_formatter(&|v| format!("{:.1}", v / 1000.0))
            .x_desc("Time [s]")
            .y_desc("Frequency offset [kHz]");
        if !self.show_grid {
            frame = frame.disable_mesh();
        }

        frame
            .draw()
            .map_err(|e| format!("Failed to draw mesh: {:?}", e))?;

        if self.show_predictions
            && let Some(satellite_predictions) = &self.satellite_predictions
        {
            let time = &satellite_predictions.times;
            for sat in &self.satellites {
                let id = sat.norad_id();
                log::trace!("Plotting satellite {}", id);
                let Some(SatPrediction {
                    frequency: freq,
                    zenith_angle: za,
                }) = &satellite_predictions.for_id(id)
                else {
                    continue;
                };

                chart
                    .draw_series(LineSeries::new(
                        izip!(time.iter(), freq.iter(), za.iter()).filter_map(|(&t, &f, &za)| {
                            if za < std::f64::consts::FRAC_PI_2 {
                                Some((t as f32, (f as f32 - spectrogram.freq)))
                            } else {
                                None
                            }
                        }),
                        &GREEN,
                    ))
                    .map_err(|e| format!("Could not draw line for satellite {}: {:?}", id, e))?
                    .label(format!("{:06}", id));

                let first_visible =
                    izip!(time.iter(), freq.iter(), za.iter()).position(|(&t, &f, &za)| {
                        x.contains(&(t as f32))
                            && y.contains(&(f as f32 - spectrogram.freq))
                            && za < std::f64::consts::FRAC_PI_2
                    });
                let Some(first_visible) = first_visible else {
                    continue;
                };
                let first_time = (time[first_visible] as f32).max(x.start);
                let first_freq = freq[first_visible] as f32 - spectrogram.freq;
                chart
                    .draw_series(vec![Text::new(
                        format!("{:06}", id),
                        (first_time, first_freq),
                        ("sans-serif", 12).into_font().color(&GREEN),
                    )])
                    .map_err(|e| format!("Could not draw label for satellite {}: {:?}", id, e))?;
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

        Ok(())
    }

    fn handle_mouse(
        &self,
        state: &mut MouseInteraction,
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
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDelta(plot_pos, *delta).into()),
                );
            } else if cursor.is_over(y_axis) {
                // Zooming over y axis
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDeltaY(plot_pos, *delta).into()),
                );
            } else if cursor.is_over(x_axis) {
                // Zooming over x axis
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDeltaX(plot_pos, *delta).into()),
                );
            }
        }

        match state {
            MouseInteraction::Idle => match event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if cursor.is_over(bounds) {
                        *state = MouseInteraction::Panning(plot_pos);
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
            MouseInteraction::Panning(prev_pos) => match event {
                mouse::Event::ButtonReleased(mouse::Button::Left) => {
                    *state = MouseInteraction::Idle;
                }
                mouse::Event::CursorMoved { position: _ } => {
                    let delta = plot_pos - *prev_pos;
                    *state = MouseInteraction::Panning(plot_pos);
                    return (Status::Captured, Some(CMessage::PanningDelta(delta).into()));
                }
                _ => {}
            },
        };

        (Status::Captured, None)
    }

    fn handle_keyboard(
        &self,
        _state: &mut MouseInteraction,
        event: &keyboard::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shared: &SharedState,
    ) -> (Status, Option<super::Message>) {
        let keyboard::Event::KeyReleased {
            key,
            modified_key: _,
            physical_key: _,
            location: _,
            modifiers: _,
        } = event
        else {
            return (Status::Ignored, None);
        };

        let Some(pos) = cursor
            .position_in(bounds)
            .map(|pos| screen::Point::new(pos.x, pos.y))
        else {
            return (Status::Ignored, None);
        };

        match key.as_ref() {
            keyboard::Key::Character("r") => {
                (Status::Captured, Some(control::Message::ResetView.into()))
            }
            keyboard::Key::Character("s") => match &shared.spectrogram {
                Some(spectrogram) => (
                    Status::Captured,
                    Some(
                        Message::AddTrackPoint(
                            pos * ScreenToDataAbsolute::new(
                                &screen::Size(bounds.size()),
                                &shared.controls.bounds(),
                                &spectrogram.bounds(),
                            ),
                        )
                        .into(),
                    ),
                ),
                None => (Status::Ignored, None),
            },
            keyboard::Key::Character("f") => (Status::Captured, Some(Message::FindSignals.into())),
            keyboard::Key::Character("p") => {
                (Status::Captured, Some(Message::TogglePredictions.into()))
            }
            _ => (Status::Ignored, None),
        }
    }

    pub fn update(
        &mut self,
        message: Message,
        shared: &SharedState,
        workspace: &WorkspaceShared,
        app: &AppShared,
    ) -> Task<Message> {
        match message {
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
            Message::UpdatePredictions => {
                self.satellites = workspace.active_satellites();
                self.predict_satellites(shared.spectrogram.as_ref(), app.config.site.as_ref())
            }
            Message::SetSatellitePredictions(predictions) => {
                self.satellite_predictions = predictions;
                Task::none()
            }
            Message::SpectrogramUpdated => {
                self.satellite_predictions = None;
                self.track_points.clear();
                self.signals.clear();
                self.crosshair = None;
                self.predict_satellites(shared.spectrogram.as_ref(), app.config.site.as_ref())
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
        }
    }

    fn predict_satellites(
        &self,
        spectrogram: Option<&Spectrogram>,
        site: Option<&Site>,
    ) -> Task<Message> {
        let Some(spectrogram) = spectrogram else {
            log::debug!("No spectrogram loaded, skipping pass predictions");
            return Task::done(Message::SetSatellitePredictions(None));
        };
        let Some(site) = site else {
            log::debug!("No site configured, skipping pass predictions");
            return Task::done(Message::SetSatellitePredictions(None));
        };
        if self.satellites.is_empty() {
            return Task::done(Message::SetSatellitePredictions(None));
        }

        log::debug!("Predicting {} satellites", self.satellites.len());
        let satellites = self.satellites.clone();

        let start_time = spectrogram.start_time;
        let length_s = spectrogram.length().as_seconds_f64();
        let site = site.clone();
        Task::future(async move {
            let result = tokio::task::spawn_blocking(move || {
                orbit::predict_satellites(satellites, start_time, length_s, &site)
            })
            .await;
            match result {
                Ok(predictions) => Message::SetSatellitePredictions(Some(predictions)),
                Err(e) => {
                    log::error!("Failed to predict satellite passes: {}", e);
                    Message::SetSatellitePredictions(None)
                }
            }
        })
    }
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Self) -> bool {
        self.track_points == other.track_points
            && self.signals == other.signals
            && self.crosshair == other.crosshair
    }
}

impl Chart<super::Message> for RFPlot {
    type State = MouseInteraction;

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, chart: ChartBuilder<DB>) {
        match self.overlay.build_chart(state, chart, &self.shared) {
            Ok(()) => (),
            Err(e) => log::error!("Error building chart: {:?}", e),
        }
    }

    fn update(
        &self,
        state: &mut Self::State,
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
                    .handle_mouse(state, event, bounds, cursor, &self.shared)
            }
            canvas::Event::Keyboard(event) => {
                self.overlay
                    .handle_keyboard(state, event, bounds, cursor, &self.shared)
            }
            _ => {
                log::debug!("{:?}", event);
                (Status::Ignored, None)
            }
        }
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            match state {
                MouseInteraction::Idle => mouse::Interaction::Idle,
                MouseInteraction::Panning(_) => mouse::Interaction::Grabbing,
            }
        } else {
            mouse::Interaction::Idle
        }
    }
}
