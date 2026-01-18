use cosmic::{
    Element,
    iced::{
        Length, Rectangle,
        widget::{self, column, row, slider, stack, text},
    },
    widget::container,
};
use glam::Vec2;
use rs_trf::spectrogram::Spectrogram;

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 17.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

mod canvas;
mod shader;

#[derive(Debug, Clone, Copy)]
struct Controls {
    zoom: Vec2,
    center: Vec2,
}

impl Controls {
    fn scale(&self) -> Vec2 {
        Vec2::new(
            1.0 / 2.0_f32.powf(self.zoom.x),
            1.0 / 2.0_f32.powf(self.zoom.y),
        )
    }

    fn bounds(&self) -> (Vec2, Vec2) {
        let half_scale = self.scale() / 2.0;
        (
            Vec2::new(self.center.x - half_scale.x, self.center.x + half_scale.x),
            Vec2::new(self.center.y - half_scale.y, self.center.y + half_scale.y),
        )
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            zoom: Vec2::new(ZOOM_MIN, ZOOM_MIN),
            center: Vec2::new(0.5, 0.5),
        }
    }
}

fn interp_bounds(bounds: Vec2, t: f32) -> f32 {
    bounds.x + (bounds.y - bounds.x) * t
}

#[derive(Debug, Clone)]
pub enum Message {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(Vec2),
    ZoomDelta(Vec2, f32),
}

pub enum MouseInteraction {
    Idle,
    Panning(Vec2),
}

impl Default for MouseInteraction {
    fn default() -> Self {
        MouseInteraction::Idle
    }
}

pub struct RFPlot {
    controls: Controls,
    spectrogram: Spectrogram,
}

impl RFPlot {
    pub fn new(spectrogram: Spectrogram) -> Self {
        Self {
            controls: Controls::default(),
            spectrogram,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::UpdateZoomX(zoom_x) => {
                self.controls.zoom.x = zoom_x;
            }
            Message::UpdateZoomY(zoom_y) => {
                self.controls.zoom.y = zoom_y;
            }
            Message::PanningDelta(delta) => {
                let scale = self.controls.scale();
                self.controls.center.x -= delta.x * scale.x;
                self.controls.center.y -= delta.y * scale.y;
            }
            Message::ZoomDelta(pos, delta) => {
                let delta = delta * ZOOM_WHEEL_SCALE;
                let prev_scale = self.controls.scale();
                let prev_zoom = self.controls.zoom;
                self.controls.zoom = (prev_zoom + Vec2::splat(delta))
                    .max(Vec2::splat(ZOOM_MIN))
                    .min(Vec2::splat(ZOOM_MAX));

                let vec = pos - self.controls.center;
                let new_scale = self.controls.scale();
                self.controls.center += vec * (prev_scale - new_scale) * 2.0;
            }
        }
    }

    fn control<'a>(
        label: &'static str,
        control: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        row![text(label), control.into()].spacing(10).into()
    }

    /// Build the RFPlot widget view.
    ///
    /// The plot itself is implemented as a stack of two layers: the spectrogram itself (see
    /// `shader.rs`) and the overlay (see `canvas.rs`).
    pub fn view(&self) -> Element<'_, Message> {
        let controls = row![
            Self::control(
                "Zoom Time",
                slider(ZOOM_MIN..=ZOOM_MAX, self.controls.zoom.x, move |zoom| {
                    Message::UpdateZoomX(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
            Self::control(
                "Zoom Freq",
                slider(ZOOM_MIN..=ZOOM_MAX, self.controls.zoom.y, move |zoom| {
                    Message::UpdateZoomY(zoom)
                })
                .step(0.01)
                .width(Length::Fill)
            ),
        ];

        let spectrogram: Element<'_, Message> = container(
            widget::shader(self)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(50.0)
        .into();
        let axes_overlay: Element<'_, Message> = widget::canvas(self)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let plot_area: Element<'_, Message> = stack![spectrogram, axes_overlay].into();

        column![plot_area, controls]
            .padding(10)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Normalize screen coordinates to [0, 1], assuming (0,0) is top-left. This seems to be the
    /// case for scrolling events, regardless of the bounds.x/y.
    fn normalize_scroll_position(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        Vec2::new(pos.x / bounds.width, 1.0 - pos.y / bounds.height)
    }

    /// Normalize screen coordinates to [0, 1], assuming (bounds.x, bounds.y) is top-left. This
    /// seems to be the case for mouse click & move events.
    fn normalize_click_position(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        Vec2::new(
            (pos.x - bounds.x) / bounds.width,
            1.0 - (pos.y - bounds.y) / bounds.height,
        )
    }

    fn screen_scroll_to_uv(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        let norm = self.normalize_scroll_position(pos, bounds);
        let center = self.controls.center;
        let scale = self.controls.scale();
        Vec2::new(
            center.x + (norm.x - 0.5) * scale.x,
            center.y + (norm.y - 0.5) * scale.y,
        )
    }

    fn screen_click_to_uv(&self, pos: Vec2, bounds: &Rectangle) -> Vec2 {
        let norm = self.normalize_click_position(pos, bounds);
        let center = self.controls.center;
        let scale = self.controls.scale();
        Vec2::new(
            center.x + (norm.x - 0.5) * scale.x,
            center.y + (norm.y - 0.5) * scale.y,
        )
    }
}
