//! This module contains the plot overlay for RFPlot. It draws everything that isn't the spectrogram
//! itself (like axes and overlays). It is also responsible for the user interaction with the plot
//! (like panning/zooming).

use copy_range::CopyRange;
use cosmic::{
    Task,
    iced::{Rectangle, event::Status, keyboard, mouse},
    widget::canvas,
};
use itertools::{Itertools, izip};
use ndarray::s;
use ndarray_stats::QuantileExt;
use plotters::prelude::*;
use plotters_iced::Chart;
use rstrf::{
    coord::{
        DataNormalizedToDataAbsolute, PlotAreaToDataAbsolute, ScreenToDataAbsolute,
        ScreenToPlotArea, data_absolute, plot_area, screen,
    },
    util::{clip_line, to_index},
};

use super::{MouseInteraction, RFPlot, control};

#[derive(Debug, Clone)]
pub enum Message {
    AddTrackPoint(data_absolute::Point),
    FindSignals,
    FoundSignals(Vec<data_absolute::Point>),
    UpdateCrosshair(Option<plot_area::Point>),
}

// TODO: make this configurable
const TRACK_BW: f32 = 10e3; // Hz

fn clamp_line_to_plot(
    bounds: &data_absolute::Rectangle,
    points: impl Iterator<Item = data_absolute::Point>,
) -> impl Iterator<Item = data_absolute::Point> {
    points
        .tuple_windows()
        .filter_map(|(a, b)| clip_line(&bounds.0, a.0, b.0))
        .flat_map(|(a, b)| vec![a, b])
        .map(|p| data_absolute::Point(p))
}

