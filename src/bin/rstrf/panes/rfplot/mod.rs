use std::path::PathBuf;

use iced::{
    Element, Length, Padding, Size, Task, keyboard,
    widget::{self, button, container},
};
use plotters_iced2::ChartWidget;
use rfd::AsyncFileDialog;
use rstrf::{coord::plot_area, spectrogram::Spectrogram};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{app::AppShared, panes::rfplot::control::Controls};

mod control;
pub mod overlay;
mod shader;

#[derive(Debug, Clone)]
pub enum Message {
    Control(control::Message),
    Overlay(overlay::Message),
    PickSpectrogram,
    LoadSpectrogram(Vec<PathBuf>),
    SpectrogramLoaded(Result<(Vec<PathBuf>, Spectrogram), String>),
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
pub struct MouseInteraction {
    pub drag: DragState,
    pub modifiers: keyboard::Modifiers,
}

#[derive(Default, Clone, Copy)]
pub enum DragState {
    #[default]
    Idle,
    Panning(plot_area::Point),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Default, Clone)]
struct SharedState {
    pub controls: Controls,
    pub spectrogram_files: Vec<PathBuf>,
    #[serde(skip)]
    pub spectrogram: Option<Spectrogram>,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct RFPlot {
    shared: SharedState,
    overlay: overlay::Overlay,
    id: Uuid,
}

impl RFPlot {
    pub fn new() -> Self {
        let shared = SharedState {
            plot_area_margin: 50.0,
            ..Default::default()
        };
        let id = Uuid::new_v4();
        Self {
            shared,
            overlay: overlay::Overlay::default(),
            id,
        }
    }

    pub fn init(&mut self, app: &AppShared) -> Task<Message> {
        if self.shared.spectrogram_files.is_empty() {
            Task::none()
        } else {
            self.update(
                Message::LoadSpectrogram(self.shared.spectrogram_files.clone()),
                app,
            )
        }
    }

    pub fn update(&mut self, message: Message, app: &AppShared) -> Task<Message> {
        let workspace = &app.workspace_shared;
        match message {
            Message::Control(message) => self.shared.controls.update(message).map(Message::Control),
            Message::Overlay(message) => self
                .overlay
                .update(message, &self.shared, workspace, app)
                .map(Message::Overlay),
            Message::LoadSpectrogram(paths) => Task::future(async move {
                let spec = rstrf::spectrogram::load(&paths).await;
                Message::SpectrogramLoaded(spec.map(|s| (paths, s)).map_err(|e| format!("{e:?}")))
            }),
            Message::SpectrogramLoaded(result) => match result {
                Ok((paths, spec)) => {
                    log::info!("Loaded spectrogram: {spec:?}");
                    self.shared.controls.set_power_bounds(spec.power_bounds);
                    self.shared.spectrogram = Some(spec);
                    self.shared.spectrogram_files = paths;
                    self.overlay
                        .update(
                            overlay::Message::SpectrogramUpdated,
                            &self.shared,
                            workspace,
                            app,
                        )
                        .map(Message::Overlay)
                }
                Err(err) => {
                    log::error!("Failed to load spectrogram: {err}");
                    Task::none()
                }
            },
            Message::PickSpectrogram => Task::future(async {
                let files = AsyncFileDialog::new()
                    .add_filter("Supported spectrogram formats", &["rstrf", "bin"])
                    .add_filter("rSTRF spectrograms", &["rstrf"])
                    .add_filter("STRF spectrograms", &["bin"])
                    .add_filter("All files", &["*"])
                    .pick_files()
                    .await;
                if let Some(files) = files
                    && !files.is_empty()
                {
                    Message::LoadSpectrogram(files.iter().map(|f| f.path().to_path_buf()).collect())
                } else {
                    Message::Nop
                }
            }),
            Message::Nop => Task::none(),
        }
    }

    pub fn title(&self) -> String {
        format!(
            "RFPlot: {}",
            self.shared
                .spectrogram
                .as_ref()
                .map(|s| s.start_time.to_string())
                .unwrap_or("No spectrogram".to_string())
        )
    }

    pub fn view(&self, _size: Size, _app: &AppShared) -> Element<'_, Message> {
        if self.shared.spectrogram.is_none() {
            return container(
                button("Open Spectrogram")
                    .style(button::primary)
                    .on_press(Message::PickSpectrogram),
            )
            .center(Length::Fill)
            .into();
        }

        let controls = self.shared.controls.view(&self.shared).map(Message::from);

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

        let plot_area: Element<'_, Message> = widget::stack![spectrogram, plot_overlay].into();

        widget::column![controls, plot_area]
            .padding(8)
            .spacing(4)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
