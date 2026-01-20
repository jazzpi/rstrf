use cosmic::{
    Element, Task,
    iced::{Length, widget as iw},
    widget::{slider, text},
};
use glam::Vec2;

use crate::widgets::rfplot::coord::Coord;

use super::coord;

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 8.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

#[derive(Debug, Clone, Copy)]
pub struct Controls {
    zoom: Vec2,
    center: coord::DataNormalized,
    /// Possible power range
    power_bounds: (f32, f32),
    /// Current power range for display
    power_range: (f32, f32),
}

#[derive(Debug, Clone)]
pub enum Message {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(coord::PlotArea),
    ZoomDelta(coord::PlotArea, f32),
    ZoomDeltaX(coord::PlotArea, f32),
    ZoomDeltaY(coord::PlotArea, f32),
    ResetView,
    UpdateMinPower(f32),
    UpdateMaxPower(f32),
}

impl Controls {
    pub fn new(power_bounds: (f32, f32)) -> Self {
        Self {
            zoom: Vec2::new(ZOOM_MIN, ZOOM_MIN),
            center: coord::DataNormalized::new(0.5, 0.5),
            power_bounds,
            power_range: power_bounds,
        }
    }

    pub fn scale(&self) -> Vec2 {
        Vec2::new(
            1.0 / 2.0_f32.powf(self.zoom.x),
            1.0 / 2.0_f32.powf(self.zoom.y),
        )
    }

    pub fn bounds(&self) -> (Vec2, Vec2) {
        let half_scale = self.scale() / 2.0;
        (
            Vec2::new(
                self.center.0.x - half_scale.x,
                self.center.0.x + half_scale.x,
            ),
            Vec2::new(
                self.center.0.y - half_scale.y,
                self.center.0.y + half_scale.y,
            ),
        )
    }

    pub fn center(&self) -> coord::DataNormalized {
        self.center
    }

    pub fn power_range(&self) -> (f32, f32) {
        self.power_range
    }

    fn control<'a>(
        label: &'static str,
        control: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        iw::row![text(label), control.into()].spacing(10).into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        iw::row![
            Self::control(
                "Zoom Time",
                slider(ZOOM_MIN..=ZOOM_MAX, self.zoom.x, move |zoom| {
                    Message::UpdateZoomX(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
            Self::control(
                "Zoom Freq",
                slider(ZOOM_MIN..=ZOOM_MAX, self.zoom.y, move |zoom| {
                    Message::UpdateZoomY(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
            Self::control(
                "Min Power",
                slider(
                    self.power_bounds.0..=self.power_bounds.1,
                    self.power_bounds.0,
                    move |power| { Message::UpdateMinPower(power) }
                )
                .step(0.1)
                .width(Length::Fill)
            ),
            Self::control(
                "Max Power",
                slider(
                    self.power_bounds.0..=self.power_bounds.1,
                    self.power_bounds.1,
                    move |power| { Message::UpdateMaxPower(power) }
                )
                .step(0.1)
                .width(Length::Fill)
            ),
        ]
        .into()
    }

    #[must_use]
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UpdateZoomX(zoom_x) => {
                self.zoom.x = zoom_x;
            }
            Message::UpdateZoomY(zoom_y) => {
                self.zoom.y = zoom_y;
            }
            Message::PanningDelta(delta) => {
                self.center -= coord::DataNormalized(delta.0 * self.scale());
            }
            Message::ZoomDelta(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;

                let old_data = plot_pos.data_normalized(&self);
                let prev_zoom = self.zoom;
                self.zoom = (prev_zoom + Vec2::splat(delta))
                    .clamp(Vec2::splat(ZOOM_MIN), Vec2::splat(ZOOM_MAX));
                let new_data = plot_pos.data_normalized(&self);
                self.center += old_data - new_data;
            }
            Message::ZoomDeltaX(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let old_x = plot_pos.data_normalized(&self).0.x;
                self.zoom.x = (self.zoom.x + delta).clamp(ZOOM_MIN, ZOOM_MAX);
                let new_x = plot_pos.data_normalized(&self).0.x;
                self.center.0.x += old_x - new_x;
            }
            Message::ZoomDeltaY(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let old_y = plot_pos.data_normalized(&self).0.y;
                self.zoom.y = (self.zoom.y + delta).clamp(ZOOM_MIN, ZOOM_MAX);
                let new_y = plot_pos.data_normalized(&self).0.y;
                self.center.0.y += old_y - new_y;
            }
            Message::ResetView => {
                self.zoom = Vec2::new(ZOOM_MIN, ZOOM_MIN);
                self.center = coord::DataNormalized::new(0.5, 0.5);
            }
            Message::UpdateMinPower(min_power) => {
                self.power_range.0 = min_power.min(self.power_range.1);
            }
            Message::UpdateMaxPower(max_power) => {
                self.power_range.1 = max_power.max(self.power_range.0);
            }
        }
        Task::none()
    }
}
