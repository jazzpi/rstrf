use std::path::PathBuf;

use iced::{
    Element, Length, Padding, Size, Task,
    widget::{self, button, container},
};
use iced_aw::{menu_bar, menu_items};
use plotters_iced2::ChartWidget;
use rfd::AsyncFileDialog;
use rstrf::{
    coord::plot_area,
    menu::{button_f, button_s, submenu, view_menu},
    spectrogram::Spectrogram,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::WorkspaceEvent,
    panes::{Message as PaneMessage, Pane, PaneTree, PaneWidget, rfplot::control::Controls},
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
pub enum MouseInteraction {
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
    #[serde(default = "Uuid::new_v4")]
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
}

impl PaneWidget for RFPlot {
    fn init(&mut self) -> Task<PaneMessage> {
        if self.shared.spectrogram_files.is_empty() {
            Task::none()
        } else {
            // TODO: This resets the power bounds after loading the spectrogram
            self.update(Message::LoadSpectrogram(self.shared.spectrogram_files.clone()).into())
        }
    }

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
                    Message::SpectrogramLoaded(
                        spec.map(|s| (paths, s)).map_err(|e| format!("{e:?}")),
                    )
                    .into()
                }),
                Message::SpectrogramLoaded(result) => match result {
                    Ok((paths, spec)) => {
                        log::info!("Loaded spectrogram: {spec:?}");
                        self.shared.controls.set_power_bounds(spec.power_bounds);
                        self.shared.spectrogram = Some(spec);
                        self.shared.spectrogram_files = paths;
                        self.overlay
                            .update(overlay::Message::SpectrogramUpdated, &self.shared)
                            .map(|m| PaneMessage::RFPlot(m.into()))
                    }
                    Err(err) => {
                        log::error!("Failed to load spectrogram: {err}");
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

        let mb = view_menu(menu_bar!((
            button_s("Spectrogram", None),
            submenu(menu_items!(
                (button_f("Load file(s)", Some(Message::PickSpectrogram))),
            ))
        )));
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

        let contents: Element<'_, Message> = widget::column![plot_area, controls]
            .padding(10)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        let result: Element<'_, Message> = widget::column![mb, contents]
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        result.map(PaneMessage::from)
    }

    fn title(&self) -> &str {
        "Plot"
    }

    fn to_tree(&self) -> PaneTree {
        PaneTree::Leaf(Pane::RFPlot(self.clone()))
    }
}
