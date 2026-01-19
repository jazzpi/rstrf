//! This module contains the plotters-cosmic-iced program for the RFPlot widget. This builds the
//! plotters plot, which shows axes and (in the future) other overlays.

use cosmic::{
    iced::{Rectangle, event::Status, mouse},
    widget::canvas,
};
use itertools::izip;
use plotters::prelude::*;
use plotters_iced::Chart;

use super::{Message, MouseInteraction, RFPlot};

impl Chart<Message> for RFPlot {
    type State = MouseInteraction;

    fn build_chart<DB: DrawingBackend>(&self, _state: &Self::State, mut chart: ChartBuilder<DB>) {
        let (x, y) = self.controls.bounds();
        let x = x * self.spectrogram.length().num_milliseconds() as f32 / 1000.0;
        let y = (y - 0.5) * self.spectrogram.bw;
        let mut chart = chart
            .x_label_area_size(self.plot_area_margin)
            .y_label_area_size(self.plot_area_margin)
            .build_cartesian_2d(x.x..x.y, y.x..y.y)
            .expect("Failed to build chart");

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
            .expect("Failed to draw mesh");

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
                    .expect(format!("Could not draw line for satellite {}", id).as_str())
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
                    .expect(format!("Could not draw label for satellite {}", id).as_str());
            }
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
            self.handle_mouse(state, event, bounds, cursor)
        } else {
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
