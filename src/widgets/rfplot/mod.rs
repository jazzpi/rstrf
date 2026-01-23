use iced::{
    Element, Length, Padding, Task,
    widget::{self, container},
};
use plotters_iced2::ChartWidget;
use rstrf::{
    coord::{data_absolute, plot_area},
    orbit,
    spectrogram::Spectrogram,
};

use crate::widgets::rfplot::control::Controls;

mod colormap;
mod control;
mod plot;
mod shader;

#[derive(Debug, Clone)]
pub enum Message {
    Control(control::Message),
    Plot(plot::Message),
    SetSatellites(Vec<orbit::Satellite>),
    SetSatellitePredictions(Option<orbit::Predictions>),
}

impl From<control::Message> for Message {
    fn from(message: control::Message) -> Self {
        Message::Control(message)
    }
}

impl From<plot::Message> for Message {
    fn from(message: plot::Message) -> Self {
        Message::Plot(message)
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

pub struct RFPlot {
    controls: Controls,
    spectrogram: Spectrogram,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    plot_area_margin: f32,
    satellites: Vec<orbit::Satellite>,
    satellite_predictions: Option<orbit::Predictions>,
    track_points: Vec<data_absolute::Point>,
    signals: Vec<data_absolute::Point>,
    crosshair: Option<data_absolute::Point>,
}

impl RFPlot {
    pub fn new(spectrogram: Spectrogram) -> Self {
        Self {
            controls: Controls::new(spectrogram.power_bounds),
            spectrogram,
            plot_area_margin: 50.0,
            satellites: Vec::new(),
            satellite_predictions: None,
            track_points: Vec::new(),
            signals: Vec::new(),
            crosshair: None,
        }
    }

    #[must_use]
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Control(message) => self.controls.update(message).map(Message::from),
            Message::Plot(message) => self.update_plot(message).map(Message::from),
            Message::SetSatellites(satellites) => {
                self.satellites = satellites;
                // TODO: clear previous predictions here?
                log::debug!("Using {} satellites", self.satellites.len());
                let satellites = self.satellites.clone();
                let start_time = self.spectrogram.start_time;
                let length_s = self.spectrogram.length().as_seconds_f64();
                Task::future(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        orbit::predict_satellites(satellites, start_time, length_s)
                    })
                    .await;
                    match result {
                        Ok(predictions) => Message::SetSatellitePredictions(Some(predictions)),
                        Err(e) => {
                            log::error!("Failed to predict satellite passes: {}", e);
                            Message::SetSatellitePredictions(None)
                        }
                    }
                })
            }
            Message::SetSatellitePredictions(predictions) => {
                self.satellite_predictions = predictions;
                Task::none()
            }
        }
    }

    /// Build the RFPlot widget view.
    ///
    /// The plot itself is implemented as a stack of two layers: the spectrogram itself (see
    /// `shader.rs`) and the overlay (see `plot.rs`).
    pub fn view(&self) -> Element<'_, Message> {
        let controls = self.controls.view().map(Message::from);

        let spectrogram: Element<'_, Message> = container(
            widget::shader(self)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: self.plot_area_margin,
            left: self.plot_area_margin,
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
