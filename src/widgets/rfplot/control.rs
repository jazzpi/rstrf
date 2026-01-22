use cosmic::{
    Element, Task,
    iced::{Length, widget as iw},
    widget::{slider, text},
};
use glam::Vec2;
use rstrf::coord::{PlotAreaToDataNormalized, data_normalized, plot_area};

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 8.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

#[derive(Debug, Clone, Copy)]
pub struct Controls {
    log_scale: Vec2,
    center: data_normalized::Point,
    /// Possible power range
    power_bounds: (f32, f32),
    /// Current power range for display
    power_range: (f32, f32),
}

#[derive(Debug, Clone)]
pub enum Message {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(plot_area::Vector),
    ZoomDelta(plot_area::Point, f32),
    ZoomDeltaX(plot_area::Point, f32),
    ZoomDeltaY(plot_area::Point, f32),
    ResetView,
    UpdateMinPower(f32),
    UpdateMaxPower(f32),
}

impl Controls {
    pub fn new(power_bounds: (f32, f32)) -> Self {
        Self {
            log_scale: Vec2::new(ZOOM_MIN, ZOOM_MIN),
            center: data_normalized::Point::new(0.5, 0.5),
            power_bounds,
            power_range: power_bounds,
        }
    }

    pub fn size(&self) -> data_normalized::Size {
        data_normalized::Size::new(
            1.0 / 2.0_f32.powf(self.log_scale.x),
            1.0 / 2.0_f32.powf(self.log_scale.y),
        )
    }

    pub fn bounds(&self) -> data_normalized::Rectangle {
        let size = self.size();
        data_normalized::Rectangle::new(
            data_normalized::Point::new(
                self.center.0.x - size.0.width / 2.0,
                self.center.0.y - size.0.height / 2.0,
            ),
            size,
        )
    }

    pub fn to_data_normalized(&self) -> PlotAreaToDataNormalized {
        PlotAreaToDataNormalized::new(&self.bounds())
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
                slider(ZOOM_MIN..=ZOOM_MAX, self.log_scale.x, move |zoom| {
                    Message::UpdateZoomX(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
            Self::control(
                "Zoom Freq",
                slider(ZOOM_MIN..=ZOOM_MAX, self.log_scale.y, move |zoom| {
                    Message::UpdateZoomY(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
            Self::control(
                "Min Power",
                slider(
                    self.power_bounds.0..=self.power_bounds.1,
                    self.power_range.0,
                    move |power| { Message::UpdateMinPower(power) }
                )
                .step(0.1)
                .width(Length::Fill)
            ),
            Self::control(
                "Max Power",
                slider(
                    self.power_bounds.0..=self.power_bounds.1,
                    self.power_range.1,
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
                self.log_scale.x = zoom_x;
            }
            Message::UpdateZoomY(zoom_y) => {
                self.log_scale.y = zoom_y;
            }
            Message::PanningDelta(delta) => {
                self.center -= delta * self.to_data_normalized();
            }
            Message::ZoomDelta(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;

                let old_data = plot_pos * self.to_data_normalized();
                let prev_zoom = self.log_scale;
                self.log_scale = (prev_zoom + Vec2::splat(delta))
                    .clamp(Vec2::splat(ZOOM_MIN), Vec2::splat(ZOOM_MAX));
                let new_data = plot_pos * self.to_data_normalized();
                self.center += old_data - new_data;
            }
            Message::ZoomDeltaX(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let old_x = (plot_pos * self.to_data_normalized()).0.x;
                self.log_scale.x = (self.log_scale.x + delta).clamp(ZOOM_MIN, ZOOM_MAX);
                let new_x = (plot_pos * self.to_data_normalized()).0.x;
                self.center.0.x += old_x - new_x;
            }
            Message::ZoomDeltaY(plot_pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let old_y = (plot_pos * self.to_data_normalized()).0.y;
                self.log_scale.y = (self.log_scale.y + delta).clamp(ZOOM_MIN, ZOOM_MAX);
                let new_y = (plot_pos * self.to_data_normalized()).0.y;
                self.center.0.y += old_y - new_y;
            }
            Message::ResetView => {
                self.log_scale = Vec2::new(ZOOM_MIN, ZOOM_MIN);
                self.center = data_normalized::Point::new(0.5, 0.5);
            }
            Message::UpdateMinPower(min_power) => {
                self.power_range.0 = min_power.min(self.power_range.1);
            }
            Message::UpdateMaxPower(max_power) => {
                self.power_range.1 = max_power.max(self.power_range.0);
            }
        }
        self.snap_to_bounds();
        Task::none()
    }

    /// Ensure that the current view bounds are within [0, 1] in both axes.
    fn snap_to_bounds(&mut self) {
        let bounds = self.bounds().0;
        let dx = if bounds.x < 0.0 {
            -bounds.x
        } else if bounds.x + bounds.width > 1.0 {
            1.0 - (bounds.x + bounds.width)
        } else {
            0.0
        };
        let dy = if bounds.y < 0.0 {
            -bounds.y
        } else if bounds.y + bounds.height > 1.0 {
            1.0 - (bounds.y + bounds.height)
        } else {
            0.0
        };
        self.center.0.x += dx;
        self.center.0.y += dy;
    }
}
