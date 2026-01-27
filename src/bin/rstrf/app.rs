// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::Config;
use crate::{Args, fl};
use iced::Application;
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{self, text};
use iced::{Element, Length, Program, Subscription, Task, Theme};
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
    rfplot: Option<crate::widgets::rfplot::RFPlot>,
    /// Loaded TLEs
    satellites: Vec<Satellite>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    #[allow(dead_code)]
    UpdateConfig(Config),
    SpectrogramLoaded(Result<Spectrogram, String>),
    SatellitesLoaded(Result<Vec<Satellite>, String>),
    RFPlot(crate::widgets::rfplot::Message),
}

impl From<crate::widgets::rfplot::Message> for Message {
    fn from(msg: crate::widgets::rfplot::Message) -> Self {
        Message::RFPlot(msg)
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
        // Construct the app model with the runtime's core.
        let app = AppModel {
            config: Config::default(),
            spectrogram: None,
            rfplot: None,
            satellites: Vec::new(),
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
            tasks.push(Task::future(async move {
                let satellites: anyhow::Result<_> = async {
                    let freqs = rstrf::orbit::load_frequencies(&freqs_path).await?;
                    rstrf::orbit::load_tles(&path, freqs).await
                }
                .await;
                Message::SatellitesLoaded(satellites.map_err(|e| format!("{e:?}")))
            }));
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

        widget::container(rfplot)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
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
                    self.rfplot = Some(crate::widgets::rfplot::RFPlot::new(
                        self.spectrogram.as_ref().cloned().unwrap(),
                    ));
                    return self.set_rfplot_satellites().map(Message::from);
                }
                Err(err) => log::error!("failed to load spectrogram: {err}"),
            },
            Message::SatellitesLoaded(result) => match result {
                Ok(satellites) => {
                    log::info!("Loaded {} TLEs", satellites.len());
                    self.satellites = satellites;
                    return self.set_rfplot_satellites().map(Message::from);
                }
                Err(err) => log::error!("failed to load TLEs: {err}"),
            },
            Message::RFPlot(message) => match &mut self.rfplot {
                Some(rfplot) => {
                    return rfplot.update(message).map(Message::from);
                }
                None => {
                    log::error!("RFPlot widget not initialized");
                }
            },
        }
        Task::none()
    }

    #[must_use]
    fn set_rfplot_satellites(&mut self) -> Task<crate::widgets::rfplot::Message> {
        match &mut self.rfplot {
            Some(rfplot) => rfplot.update(
                crate::widgets::rfplot::overlay::Message::SetSatellites(self.satellites.clone())
                    .into(),
            ),
            None => Task::none(),
        }
    }
}
