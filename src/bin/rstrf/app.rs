// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::Args;
use crate::config::Config;
use crate::panes::{rfplot::RFPlot, sat_manager::SatManager};
use crate::windows::{self, Window};
use crate::workspace::{WindowSpec, Workspace, WorkspaceShared};
use anyhow::Context;
use iced::widget::space;
use iced::window::Settings;
use iced::window::settings::PlatformSpecific;
use iced::{Daemon, window};
use iced::{Element, Program, Subscription, Task, Theme};
use rfd::AsyncFileDialog;
use rstrf::orbit::Satellite;
use rstrf::util::pick_file;
use space_track::SpaceTrack;
use tokio::sync::Mutex;

/// State that is shared across the entire application, but not persisted in the workspace.
#[derive(Default)]
pub struct AppShared {
    // SpaceTrack saves/refreshes its credentials seamlessly, which means all methods on it require
    // mutable access.
    pub space_track: Option<Arc<Mutex<SpaceTrack>>>,
    /// Configuration data that persists between application runs.
    pub config: Config,
    /// Shared workspace state (satellites, frequencies) — synced from AppModel.workspace.
    pub workspace_shared: WorkspaceShared,
    /// Whether auto-save is enabled
    pub workspace_auto_save: bool,
}

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    config_path: PathBuf,
    shared_state: AppShared,
    windows: HashMap<window::Id, Box<dyn Window>>,
    workspace: Workspace,
    workspace_path: Option<PathBuf>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    UpdateConfig(Config),
    OpenRFPlot(Option<Box<RFPlot>>),
    WindowOpenedRFPlot(window::Id, Option<Box<RFPlot>>),
    OpenSatManager(Option<Box<SatManager>>),
    WindowOpenedSatManager(window::Id, Option<Box<SatManager>>),
    OpenPreferences,
    WindowOpenedPreferences(window::Id),
    WindowClosed(window::Id),
    #[allow(clippy::enum_variant_names)]
    WindowMessage(window::Id, windows::Message),
    Event(AppEvent),
    WorkspaceNew,
    WorkspaceOpen,
    WorkspaceSave,
    WorkspaceSaveAs,
    WorkspaceToggleAutoSave,
    WorkspaceDoLoad(PathBuf),
    WorkspaceDoSave(PathBuf),
    WorkspaceSatellitesChanged(Vec<(Satellite, bool)>),
    WorkspaceSatelliteChanged(usize, Box<(Satellite, bool)>),
    WorkspaceFrequenciesChanged(HashMap<u64, f64>),
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    ConfigUpdated,
    WorkspaceSharedChanged,
}