impl RFPlot {
    fn build_chart<DB: DrawingBackend>(
        &self,
        _state: &MouseInteraction,
        mut chart: ChartBuilder<DB>,
    ) -> Result<(), String> {
        let bounds =
            self.controls.bounds() * DataNormalizedToDataAbsolute::new(&self.spectrogram.bounds());
        let x = CopyRange::from_std(bounds.0.x..(bounds.0.x + bounds.0.width));
        let y = CopyRange::from_std(bounds.0.y..(bounds.0.y + bounds.0.height));
        let mut chart = chart
            .x_label_area_size(self.plot_area_margin)
            .y_label_area_size(self.plot_area_margin)
            .build_cartesian_2d(x.into_std(), y.into_std())
            .map_err(|e| format!("Failed to build chart: {:?}", e))?;

        chart
            .configure_mesh()
            .axis_style(&WHITE)
            .label_style(&WHITE)
            .bold_line_style(&WHITE.mix(0.2))
            .light_line_style(&WHITE.mix(0.2))
            .y_label_formatter(&|v| format!("{:.1}", v / 1000.0))
            .x_desc("Time [s]")
            .y_desc("Frequency offset [kHz]")
            .draw()
            .map_err(|e| format!("Failed to draw mesh: {:?}", e))?;

        if let Some(satellite_predictions) = &self.satellite_predictions {
            let time = &satellite_predictions.times;
            for sat in &self.satellites {
                let id = sat.norad_id();
                log::trace!("Plotting satellite {}", id);
                let freq = &satellite_predictions
                    .frequencies
                    .get(&id)
                    .expect("Missing frequency prediction for satellite");
                let za = &satellite_predictions
                    .zenith_angles
                    .get(&id)
                    .expect("Missing zenith angle prediction for satellite");

                chart
                    .draw_series(LineSeries::new(
                        izip!(time.iter(), freq.iter(), za.iter()).filter_map(|(&t, &f, &za)| {
                            if za < std::f64::consts::FRAC_PI_2 {
                                Some((t as f32, (f - sat.tx_freq) as f32))
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
                            && y.contains(&((f - sat.tx_freq) as f32))
                            && za < std::f64::consts::FRAC_PI_2
                    });
                let Some(first_visible) = first_visible else {
                    continue;
                };
                let first_time = (time[first_visible] as f32).max(x.start);
                let first_freq = (freq[first_visible] - sat.tx_freq) as f32;
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
                    self.track_points
                        .iter()
                        .map(|pos| data_absolute::Point::new(pos.0.x, pos.0.y + TRACK_BW / 2.0)),
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
                    self.track_points
                        .iter()
                        .map(|pos| data_absolute::Point::new(pos.0.x, pos.0.y - TRACK_BW / 2.0)),
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
        if let Some(crosshair) = &self.crosshair {
            if bounds.contains(*crosshair) {
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
                        style.clone(),
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
                let power = self.spectrogram.data()[(
                    to_index(
                        crosshair.0.x
                            * (self.spectrogram.data().dim().0 as f32
                                / self.spectrogram.length().as_seconds_f32()),
                        self.spectrogram.data().dim().0,
                    ),
                    to_index(
                        (crosshair.0.y + self.spectrogram.bw / 2.0)
                            * (self.spectrogram.data().dim().1 as f32 / self.spectrogram.bw),
                        self.spectrogram.data().dim().1,
                    ),
                )];
                let crosshair_pos = plot_area::Point::new(0.01, 0.99)
                    * PlotAreaToDataAbsolute::new(
                        &self.controls.bounds(),
                        &self.spectrogram.bounds(),
                    );
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
        }

        Ok(())
    }

    fn handle_mouse(
        &self,
        state: &mut MouseInteraction,
        event: mouse::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
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
                height: self.plot_area_margin,
            };
            let y_axis = Rectangle {
                x: bounds.x - self.plot_area_margin,
                y: bounds.y,
                width: self.plot_area_margin,
                height: bounds.height,
            };
            if cursor.is_over(bounds) {
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDelta(plot_pos, delta).into()),
                );
            } else if cursor.is_over(y_axis) {
                // Zooming over y axis
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDeltaY(plot_pos, delta).into()),
                );
            } else if cursor.is_over(x_axis) {
                // Zooming over x axis
                return (
                    Status::Captured,
                    Some(CMessage::ZoomDeltaX(plot_pos, delta).into()),
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
        event: keyboard::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
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

        let pos = cursor
            .position_in(bounds)
            .map(|pos| screen::Point::new(pos.x, pos.y));

        match (key.as_ref(), pos) {
            (keyboard::Key::Character("r"), _) => {
                (Status::Captured, Some(control::Message::ResetView.into()))
            }
            (keyboard::Key::Character("s"), Some(pos)) => (
                Status::Captured,
                Some(
                    Message::AddTrackPoint(
                        pos * ScreenToDataAbsolute::new(
                            &screen::Size(bounds.size()),
                            &self.controls.bounds(),
                            &self.spectrogram.bounds(),
                        ),
                    )
                    .into(),
                ),
            ),
            (keyboard::Key::Character("f"), _) => {
                (Status::Captured, Some(Message::FindSignals.into()))
            }
            _ => (Status::Ignored, None),
        }
    }

    #[must_use]
    pub fn update_plot(&mut self, message: Message) -> Task<Message> {
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
                    let data = self.spectrogram.data();
                    let (nt, nf) = data.dim();
                    let t_scale = nt as f32 / self.spectrogram.length().as_seconds_f32();
                    let bw = self.spectrogram.bw;
                    let f_scale = nf as f32 / bw;
                    let half_bw_idx = (TRACK_BW * 0.5 * f_scale) as usize;
                    let track_points = self
                        .track_points
                        .iter()
                        .map(|p| {
                            (
                                // TODO: This will clamp x/y to bounds individually -> might change slope
                                // for out-of-bounds points
                                to_index(p.0.x * t_scale, nt),
                                to_index((p.0.y + bw / 2.0) * f_scale, nf),
                            )
                        })
                        .collect_vec();
                    let t_range =
                        track_points.first().unwrap().0..(track_points.last().unwrap().0 + 1);
                    let data = data.slice(s![t_range.clone(), ..]).to_owned();
                    cosmic::task::future(async move {
                        tokio::task::spawn_blocking(move || {
                            let peaks = track_points
                                .iter()
                                .tuple_windows()
                                .flat_map(|(a, b)| {
                                    let slope =
                                        (b.1 as f32 - a.1 as f32) / (b.0 as f32 - a.0 as f32);
                                    (a.0..=b.0)
                                        .map(|t_idx| {
                                            let center_f =
                                                (a.1 as f32 + slope * (t_idx - a.0) as f32).round()
                                                    as usize;
                                            let f_range = center_f.saturating_sub(half_bw_idx)
                                                ..(center_f + half_bw_idx).min(nf - 1);
                                            // This approximates the rfplot fit_trace() algorithm.
                                            // That works on non-log data, and for some reason it
                                            // doesn't seem to work very well with log-scale data.
                                            let slice = data
                                                .slice(s![t_idx - t_range.start, f_range.clone()])
                                                .mapv(|v| 10.0_f32.powf(v / 10.0));

                                            let max = slice.max().ok()?;
                                            let sum = slice.sum() - max;
                                            let sq_sum = slice.mapv(|v| v * v).sum() - max * max;
                                            let mean = sum / (slice.len() as f32 - 1.0);
                                            let std_dev = ((sq_sum / (slice.len() as f32 - 1.0))
                                                - (mean * mean))
                                                .sqrt();
                                            let sigma = (max - mean) / std_dev;
                                            // TODO: make this configurable
                                            if sigma > 5.0 {
                                                Some(data_absolute::Point::new(
                                                    t_idx as f32 / t_scale,
                                                    ((slice.argmax().ok()? + f_range.start) as f32
                                                        / f_scale)
                                                        - bw / 2.0,
                                                ))
                                            } else {
                                                None
                                            }
                                        })
                                        // looks dumb but without this we get ownership issues for
                                        // `slope` for some reason
                                        .collect_vec()
                                        .into_iter()
                                        .flatten()
                                })
                                .collect_vec();
                            log::info!("Found {} signal peaks", peaks.len());
                            Message::FoundSignals(peaks)
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
                self.crosshair = plot_pos.map(|p| {
                    p * PlotAreaToDataAbsolute::new(
                        &self.controls.bounds(),
                        &self.spectrogram.bounds(),
                    )
                });
                Task::none()
            }
        }
    }
}

impl Chart<super::Message> for RFPlot {
    type State = MouseInteraction;

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, chart: ChartBuilder<DB>) {
        match self.build_chart(state, chart) {
            Ok(()) => (),
            Err(e) => log::error!("Error building chart: {:?}", e),
        }
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<super::Message>) {
        let bounds = Rectangle {
            x: bounds.x + self.plot_area_margin,
            y: bounds.y,
            width: bounds.width - self.plot_area_margin,
            height: bounds.height - self.plot_area_margin,
        };
        match event {
            canvas::Event::Mouse(event) => self.handle_mouse(state, event, bounds, cursor),
            canvas::Event::Keyboard(event) => self.handle_keyboard(state, event, bounds, cursor),
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
