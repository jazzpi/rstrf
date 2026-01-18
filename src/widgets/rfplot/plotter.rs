//! This module contains the plotters-cosmic-iced program for the RFPlot widget. This builds the
//! plotters plot, which shows axes and (in the future) other overlays.

use cosmic::{
    iced::{Rectangle, Size, event::Status, mouse},
    widget::canvas,
};
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
            .x_label_area_size(50)
            .y_label_area_size(50)
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
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<Message>) {
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
