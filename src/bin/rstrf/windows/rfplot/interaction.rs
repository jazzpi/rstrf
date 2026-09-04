//! Transient interaction state and the mouse/keyboard handlers that drive it.
//!
//! The handlers take `&State` and mutate the `Cell`s below directly rather than round-tripping
//! through a message, so a redraw has to be forced with `Message::Refresh` afterwards.

use std::cell::Cell;

use iced::{
    Rectangle,
    event::Status,
    keyboard::{self, key::Named},
    mouse,
};
use rstrf::{
    coord::{
        DataAbsoluteToDataNormalized, DataAbsoluteToScreen, PlotAreaToDataAbsolute,
        ScreenToPlotArea, data_absolute, plot_area, screen,
    },
    util::is_modifier,
};

use super::{DisplayMsg, MarksMsg, State, ViewMsg, marks::MarkAction};

#[derive(Clone, Copy, Debug)]
pub enum RectAction {
    Delete,
    Zoom,
    MarkCentroid,
}

#[derive(Default, Clone, Copy, Debug)]
pub enum MouseState {
    #[default]
    Idle,
    Panning(plot_area::Point),
    DrawingRect {
        action: RectAction,
        corner1: plot_area::Point,
        corner2: plot_area::Point,
    },
    Marking(MarkAction),
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Interaction {
    pub crosshair: Cell<Option<data_absolute::Point>>,
    pub mouse_state: Cell<MouseState>,
    pub modifiers: Cell<keyboard::Modifiers>,
}

/// Maximum cursor-to-mark distance (in screen pixels) for a right-click to delete a mark. Marks
/// render as radius-5 circles, so this gives a comfortable grab radius around them.
const DELETE_TOLERANCE_PX: f32 = 15.0;

impl State {
    pub(super) fn handle_mouse(
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

    pub(super) fn handle_keyboard(
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstrf::coord::data_normalized;

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
}