impl AppModel {
    pub fn create(args: Args) -> Daemon<impl Program<Message = Message, Theme = Theme>> {
        iced::daemon(move || Self::init(args.clone()), Self::update, Self::view)
            .subscription(Self::subscription)
            .theme(Self::theme)
            .title(Self::title)
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

        let (workspace, workspace_path) = if let Some(path) = flags.workspace.clone() {
            match Workspace::load(path.clone()) {
                Ok(ws) => (ws, Some(path)),
                Err(err) => {
                    log::error!("Failed to load workspace: {:?}", err);
                    (Workspace::default(), None)
                }
            }
        } else {
            (Workspace::default(), None)
        };

        let app = AppModel {
            config_path,
            shared_state: AppShared::default(),
            windows: HashMap::default(),
            workspace: workspace.clone(),
            workspace_path,
        };

        // Open a window for each WindowSpec in the loaded workspace
        for spec in &workspace.windows {
            tasks.push(match spec {
                WindowSpec::RFPlot(rfplot) => {
                    let rfplot = rfplot.clone();
                    Self::open_window()
                        .map(move |id| Message::WindowOpenedRFPlot(id, Some(rfplot.clone())))
                }
                WindowSpec::SatManager(sm) => {
                    let sm = sm.clone();
                    Self::open_window()
                        .map(move |id| Message::WindowOpenedSatManager(id, Some(sm.clone())))
                }
            });
        }

        // If no windows were opened (empty workspace), open defaults
        if workspace.windows.is_empty() {
            tasks.push(Self::open_window().map(|id| Message::WindowOpenedRFPlot(id, None)));
            tasks.push(Self::open_window().map(|id| Message::WindowOpenedSatManager(id, None)));
        }

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

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![window::close_events().map(Message::WindowClosed)];

        // Auto-save subscription
        if self.workspace.auto_save
            && let Some(ws_path) = self.workspace_path.clone()
        {
            subscriptions.push(
                iced::time::every(iced::time::Duration::from_secs(5))
                    .with(ws_path)
                    .map(|(ws_path, _)| Message::WorkspaceDoSave(ws_path)),
            );
        }

        subscriptions.extend(self.windows.iter().map(|(id, window)| {
            window
                .subscription()
                .with(*id)
                .map(|(id, msg)| Message::WindowMessage(id, msg))
        }));
        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UpdateConfig(config) => self.update_config(config),
            Message::OpenRFPlot(rfplot) => {
                Self::open_window().map(move |id| Message::WindowOpenedRFPlot(id, rfplot.clone()))
            }
            Message::WindowOpenedRFPlot(id, rfplot) => {
                let rfplot = rfplot.map(|b| *b).unwrap_or_else(RFPlot::new);
                let mut window = windows::rfplot::Window::new(rfplot);
                let task = window
                    .init(&self.shared_state)
                    .map(windows::Message::RFPlot)
                    .map(move |msg| Message::WindowMessage(id, msg));
                self.windows.insert(id, Box::new(window));
                task
            }
            Message::OpenSatManager(sm) => {
                Self::open_window().map(move |id| Message::WindowOpenedSatManager(id, sm.clone()))
            }
            Message::WindowOpenedSatManager(id, sm) => {
                let sm = sm.map(|b| *b).unwrap_or_else(SatManager::new);
                let window = windows::sat_manager::Window::new(sm);
                self.windows.insert(id, Box::new(window));
                Task::none()
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
            Message::Event(app_event) => {
                let tasks = self.windows.iter_mut().map(|(id, window)| {
                    let id = *id;
                    window
                        .app_event(app_event.clone(), &self.shared_state)
                        .map(move |msg| Message::WindowMessage(id, msg))
                });
                Task::batch(tasks)
            }
            Message::WorkspaceNew => {
                self.workspace = Workspace::default();
                self.workspace_path = None;
                self.sync_workspace_shared();
                // Close all non-preferences windows and open fresh defaults
                let to_close: Vec<window::Id> = self.windows.keys().copied().collect();
                let mut tasks: Vec<Task<Message>> =
                    to_close.iter().map(|id| window::close(*id)).collect();
                tasks.push(Self::open_window().map(|id| Message::WindowOpenedRFPlot(id, None)));
                tasks.push(Self::open_window().map(|id| Message::WindowOpenedSatManager(id, None)));
                Task::batch(tasks)
            }
            Message::WorkspaceOpen => Task::future(pick_file(&[("Workspaces", &["json"])]))
                .and_then(|p| Task::done(Message::WorkspaceDoLoad(p))),
            Message::WorkspaceSave => {
                if let Some(ref path) = self.workspace_path {
                    Task::done(Message::WorkspaceDoSave(path.clone()))
                } else {
                    Task::done(Message::WorkspaceSaveAs)
                }
            }
            Message::WorkspaceSaveAs => Task::future(async {
                AsyncFileDialog::new()
                    .add_filter("Workspaces", &["json"])
                    .save_file()
                    .await
                    .map(|f| {
                        let f = f.path();
                        if f.extension().is_none() {
                            f.with_extension("json")
                        } else {
                            f.into()
                        }
                    })
            })
            .and_then(|p| Task::done(Message::WorkspaceDoSave(p))),
            Message::WorkspaceToggleAutoSave => {
                self.workspace.auto_save = !self.workspace.auto_save;
                self.shared_state.workspace_auto_save = self.workspace.auto_save;
                Task::none()
            }
            Message::WorkspaceDoLoad(path) => match Workspace::load(path.clone()) {
                Ok(ws) => {
                    self.workspace = ws;
                    self.workspace_path = Some(path);
                    self.sync_workspace_shared();
                    let to_close: Vec<window::Id> = self.windows.keys().copied().collect();
                    let mut tasks: Vec<Task<Message>> =
                        to_close.iter().map(|id| window::close(*id)).collect();
                    for spec in &self.workspace.windows.clone() {
                        tasks.push(match spec {
                            WindowSpec::RFPlot(rfplot) => {
                                let rfplot = rfplot.clone();
                                Self::open_window().map(move |id| {
                                    Message::WindowOpenedRFPlot(id, Some(rfplot.clone()))
                                })
                            }
                            WindowSpec::SatManager(sm) => {
                                let sm = sm.clone();
                                Self::open_window().map(move |id| {
                                    Message::WindowOpenedSatManager(id, Some(sm.clone()))
                                })
                            }
                        });
                    }
                    Task::batch(tasks)
                }
                Err(err) => {
                    log::error!("Failed to load workspace: {:?}", err);
                    Task::none()
                }
            },
            Message::WorkspaceDoSave(path) => {
                self.capture_workspace_state();
                match serde_json::to_string(&self.workspace) {
                    Ok(json) => {
                        self.workspace_path = Some(path.clone());
                        Task::future(async move {
                            match tokio::fs::write(path.clone(), json).await {
                                Ok(_) => log::debug!("Saved workspace to {path:?}"),
                                Err(e) => {
                                    log::error!("Failed to save workspace to {path:?}: {e:?}")
                                }
                            }
                        })
                        .discard()
                    }
                    Err(err) => {
                        log::error!("Failed to serialize workspace: {:?}", err);
                        Task::none()
                    }
                }
            }
            Message::WorkspaceSatellitesChanged(sats) => {
                self.workspace.shared.satellites = sats;
                self.sync_workspace_shared();
                Task::done(Message::Event(AppEvent::WorkspaceSharedChanged))
            }
            Message::WorkspaceSatelliteChanged(idx, data) => {
                match self.workspace.shared.satellites.get_mut(idx) {
                    Some(sat) => *sat = *data,
                    None => log::error!("Got SatelliteChanged for non-existent index {}", idx),
                };
                self.sync_workspace_shared();
                Task::done(Message::Event(AppEvent::WorkspaceSharedChanged))
            }
            Message::WorkspaceFrequenciesChanged(freqs) => {
                self.workspace
                    .shared
                    .satellites
                    .iter_mut()
                    .for_each(|(sat, _)| {
                        if let Some(freq) = freqs.get(&sat.norad_id()) {
                            sat.tx_freq = *freq;
                        }
                    });
                self.workspace.shared.frequencies = freqs;
                self.sync_workspace_shared();
                Task::done(Message::Event(AppEvent::WorkspaceSharedChanged))
            }
        }
    }

