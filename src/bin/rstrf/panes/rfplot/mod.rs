use std::path::PathBuf;

use iced::{
    Element, Length, Padding, Size, Task,
    widget::{self, button, container},
};
use plotters_iced2::ChartWidget;
use rfd::AsyncFileDialog;
use rstrf::{coord::plot_area, spectrogram::Spectrogram};

use crate::{
    app::WorkspaceEvent,
    panes::{Message as PaneMessage, PaneWidget, rfplot::control::Controls},
};

mod colormap;
mod control;
pub mod overlay;
mod shader;

#[derive(Debug, Clone)]
pub enum Message {
    Control(control::Message),
    Overlay(overlay::Message),
    PickSpectrogram,
    LoadSpectrogram(Vec<PathBuf>),
    SpectrogramLoaded(Result<Spectrogram, String>),
    Nop,
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

#[derive(Default)]
pub enum MouseInteraction {
    #[default]
    Idle,
    Panning(plot_area::Point),
}

struct SharedState {
    pub controls: Controls,
    pub spectrogram: Option<Spectrogram>,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
}

pub struct RFPlot {
    shared: SharedState,
    overlay: overlay::Overlay,
}

impl RFPlot {
    pub fn new() -> Self {
        let shared = SharedState {
            controls: Controls::new(),
            spectrogram: None,
            plot_area_margin: 50.0,
        };
        Self {
            shared,
            overlay: overlay::Overlay::default(),
        }
    }
}

impl PaneWidget for RFPlot {
    fn update(&mut self, message: PaneMessage) -> Task<PaneMessage> {
        match message {
            PaneMessage::RFPlot(message) => match message {
                Message::Control(message) => self
                    .shared
                    .controls
                    .update(message)
                    .map(|m| PaneMessage::RFPlot(m.into())),
                Message::Overlay(message) => self
                    .overlay
                    .update(message, &self.shared)
                    .map(|m| PaneMessage::RFPlot(m.into())),
                Message::LoadSpectrogram(paths) => Task::future(async move {
                    let spec = rstrf::spectrogram::load(&paths).await;
                    Message::SpectrogramLoaded(spec.map_err(|e| format!("{e:?}"))).into()
                }),
                Message::SpectrogramLoaded(result) => match result {
                    Ok(spec) => {
                        log::info!("Loaded spectrogram: {spec:?}");
                        self.shared.controls.set_power_bounds(spec.power_bounds);
                        self.shared.spectrogram = Some(spec);
                        self.overlay
                            .update(overlay::Message::SpectrogramUpdated, &self.shared)
                            .map(|m| PaneMessage::RFPlot(m.into()))
                    }
                    Err(err) => {
                        log::error!("failed to load spectrogram: {err}");
                        Task::none()
                    }
                },
                Message::PickSpectrogram => Task::future(async {
                    let files = AsyncFileDialog::new()
                        .add_filter("RFFFT spectrograms", &["bin"])
                        .add_filter("All files", &["*"])
                        .pick_files()
                        .await;
                    if let Some(files) = files
                        && !files.is_empty()
                    {
                        Message::LoadSpectrogram(
                            files.iter().map(|f| f.path().to_path_buf()).collect(),
                        )
                        .into()
                    } else {
                        Message::Nop.into()
                    }
                }),
                Message::Nop => Task::none(),
            },
            PaneMessage::Workspace(event) => match event {
                WorkspaceEvent::SatellitesChanged(satellites) => self
                    .overlay
                    .update(overlay::Message::SetSatellites(satellites), &self.shared)
                    .map(|m| PaneMessage::RFPlot(m.into())),
            },
            _ => Task::none(),
        }
    }

    fn view(&self, _size: Size) -> Element<'_, PaneMessage> {
        // The plot is implemented as a stack of two layers: the spectrogram itself (see
        // `shader.rs`) and the overlay (see `overlay.rs`).

        if self.shared.spectrogram.is_none() {
            return container(
                button("Open Spectrogram")
                    .style(button::primary)
                    .on_press(Message::PickSpectrogram.into()),
            )
            .center(Length::Fill)
            .into();
        }

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

        let result: Element<'_, Message> = widget::column![plot_area, controls]
            .padding(10)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        result.map(PaneMessage::from)
    }

    fn title(&self) -> &str {
        "Plot"
    }
}
