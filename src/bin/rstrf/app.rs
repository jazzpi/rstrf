// SPDX-License-Identifier: GPL-3.0-or-later

use crate::Args;
use crate::config::Config;
use crate::panes::rfplot::{self, RFPlot};
use crate::panes::sat_manager::{self, SatManager};
use iced::Application;
use iced::widget::{PaneGrid, pane_grid, responsive, text};
use iced::{Element, Program, Subscription, Task, Theme};

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    #[allow(dead_code)]
    /// Configuration data that persists between application runs.
    config: Config,
    panes: pane_grid::State<Pane>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    #[allow(dead_code)]
    UpdateConfig(Config),
    PaneMessage(pane_grid::Pane, PaneMessage),
}

#[derive(Debug, Clone)]
pub enum PaneMessage {
    RFPlot(rfplot::Message),
    SatManager(sat_manager::Message),
}

impl From<rfplot::Message> for PaneMessage {
    fn from(msg: rfplot::Message) -> Self {
        PaneMessage::RFPlot(msg)
    }
}

impl From<sat_manager::Message> for PaneMessage {
    fn from(msg: sat_manager::Message) -> Self {
        PaneMessage::SatManager(msg)
    }
}

impl AppModel {
    pub fn create(args: Args) -> Application<impl Program<Message = Message, Theme = Theme>> {
        iced::application(move || Self::init(args.clone()), Self::update, Self::view)
            .subscription(Self::subscription)
            .theme(Theme::Dark)
        // TODO
        // .title(Self::title)
        // .font()
        // .presets()
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(flags: Args) -> (Self, Task<Message>) {
        let mut rfplot = RFPlot::new();
        let mut sat_manager = SatManager::new();

        let mut spectrogram_task = Some(
            rfplot
                .update(rfplot::Message::LoadSpectrogram(
                    flags.spectrogram_path.clone(),
                ))
                .map(PaneMessage::from),
        );
        let mut tle_task = flags.tle_path.map(|tle_path| {
            let freqs_path = flags
                .frequencies_path
                .expect("frequencies_path should be present when tle_path is present");
            sat_manager
                .update(sat_manager::Message::LoadTLEs {
                    tle_path,
                    freqs_path,
                })
                .map(PaneMessage::from)
        });

        let panes = pane_grid::State::with_configuration(pane_grid::Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio: 0.7,
            a: Box::new(pane_grid::Configuration::Pane(Pane::RFPlot(rfplot))),
            b: Box::new(pane_grid::Configuration::Pane(Pane::SatManager(
                sat_manager,
            ))),
        });

        let mut tasks: Vec<Task<Message>> = Vec::new();
        for (id, state) in panes.iter() {
            let id = *id;
            match state {
                Pane::RFPlot(_) => {
                    let Some(task) = spectrogram_task else {
                        continue;
                    };
                    tasks.push(task.map(move |m| Message::PaneMessage(id, m)));
                    spectrogram_task = None;
                }
                Pane::SatManager(_) => {
                    let Some(task) = tle_task else {
                        continue;
                    };
                    tasks.push(task.map(move |m| Message::PaneMessage(id, m)));
                    tle_task = None;
                }
            }
        }

        let command = Task::batch(tasks);
        let app = AppModel {
            config: Config::default(),
            panes,
        };

        (app, command)
    }

    /// Describes the interface based on the current state of the application model.
    ///
    /// Application events will be processed through the view. Any messages emitted by
    /// events received by widgets will be passed to the update method.
    fn view(&self) -> Element<'_, Message> {
        let pane_grid = PaneGrid::new(&self.panes, move |id, pane, _is_maximized| {
            let title = text(pane.title());
            let title_bar = pane_grid::TitleBar::new(title);
            pane_grid::Content::new(responsive(move |size| pane.view(id, size)))
                .title_bar(title_bar)
        });
        pane_grid.into()
    }

    /// Register subscriptions for this application.
    ///
    /// Subscriptions are long-running async tasks running in the background which
    /// emit messages to the application through a channel. They can be dynamically
    /// stopped and started conditionally based on application state, or persist
    /// indefinitely.
    fn subscription(&self) -> Subscription<Message> {
        Subscription::none()
    }

    /// Handles messages emitted by the application and its widgets.
    ///
    /// Tasks may be returned for asynchronous execution of code in the background
    /// on the application's async runtime.
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UpdateConfig(config) => {
                self.config = config;
            }
            Message::PaneMessage(id, pane_message) => {
                let mut tasks = self.forward_updates(&pane_message);

                match self.panes.get_mut(id) {
                    Some(pane) => tasks.push(
                        pane.update(pane_message)
                            .map(move |m| Message::PaneMessage(id, m)),
                    ),
                    None => log::warn!("Received PaneMessage for unknown pane ID {:?}", id),
                }

                return Task::batch(tasks);
            }
        }
        Task::none()
    }

    fn forward_updates(&mut self, pane_message: &PaneMessage) -> Vec<Task<Message>> {
        match pane_message {
            PaneMessage::SatManager(sat_manager::Message::SatellitesChanged(satellites)) => self
                .panes
                .iter_mut()
                .filter_map(|(id, p)| {
                    let id = *id;
                    match p {
                        Pane::RFPlot(rfplot) => Some(
                            rfplot
                                .update(
                                    rfplot::overlay::Message::SetSatellites(satellites.clone())
                                        .into(),
                                )
                                .map(move |m| Message::PaneMessage(id, m.into())),
                        ),
                        _ => None,
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum Pane {
    RFPlot(RFPlot),
    SatManager(SatManager),
}

impl Pane {
    fn title(&self) -> &str {
        match self {
            Pane::RFPlot(_) => "Plot",
            Pane::SatManager(_) => "Satellites",
        }
    }

    fn view(&self, id: pane_grid::Pane, _size: iced::Size) -> Element<'_, Message> {
        match self {
            Pane::RFPlot(rfplot) => rfplot
                .view()
                .map(move |msg| Message::PaneMessage(id, PaneMessage::RFPlot(msg))),
            Pane::SatManager(sat_manager) => sat_manager
                .view()
                .map(move |msg| Message::PaneMessage(id, PaneMessage::SatManager(msg))),
        }
    }

    fn update(&mut self, message: PaneMessage) -> Task<PaneMessage> {
        match self {
            Pane::RFPlot(rfplot) => match message {
                PaneMessage::RFPlot(msg) => rfplot.update(msg).map(PaneMessage::from),
                _ => {
                    log::warn!("Received incompatible PaneMessage for RFPlot pane");
                    Task::none()
                }
            },
            Pane::SatManager(sat_manager) => match message {
                PaneMessage::SatManager(msg) => sat_manager.update(msg).map(PaneMessage::from),
                _ => {
                    log::warn!("Received incompatible PaneMessage for SatManager pane");
                    Task::none()
                }
            },
        }
    }
}
