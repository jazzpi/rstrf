// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::Config;
use crate::widgets::rfplot::{self, RFPlot};
use crate::widgets::sat_manager::{self, SatManager};
use crate::{Args, fl};
use iced::Application;
use iced::alignment::Horizontal;
use iced::widget::text;
use iced::{Element, Program, Subscription, Task, Theme};
use iced_aw::Tabs;
use rstrf::orbit::Satellite;
use rstrf::spectrogram::Spectrogram;

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    #[allow(dead_code)]
    /// Configuration data that persists between application runs.
    config: Config,
    /// Spectrogram for plotting
    spectrogram: Option<Spectrogram>,
    /// RFPlot widget
    rfplot: Option<RFPlot>,
    /// SatManager widget
    sat_manager: SatManager,
    active_tab: TabId,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    #[allow(dead_code)]
    UpdateConfig(Config),
    SpectrogramLoaded(Result<Spectrogram, String>),
    RFPlot(rfplot::Message),
    SatManager(sat_manager::Message),
    TabSelected(TabId),
}

impl From<rfplot::Message> for Message {
    fn from(msg: rfplot::Message) -> Self {
        Message::RFPlot(msg)
    }
}

impl From<sat_manager::Message> for Message {
    fn from(msg: sat_manager::Message) -> Self {
        Message::SatManager(msg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabId {
    #[default]
    RFPlot,
    SatManager,
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
        // Construct the app model with the runtime's core.
        let mut app = AppModel {
            config: Config::default(),
            spectrogram: None,
            rfplot: None,
            sat_manager: SatManager::new(),
            active_tab: TabId::default(),
        };

        let spectrogram = Task::future(async move {
            let spec = rstrf::spectrogram::load(&flags.spectrogram_path).await;
            Message::SpectrogramLoaded(spec.map_err(|e| format!("{e:?}")))
        });
        let mut tasks = vec![spectrogram];
        if let Some(path) = flags.tle_path {
            let freqs_path = flags
                .frequencies_path
                .expect("frequencies_path should be present when tle_path is present");
            tasks.push(
                app.sat_manager
                    .update(sat_manager::Message::LoadTLEs {
                        tle_path: path.clone(),
                        freqs_path: freqs_path.clone(),
                    })
                    .map(Message::from),
            );
        }
        let command = Task::batch(tasks);

        (app, command)
    }

    /// Describes the interface based on the current state of the application model.
    ///
    /// Application events will be processed through the view. Any messages emitted by
    /// events received by widgets will be passed to the update method.
    fn view(&self) -> Element<'_, Message> {
        let rfplot = match &self.rfplot {
            Some(rfplot) => rfplot.view().map(Message::RFPlot),
            None => text(fl!("loading-spectrogram"))
                .align_x(Horizontal::Center)
                .into(),
        };

        Tabs::new(Message::TabSelected)
            .push(TabId::RFPlot, fl!("tab-rfplot").into(), rfplot)
            .push(
                TabId::SatManager,
                fl!("tab-satellites").into(),
                self.sat_manager.view().map(Message::SatManager),
            )
            .set_active_tab(&self.active_tab)
            .tab_bar_position(iced_aw::TabBarPosition::Top)
            .into()
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
            Message::SpectrogramLoaded(result) => match result {
                Ok(spec) => {
                    log::info!("Loaded spectrogram: {spec:?}");
                    self.spectrogram = Some(spec);
                    let mut rfplot = RFPlot::new(self.spectrogram.as_ref().cloned().unwrap());
                    // TODO
                    let task = Self::set_rfplot_satellites(
                        &mut rfplot,
                        self.sat_manager.satellites().to_vec(),
                    )
                    .map(Message::from);
                    self.rfplot = Some(rfplot);
                    return task;
                }
                Err(err) => log::error!("failed to load spectrogram: {err}"),
            },
            Message::RFPlot(message) => match &mut self.rfplot {
                Some(rfplot) => {
                    return rfplot.update(message).map(Message::from);
                }
                None => {
                    log::error!("RFPlot widget not initialized");
                }
            },
            Message::SatManager(message) => match message {
                sat_manager::Message::SatellitesChanged(satellites) => match &mut self.rfplot {
                    Some(rfplot) => {
                        return Self::set_rfplot_satellites(rfplot, satellites).map(Message::from);
                    }
                    None => (),
                },
                _ => return self.sat_manager.update(message).map(Message::from),
            },
            Message::TabSelected(tab_id) => self.active_tab = tab_id,
        }
        Task::none()
    }

    #[must_use]
    fn set_rfplot_satellites(
        rfplot: &mut RFPlot,
        satellites: Vec<Satellite>,
    ) -> Task<rfplot::Message> {
        rfplot.update(rfplot::overlay::Message::SetSatellites(satellites).into())
    }
}
