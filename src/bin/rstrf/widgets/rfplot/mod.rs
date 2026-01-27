use iced::{
    Element, Length, Padding, Task,
    widget::{self, container},
};
use plotters_iced2::ChartWidget;
use rstrf::{coord::plot_area, spectrogram::Spectrogram};

use crate::widgets::rfplot::control::Controls;

mod colormap;
mod control;
pub mod overlay;
mod shader;

#[derive(Debug, Clone)]
pub enum Message {
    Control(control::Message),
    Overlay(overlay::Message),
}

impl From<control::Message> for Message {
    fn from(message: control::Message) -> Self {
        Message::Control(message)
    }
}

impl From<overlay::Message> for Message {
    fn from(message: overlay::Message) -> Self {
        Message::Overlay(message)
    }
}

pub enum MouseInteraction {
    Idle,
    Panning(plot_area::Point),
}

impl Default for MouseInteraction {
    fn default() -> Self {
        MouseInteraction::Idle
    }
}

struct SharedState {
    pub controls: Controls,
    pub spectrogram: Spectrogram,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
}

pub struct RFPlot {
    shared: SharedState,
    overlay: overlay::Overlay,
}

impl RFPlot {
    pub fn new(spectrogram: Spectrogram) -> Self {
        let shared = SharedState {
            controls: Controls::new(spectrogram.power_bounds),
            spectrogram,
            plot_area_margin: 50.0,
        };
        Self {
            shared,
            overlay: overlay::Overlay::default(),
        }
    }

    #[must_use]
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Control(message) => self.shared.controls.update(message).map(Message::from),
            Message::Overlay(message) => self
                .overlay
                .update(message, &self.shared)
                .map(Message::from),
        }
    }

    /// Build the RFPlot widget view.
    ///
    /// The plot itself is implemented as a stack of two layers: the spectrogram itself (see
    /// `shader.rs`) and the overlay (see `overlay.rs`).
    pub fn view(&self) -> Element<'_, Message> {
        let controls = self.shared.controls.view().map(Message::from);

        let spectrogram: Element<'_, Message> = container(
            widget::shader(self)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: self.shared.plot_area_margin,
            left: self.shared.plot_area_margin,
        })
        .into();
        let plot_overlay: Element<'_, Message> = ChartWidget::new(self)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let plot_area: Element<'_, Message> = widget::stack![spectrogram, plot_overlay,].into();

        widget::column![plot_area, controls]
            .padding(10)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