    fn title(&self, window_id: window::Id) -> String {
        let window_title = match self.windows.get(&window_id) {
            Some(window) => window.title(),
            None => "Unknown Window".into(),
        };
        format!("rSTRF - {}", window_title)
    }

    fn theme<State>(&self, _: State) -> Theme {
        self.shared_state.config.theme.into()
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
        Task::done(Message::Event(AppEvent::ConfigUpdated))
    }

    /// Sync AppShared.workspace_shared from the canonical workspace state.
    fn sync_workspace_shared(&mut self) {
        self.shared_state.workspace_shared = self.workspace.shared.clone();
        self.shared_state.workspace_auto_save = self.workspace.auto_save;
    }

    /// Collect the current state of all open windows into workspace.windows for serialization.
    fn capture_workspace_state(&mut self) {
        self.workspace.windows = self
            .windows
            .values()
            .filter_map(|w| w.to_window_spec())
            .collect();
    }

    fn open_window() -> Task<window::Id> {
        let (_, open) = window::open(Settings {
            platform_specific: PlatformSpecific {
                #[cfg(target_os = "linux")]
                application_id: "de.jazzpi.rstrf".into(),
                ..Default::default()
            },
            icon: Some(
                window::icon::from_rgba(
                    include_bytes!(
                        "../../../resources/icons/hicolor/64x64/apps/de.jazzpi.rstrf.rgba"
                    )
                    .into(),
                    64,
                    64,
                )
                .unwrap(),
            ),
            ..Default::default()
        });
        open
    }
}
