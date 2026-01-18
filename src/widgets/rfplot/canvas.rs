//! This module contains the canvas implementation for the RFPlot widget. The canvas is responsible
//! for rendering the axes and (in the future) other overlays on top of the spectrogram.
use cosmic::iced::{Color, Font, Point, Rectangle, Size, mouse, widget::canvas};

use super::{Message, RFPlot, interp_bounds};

impl canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for RFPlot {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        let margin = 50.0;
        let plot_width = bounds.width - margin * 2.0;
        let plot_height = bounds.height - margin * 2.0;

        // Draw border around plot area
        let plot_rect = canvas::Path::rectangle(
            Point::new(margin, margin),
            Size::new(plot_width, plot_height),
        );
        frame.stroke(
            &plot_rect,
            canvas::Stroke::default()
                .with_width(1.0)
                .with_color(Color::from_rgb(0.5, 0.5, 0.5)),
        );

        let (x_bounds, y_bounds) = self.controls.bounds();

        // Draw X-axis (time) ticks and labels
        let num_x_ticks = 5;
        let spec_duration = self.spectrogram.length();
        for i in 0..=num_x_ticks {
            let x = margin + (plot_width * i as f32 / num_x_ticks as f32);
            let y = margin + plot_height;

            // Tick mark
            let tick = canvas::Path::line(Point::new(x, y), Point::new(x, y + 5.0));
            frame.stroke(
                &tick,
                canvas::Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgb(0.5, 0.5, 0.5)),
            );

            // Label
            let time_offset = spec_duration.num_milliseconds() as f32
                * interp_bounds(x_bounds, i as f32 / num_x_ticks as f32)
                / 1000.0;
            let label = format!("{:.1}s", time_offset);
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x, y + 10.0),
                color: Color::from_rgb(0.8, 0.8, 0.8),
                size: 12.0.into(),
                font: Font::default(),
                horizontal_alignment: cosmic::iced::alignment::Horizontal::Center,
                vertical_alignment: cosmic::iced::alignment::Vertical::Top,
                ..Default::default()
            });
        }

        // Draw Y-axis (frequency) ticks and labels
        // TODO: Show center frequency + offsets instead? Otherwise differentiating between
        // 401.023 MHz and 401.026 MHz is a bit difficult.
        let num_y_ticks = 5;
        let freq_min = self.spectrogram.freq - self.spectrogram.bw / 2.0;
        let freq_max = self.spectrogram.freq + self.spectrogram.bw / 2.0;
        for i in 0..=num_y_ticks {
            let x = margin;
            let y = margin + plot_height - (plot_height * i as f32 / num_y_ticks as f32);

            // Tick mark
            let tick = canvas::Path::line(Point::new(x - 5.0, y), Point::new(x, y));
            frame.stroke(
                &tick,
                canvas::Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgb(0.5, 0.5, 0.5)),
            );

            // Label
            let freq = freq_min
                + (freq_max - freq_min) * interp_bounds(y_bounds, i as f32 / num_y_ticks as f32);
            let label = if freq > 1e6 {
                format!("{:.1}MHz", freq / 1e6)
            } else if freq > 1e3 {
                format!("{:.1}kHz", freq / 1e3)
            } else {
                format!("{:.0}Hz", freq)
            };
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x - 10.0, y),
                color: Color::from_rgb(0.8, 0.8, 0.8),
                size: 12.0.into(),
                font: Font::default(),
                horizontal_alignment: cosmic::iced::alignment::Horizontal::Right,
                vertical_alignment: cosmic::iced::alignment::Vertical::Center,
                ..Default::default()
            });
        }

        // Axis labels
        frame.fill_text(canvas::Text {
            content: "Time".to_string(),
            position: Point::new(bounds.width / 2.0, bounds.height - 10.0),
            color: Color::from_rgb(0.8, 0.8, 0.8),
            size: 14.0.into(),
            font: Font::default(),
            horizontal_alignment: cosmic::iced::alignment::Horizontal::Center,
            vertical_alignment: cosmic::iced::alignment::Vertical::Bottom,
            ..Default::default()
        });

        frame.fill_text(canvas::Text {
            content: "Frequency".to_string(),
            position: Point::new(10.0, bounds.height / 2.0),
            color: Color::from_rgb(0.8, 0.8, 0.8),
            size: 14.0.into(),
            font: Font::default(),
            horizontal_alignment: cosmic::iced::alignment::Horizontal::Left,
            vertical_alignment: cosmic::iced::alignment::Vertical::Center,
            ..Default::default()
        });

        vec![frame.into_geometry()]
    }
}
