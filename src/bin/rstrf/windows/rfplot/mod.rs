use std::path::PathBuf;

use iced::{
    Element, Length, Padding, Task, keyboard,
    widget::{self, button, container},
};
use plotters_iced2::ChartWidget;
use rfd::AsyncFileDialog;
use rstrf::{coord::plot_area, menu::MenuItem, spectrogram::Spectrogram};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::{AppEvent, AppShared},
    // panes::{Message as PaneMessage, Pane, PaneTree, PaneWidget, rfplot::control::Controls},
    windows::{Window, WindowOut, rfplot::control::Controls},
};

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

#[derive(Serialize, Deserialize, PartialEq, Default, Clone)]
struct SharedState {
    pub controls: Controls,
    pub spectrogram_files: Vec<PathBuf>,
    #[serde(skip)]
    pub spectrogram: Option<Spectrogram>,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
}

#[derive(Serialize, Deserialize, PartialEq, Clone)]
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

    // TODO
    pub fn app_event(&mut self, _event: AppEvent, app: &AppShared) -> Task<WindowOut<Message>> {
        // Trigger a prediction cache check
        self.overlay
            .update(overlay::Message::RefreshCache, &self.shared, app)
            .map(Message::Overlay)
            .map(WindowOut::Msg)
    }
}

impl Window<Message> for RFPlot {
    fn init(&mut self, app: &AppShared) -> Task<WindowOut<Message>> {
        if self.shared.spectrogram_files.is_empty() {
            Task::none()
        } else {
            self.update(
                Message::LoadSpectrogram(self.shared.spectrogram_files.clone()),
                app,
            )
        }
    }

    fn menu_bar(&self) -> Vec<MenuItem<WindowOut<Message>>> {
        vec![MenuItem::Submenu {
            label: "File".to_string(),
            msg: Some(Message::Nop.into()),
            items: vec![MenuItem::Button {
                label: "Load spectrogram(s)".to_string(),
                msg: Some(Message::PickSpectrogram.into()),
            }],
        }]
    }

    fn view(&self, _app: &AppShared) -> Element<'_, WindowOut<Message>> {
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

        let plot_area: Element<'_, Message> = widget::stack![spectrogram, plot_overlay,].into();

        let contents: Element<'_, Message> = widget::column![controls, plot_area]
            .padding(8)
            .spacing(4)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        contents.map(WindowOut::Msg)
    }

    fn update(&mut self, message: Message, app: &AppShared) -> Task<WindowOut<Message>> {
        let result = match message {
            Message::Control(message) => self.shared.controls.update(message).map(Message::Control),
            Message::Overlay(message) => self
                .overlay
                .update(message, &self.shared, app)
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
                        .update(overlay::Message::SpectrogramUpdated, &self.shared, app)
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
        };
        result.map(WindowOut::Msg)
    }

    fn title(&self) -> String {
        format!(
            "Plot: {}",
            self.shared
                .spectrogram
                .as_ref()
                .map(|s| s.start_time.to_string())
                .unwrap_or("Loading...".to_string())
        )
    }
}
