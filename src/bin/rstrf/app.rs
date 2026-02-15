// SPDX-License-Identifier: GPL-3.0-or-later

use crate::Args;
use crate::config::Config;
use crate::windows::{self, Window};
use anyhow::Context;
use iced::widget::space;
use iced::window::Settings;
use iced::window::settings::PlatformSpecific;
use iced::{Daemon, window};
use iced::{Element, Program, Subscription, Task, Theme};
use space_track::SpaceTrack;
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// State that is shared across the entire application, but not persisted in the workspace.
#[derive(Default)]
pub struct AppShared {
    // SpaceTrack saves/refreshes its credentials seamlessly, which means all methods on it require
    // mutable access.
    pub space_track: Option<Arc<Mutex<SpaceTrack>>>,
    /// Configuration data that persists between application runs.
    pub config: Config,
}

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    config_path: PathBuf,
    shared_state: AppShared,
    windows: HashMap<window::Id, Box<dyn Window>>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    UpdateConfig(Config),
    OpenWorkspace(Option<PathBuf>),
    WindowOpenedWorkspace(window::Id, Option<PathBuf>),
    OpenPreferences,
    WindowOpenedPreferences(window::Id),
    WindowClosed(window::Id),
    #[allow(clippy::enum_variant_names)]
    WindowMessage(window::Id, windows::Message),
}

impl AppModel {
    pub fn create(args: Args) -> Daemon<impl Program<Message = Message, Theme = Theme>> {
        iced::daemon(move || Self::init(args.clone()), Self::update, Self::view)
            .subscription(Self::subscription)
            .theme(Theme::Dark)
            .title(Self::title)
        // TODO
        // .font()
        // .presets()
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(flags: Args) -> (Self, Task<Message>) {
        let mut tasks: Vec<Task<Message>> = Vec::new();

        let config_path = match dirs::config_dir() {
            Some(mut path) => {
                path.push("rstrf");
                match std::fs::create_dir_all(&path) {
                    Ok(_) => {
                        path.push("config.json");
                        path
                    }
                    Err(err) => {
                        log::error!("Failed to create config directory {:?}: {:?}", path, err);
                        "/dev/null".into()
                    }
                }
            }
            None => {
                log::error!("Failed to get config directory");
                "/dev/null".into()
            }
        };

        let config = match Self::load_config(&config_path) {
            Ok(config) => config,
            Err(err) => {
                log::error!("Failed to load config: {:?}", err);
                Config::default()
            }
        };
        tasks.push(Task::done(Message::UpdateConfig(config)));
        tasks.push(Task::done(Message::OpenWorkspace(flags.workspace.clone())));

        let app = AppModel {
            config_path,
            shared_state: AppShared::default(),
            windows: HashMap::default(),
        };

        (app, Task::batch(tasks))
    }

    fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        match self.windows.get(&window_id) {
            Some(window) => window
                .view(&self.shared_state)
                .map(move |msg| Message::WindowMessage(window_id, msg)),
            None => space().into(),
        }
    }

    /// Register subscriptions for this application.
    ///
    /// Subscriptions are long-running async tasks running in the background which
    /// emit messages to the application through a channel. They can be dynamically
    /// stopped and started conditionally based on application state, or persist
    /// indefinitely.
    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![window::close_events().map(Message::WindowClosed)];
        subscriptions.extend(self.windows.iter().map(|(id, window)| {
            window
                .subscription()
                .with(*id)
                .map(|(id, msg)| Message::WindowMessage(id, msg))
        }));
        Subscription::batch(subscriptions)
    }

    /// Handles messages emitted by the application and its widgets.
    ///
    /// Tasks may be returned for asynchronous execution of code in the background
    /// on the application's async runtime.
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UpdateConfig(config) => self.update_config(config),
            Message::OpenWorkspace(path) => {
                Self::open_window().map(move |id| Message::WindowOpenedWorkspace(id, path.clone()))
            }
            Message::WindowOpenedWorkspace(id, path_buf) => {
                let (window, task) = windows::workspace::Window::init(path_buf);
                self.windows.insert(id, Box::new(window));
                task.map(move |msg| Message::WindowMessage(id, msg))
            }
            Message::OpenPreferences => Self::open_window().map(Message::WindowOpenedPreferences),
            Message::WindowOpenedPreferences(id) => {
                self.windows.insert(
                    id,
                    Box::new(windows::preferences::Window::new(&self.shared_state)),
                );
                Task::none()
            }
            Message::WindowClosed(id) => {
                self.windows.remove(&id);
                if self.windows.is_empty() {
                    iced::exit()
                } else {
                    Task::none()
                }
            }
            Message::WindowMessage(id, message) => match message {
                windows::Message::ToApp(message) => self.update(*message),
                _ => match self.windows.get_mut(&id) {
                    Some(window) => window
                        .update(message, &self.shared_state)
                        .map(move |msg| Message::WindowMessage(id, msg)),
                    None => {
                        log::warn!(
                            "Received message for unknown window {:?}: {:?}",
                            id,
                            message
                        );
                        Task::none()
                    }
                },
            },
        }
    }

    fn title(&self, window_id: window::Id) -> String {
        let window_title = match self.windows.get(&window_id) {
            Some(window) => window.title(),
            None => "Unknown Window".into(),
        };
        format!("rSTRF - {}", window_title)
    }

    fn load_config(path: &PathBuf) -> anyhow::Result<Config> {
        let reader =
            std::fs::File::open(path).context(format!("Failed to open config file: {:?}", path))?;
        let config = serde_json::from_reader(reader)
            .context(format!("Failed to parse config file: {:?}", path))?;
        Ok(config)
    }

    fn save_config(&self) -> anyhow::Result<()> {
        let json = serde_json::to_string(&self.shared_state.config)?;
        std::fs::write(&self.config_path, json).context(format!(
            "Failed to write config file: {:?}",
            self.config_path
        ))?;
        Ok(())
    }

    fn update_config(&mut self, config: Config) -> Task<Message> {
        self.shared_state.space_track = config.space_track_creds.as_ref().map(|(user, pass)| {
            Arc::new(Mutex::new(SpaceTrack::new(space_track::Credentials {
                identity: user.clone(),
                password: pass.clone(),
            })))
        });
        self.shared_state.config = config;
        match self.save_config() {
            Ok(_) => log::debug!("Saved config"),
            Err(err) => log::error!("Failed to save config: {:?}", err),
        }
        Task::none()
    }

    fn open_window() -> Task<window::Id> {
        let (_, open) = window::open(Settings {
            platform_specific: PlatformSpecific {
                application_id: "de.jazzpi.rstrf".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        open
    }
}
