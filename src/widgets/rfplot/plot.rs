//! This module contains the plot overlay for RFPlot. It draws everything that isn't the spectrogram
//! itself (like axes and overlays). It is also responsible for the user interaction with the plot
//! (like panning/zooming).

use cosmic::{
    Task,
    iced::{Rectangle, event::Status, keyboard, mouse},
    widget::canvas,
};
use glam::Vec2;
use itertools::{Itertools, izip};
use plotters::prelude::*;
use plotters_iced::Chart;

use super::{
    MouseInteraction, RFPlot, control,
    coord::{self, Coord, clip_line},
};

#[derive(Debug, Clone)]
pub enum Message {
    AddTrackPoint(coord::DataAbsolute),
}

const TRACK_BW: f32 = 5e3; // Hz

fn clamp_line_to_plot(
    bounds: &coord::Bounds,
    points: impl Iterator<Item = Vec2>,
) -> impl Iterator<Item = Vec2> {
    points
        .tuple_windows()
        .filter_map(|(a, b)| clip_line(bounds, a, b))
        .flat_map(|(a, b)| vec![a, b])
}

impl RFPlot {
    fn build_chart<DB: DrawingBackend>(
        &self,
        _state: &MouseInteraction,
        mut chart: ChartBuilder<DB>,
    ) -> Result<(), String> {
        let bounds = (self.controls.bounds() - Vec2::new(0.0, 0.5))
            * Vec2::new(
                self.spectrogram.length().as_seconds_f32(),
                self.spectrogram.bw,
            );
        let mut chart = chart
            .x_label_area_size(self.plot_area_margin)
            .y_label_area_size(self.plot_area_margin)
            .build_cartesian_2d(bounds.x.into_std(), bounds.y.into_std())
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
                        bounds.x.contains(&(t as f32))
                            && bounds.y.contains(&((f - sat.tx_freq) as f32))
                            && za < std::f64::consts::FRAC_PI_2
                    });
                let Some(first_visible) = first_visible else {
                    continue;
                };
                let first_time = (time[first_visible] as f32).max(bounds.x.start);
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
                if bounds.contains(&pos.0) {
                    Some(Circle::new(pos.0.into(), 5, YELLOW.filled()))
                } else {
                    log::debug!("Out of bounds: {:?}, {:?}", pos, bounds);
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
                        .map(|pos| Vec2::new(pos.0.x, pos.0.y + TRACK_BW)),
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
                        .map(|pos| Vec2::new(pos.0.x, pos.0.y - TRACK_BW)),
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
        let pos = coord::Screen(Vec2::new(cursor_pos.x - bounds.x, cursor_pos.y - bounds.y));
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
            let plot_pos = pos.plot(&bounds);
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
                mouse::Event::CursorMoved { position: _ } => {
                    let plot_pos = pos.plot(&bounds);
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

        let plot_pos = cursor
            .position_in(bounds)
            .map(|pos| coord::Screen::new(pos.x, pos.y));

        match key.as_ref() {
            keyboard::Key::Character("r") => {
                (Status::Captured, Some(control::Message::ResetView.into()))
            }
            keyboard::Key::Character("s") => match plot_pos {
                Some(plot_pos) => (
                    Status::Captured,
                    Some(
                        Message::AddTrackPoint(plot_pos.data_absolute(
                            &bounds,
                            &self.controls,
                            &self.spectrogram,
                        ))
                        .into(),
                    ),
                ),
                None => (Status::Ignored, None),
            },
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
            }
        }
        Task::none()
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
