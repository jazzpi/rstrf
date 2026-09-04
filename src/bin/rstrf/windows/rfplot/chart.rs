//! Drawing the plot: axes, grid, satellite curves, marks and the crosshair readout.
//!
//! The spectrogram itself is drawn by the wgpu pipeline in `shader.rs`; everything layered on
//! top of it is drawn here.

use chrono::Duration;
use copy_range::CopyRange;
use iced::{Rectangle, event::Status, keyboard, mouse, widget::canvas};
use itertools::{Itertools, izip};
use ndarray::s;
use plotters::coord::types::RangedCoordf32;
use plotters::coord::{
    combinators::WithKeyPoints,
    ranged1d::{KeyPointHint, NoDefaultFormatting, ValueFormatter},
};
use plotters::prelude::*;
use plotters::style::text_anchor::{HPos, Pos, VPos};
use plotters_iced2::Chart;
use rstrf::{
    chart::{ReferenceMode, ReferencedTicks, datetime_referenced_ticks},
    coord::{
        DataAbsoluteToDataNormalized, DataNormalizedToDataAbsolute, PlotAreaToDataAbsolute,
        data_absolute, plot_area,
    },
    util::{clip_line, sec_to_duration},
};

use crate::app::AppShared;

use super::{MouseState, RectAction, State};

/// Borrows everything `build_chart` needs, so the `Chart` impl can reach both the window's state
/// and the shared app state that `view()` holds.
pub(super) struct PlotChart<'a> {
    pub state: &'a State,
    pub app: &'a AppShared,
}

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
                                ("sans-serif", 12)
                                    .into_font()
                                    .color(&color)
                                    .pos(Pos::new(HPos::Left, VPos::Bottom)),
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
