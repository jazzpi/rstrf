//! This module contains the plot overlay for RFPlot. It draws everything that isn't the spectrogram
//! itself (like axes and overlays). It is also responsible for the user interaction with the plot
//! (like panning/zooming).

use cosmic::{
    iced::{Rectangle, event::Status, mouse},
    widget::canvas,
};
use glam::Vec2;
use itertools::izip;
use plotters::prelude::*;
use plotters_iced::Chart;

use super::{Message, MouseInteraction, RFPlot, control, coord};

impl RFPlot {
    fn build_chart<DB: DrawingBackend>(
        &self,
        _state: &MouseInteraction,
        mut chart: ChartBuilder<DB>,
    ) -> Result<(), String> {
        let (x, y) = self.controls.bounds();
        let x = x * self.spectrogram.length().as_seconds_f32();
        let y = (y - 0.5) * self.spectrogram.bw;
        let mut chart = chart
            .x_label_area_size(self.plot_area_margin)
            .y_label_area_size(self.plot_area_margin)
            .build_cartesian_2d(x.x..x.y, y.x..y.y)
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
                        t > x.x.into()
                            && t < x.y.into()
                            && f > y.x as f64 + sat.tx_freq
                            && f < y.y as f64 + sat.tx_freq
                            && za < std::f64::consts::FRAC_PI_2
                    });
                let Some(first_visible) = first_visible else {
                    continue;
                };
                let first_time = (time[first_visible] as f32).max(x.x);
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
        Ok(())
    }

    fn handle_mouse(
        &self,
        state: &mut MouseInteraction,
        event: mouse::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<control::Message>) {
        use control::Message;
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
                return (Status::Captured, Some(Message::ZoomDelta(plot_pos, delta)));
            } else if cursor.is_over(y_axis) {
                // Zooming over y axis
                return (Status::Captured, Some(Message::ZoomDeltaY(plot_pos, delta)));
            } else if cursor.is_over(x_axis) {
                // Zooming over x axis
                return (Status::Captured, Some(Message::ZoomDeltaX(plot_pos, delta)));
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
                    return (Status::Captured, Some(Message::PanningDelta(delta)));
                }
                _ => {}
            },
        };

        (Status::Captured, None)
    }
}

impl Chart<Message> for RFPlot {
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
    ) -> (Status, Option<Message>) {
        let bounds = Rectangle {
            x: bounds.x + self.plot_area_margin,
            y: bounds.y,
            width: bounds.width - self.plot_area_margin,
            height: bounds.height - self.plot_area_margin,
        };
        if let canvas::Event::Mouse(event) = event {
            let (status, msg) = self.handle_mouse(state, event, bounds, cursor);
            (status, msg.map(Message::from))
        } else {
            log::debug!("{:?}", event);
            (Status::Ignored, None)
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
