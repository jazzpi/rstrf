// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::Config;
use crate::pass_png::{self, PassPngMode};
use crate::windows::rfplot::{InitialView, RFPlot};
use crate::windows::sat_manager::SatManager;
use crate::windows::{self, AnyWindow};
use crate::{CliArgs, Command, PassPngArgs, PlotArgs};
use anyhow::Context;
use iced::widget::{self, space};
use iced::window::Settings;
use iced::window::settings::PlatformSpecific;
use iced::{Daemon, window};
use iced::{Element, Program, Subscription, Task, Theme};
use rstrf::menu::{MenuItem, view_menu};
use rstrf::orbit::{Satellite, Site};
use rstrf::spectrogram::SpectrogramBounds;
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
    /// Site determined from site_id & STRF's sites.txt
    strf_site: Option<Site>,
    pub satellites: Vec<(Satellite, bool)>,
    pub frequencies: HashMap<u64, Vec<f64>>,
    /// Site ID for saving signals (set from --site-id/-C CLI arg).
    ///
    /// Also used for location if `config.follow_strf_site` is true.
    pub site_id: Option<i32>,
    /// Frequency range to load in Hz (channels outside this range are skipped)
    pub freq_range: Option<(u64, u64)>,
}

impl AppShared {
    pub fn active_satellites(&self) -> Vec<Satellite> {
        self.satellites
            .iter()
            .filter_map(|(sat, active)| active.then(|| sat.clone()))
            .collect()
    }

    pub fn active_satellite_ids(&self) -> Vec<u64> {
        self.satellites
            .iter()
            .filter_map(|(sat, active)| active.then(|| sat.norad_id()))
            .collect()
    }

    pub fn site(&self) -> Option<Site> {
        if self.config.follow_strf_site {
            self.strf_site.clone()
        } else {
            self.config.site.clone()
        }
    }
}

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    config_path: PathBuf,
    shared_state: AppShared,
    windows: HashMap<window::Id, AnyWindow>,
    pass_png: Option<PassPngMode>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    Nop,
    UpdateConfig(Config),
    UpdateStrfSite(Option<Site>),
    // TODO: how will the app restore an rfplot with a given spectrogram/controls?
    OpenRFPlot,
    WindowOpenedRFPlot(window::Id),
    OpenSatManager,
    WindowOpenedSatManager(window::Id),
    OpenPreferences,
    WindowOpenedPreferences(window::Id),
    WindowClosed(window::Id),
    #[allow(clippy::enum_variant_names)]
    WindowMessage(window::Id, windows::Message),
    Event(AppEvent),
    WindowOpenedRFPlotWith(window::Id, Box<PlotArgs>),
    WindowOpenedPassPng(window::Id, Box<PassPngArgs>),
    CatalogLoaded {
        satellites: Vec<(Satellite, bool)>,
        frequencies: HashMap<u64, Vec<f64>>,
    },
    SatellitesChanged(Vec<(Satellite, bool)>),
    SatelliteChanged(usize, Box<(Satellite, bool)>),
    FrequenciesChanged(HashMap<u64, Vec<f64>>),
    RFPlotReady(window::Id, SpectrogramBounds),
    PassPng(pass_png::Message),
    ScreenshotSaved(PathBuf),
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    ConfigUpdated,
    SatellitesChanged,
}

impl AppModel {
    pub fn create(args: CliArgs) -> Daemon<impl Program<Message = Message, Theme = Theme>> {
        iced::daemon(move || Self::init(args.clone()), Self::update, Self::view)
            .subscription(Self::subscription)
            .theme(Self::theme)
            .title(Self::title)
    }

    fn init(flags: CliArgs) -> (Self, Task<Message>) {
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

        match flags.command {
            Some(Command::Plot(args)) => {
                tasks.push(
                    Self::load_catalog(args.catalog.clone(), args.freqs.clone(), HashMap::new())
                        .map(|(satellites, frequencies)| Message::CatalogLoaded {
                            satellites,
                            frequencies,
                        }),
                );
                tasks
                    .push(Self::open_window(None).map(move |id| {
                        Message::WindowOpenedRFPlotWith(id, Box::new(args.clone()))
                    }));
            }
            Some(Command::PassPng(args)) => {
                let norad_id = args.norad_id as u64;
                let frequencies = HashMap::from([(norad_id, args.freq.clone())]);
                tasks.push(
                    Self::load_catalog(Some(args.catalog.clone()), args.freqs.clone(), frequencies)
                        .map(move |(satellites, frequencies)| Message::CatalogLoaded {
                            satellites: satellites
                                .into_iter()
                                .filter(|(sat, _)| sat.norad_id() == norad_id)
                                .collect(),
                            frequencies,
                        }),
                );
                tasks.push(
                    Self::open_window(Some(iced::Size::new(args.width as f32, args.height as f32)))
                        .map(move |id| Message::WindowOpenedPassPng(id, Box::new(args.clone()))),
                );
            }
            None => {
                tasks.push(Task::done(Message::OpenRFPlot));
            }
        }

        let app = AppModel {
            config_path,
            shared_state: AppShared {
                freq_range: flags
                    .freq_range
                    .map(|v| (v[0].round() as u64, v[1].round() as u64)),
                ..Default::default()
            },
            windows: HashMap::default(),
            pass_png: None,
        };

        (app, Task::batch(tasks))
    }

    fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        match self.windows.get(&window_id) {
            Some(window) => {
                let mut menu = Self::workspace_menu();
                menu.extend(
                    window
                        .menu_bar()
                        .into_iter()
                        .map(|item| item.map_msg(|msg| Message::WindowMessage(window_id, msg))),
                );
                let mb = view_menu(menu);
                let content = window
                    .view(&self.shared_state)
                    .map(move |msg| Message::WindowMessage(window_id, msg));
                widget::column![mb, content].into()
            }
            None => space().into(),
        }
    }

    fn workspace_menu() -> Vec<MenuItem<Message>> {
        vec![MenuItem::Submenu {
            label: "Workspace".to_string(),
            msg: Some(Message::Nop),
            items: vec![
                MenuItem::Button {
                    label: "Open new RFPlot window".to_string(),
                    msg: Some(Message::OpenRFPlot),
                },
                MenuItem::Button {
                    label: "Open new SatManager window".to_string(),
                    msg: Some(Message::OpenSatManager),
                },
                MenuItem::Button {
                    label: "Open Preferences".to_string(),
                    msg: Some(Message::OpenPreferences),
                },
            ],
        }]
    }

    /// Register subscriptions for this application.
    ///
    /// Subscriptions are long-running async tasks running in the background which
    /// emit messages to the application through a channel. They can be dynamically
    /// stopped and started conditionally based on application state, or persist
    /// indefinitely.
    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![window::close_events().map(Message::WindowClosed)];
        for (id, window) in &self.windows {
            subscriptions.push(
                window
                    .subscription(&self.shared_state)
                    .with(*id)
                    .map(|(id, msg)| Message::WindowMessage(id, msg)),
            );
        }
        if let Some(m) = &self.pass_png {
            subscriptions.extend(m.subscription().map(|s| s.map(Message::PassPng)));
        }
        Subscription::batch(subscriptions)
    }

    /// Handles messages emitted by the application and its widgets.
    ///
    /// Tasks may be returned for asynchronous execution of code in the background
    /// on the application's async runtime.
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Nop => Task::none(),
            Message::UpdateConfig(config) => self.update_config(config),
            Message::UpdateStrfSite(site) => {
                self.shared_state.strf_site = site;
                Task::done(Message::Event(AppEvent::ConfigUpdated))
            }
            Message::OpenRFPlot => Self::open_window(None).map(Message::WindowOpenedRFPlot),
            Message::WindowOpenedRFPlot(id) => {
                self.windows
                    .insert(id, AnyWindow::RFPlot(Box::new(RFPlot::new())));
                Task::none()
            }
            Message::WindowOpenedRFPlotWith(id, args) => {
                self.shared_state.site_id = args.site_id;
                let view = InitialView {
                    fmin: args.fmin,
                    fmax: args.fmax,
                    tmin: args.tmin,
                    tmax: args.tmax,
                    zmin: args.zmin,
                    zmax: args.zmax,
                };
                let rfplot_task = self.open_rfplot_with(id, args.spectrograms.clone(), view);
                if self.shared_state.config.follow_strf_site {
                    let strf_site_task = self.update_strf_site();
                    Task::batch([rfplot_task, strf_site_task])
                } else {
                    rfplot_task
                }
            }
            Message::WindowOpenedPassPng(id, args) => {
                let view = InitialView {
                    fmin: None,
                    fmax: None,
                    tmin: None,
                    tmax: None,
                    zmin: args.zmin,
                    zmax: args.zmax,
                };
                let task = self.open_rfplot_with(id, args.spectrograms.clone(), view);
                self.pass_png = Some(PassPngMode::new(id, *args));
                task
            }
            Message::CatalogLoaded {
                satellites,
                frequencies,
            } => Task::batch([
                Task::done(Message::SatellitesChanged(satellites)),
                Task::done(Message::FrequenciesChanged(frequencies)),
            ]),
            Message::OpenSatManager => Self::open_window(None).map(Message::WindowOpenedSatManager),
            Message::WindowOpenedSatManager(id) => {
                self.windows
                    .insert(id, AnyWindow::SatManager(Box::new(SatManager::new())));
                Task::none()
            }
            Message::OpenPreferences => {
                Self::open_window(None).map(Message::WindowOpenedPreferences)
            }
            Message::WindowOpenedPreferences(id) => {
                self.windows.insert(
                    id,
                    AnyWindow::Preferences(Box::new(windows::preferences::Window::new(
                        &self.shared_state,
                    ))),
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
                        .update(id, message, &self.shared_state)
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
            Message::SatellitesChanged(sats) => {
                self.shared_state.satellites = sats;
                Task::done(Message::Event(AppEvent::SatellitesChanged))
            }
            Message::SatelliteChanged(idx, data) => {
                log::debug!("SatelliteChanged({}, {:?})", idx, data);
                match self.shared_state.satellites.get_mut(idx) {
                    Some(sat) => *sat = *data,
                    None => log::error!("Got SatelliteChanged for non-existent index {}", idx),
                };
                Task::done(Message::Event(AppEvent::SatellitesChanged))
            }
            Message::FrequenciesChanged(freqs) => {
                self.shared_state
                    .satellites
                    .iter_mut()
                    .for_each(|(sat, _)| {
                        if let Some(freq) = freqs.get(&sat.norad_id()) {
                            sat.transmitters = freq.clone();
                        }
                    });
                self.shared_state.frequencies = freqs;
                Task::done(Message::Event(AppEvent::SatellitesChanged))
            }
            Message::RFPlotReady(window_id, spec_bounds) => {
                let Some(mode) = &mut self.pass_png else {
                    return Task::none();
                };
                mode.update(
                    pass_png::Message::RFPlotReady(window_id, spec_bounds),
                    &self.shared_state,
                )
            }
            Message::ScreenshotSaved(path) => match &mut self.pass_png {
                Some(mode) => {
                    mode.update(pass_png::Message::ScreenshotSaved(path), &self.shared_state)
                }
                None => Task::none(),
            },
            Message::PassPng(msg) => match &mut self.pass_png {
                Some(mode) => mode.update(msg, &self.shared_state),
                None => Task::none(),
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
        if self.shared_state.config.follow_strf_site {
            self.update_strf_site()
        } else {
            Task::done(Message::Event(AppEvent::ConfigUpdated))
        }
    }

    fn update_strf_site(&mut self) -> Task<Message> {
        let site_id = self.shared_state.site_id.or_else(|| {
            std::env::var("ST_COSPAR")
                .ok()
                .and_then(|s| s.parse::<i32>().ok())
        });
        let Some(site_id) = site_id else {
            log::error!("no site ID provided for STRF site lookup");
            return Task::done(Message::Event(AppEvent::ConfigUpdated));
        };
        let sites_path = std::env::var("ST_SITES_TXT")
            .map(PathBuf::from)
            .or_else(|_| {
                std::env::var("ST_DATADIR").map(|dir| {
                    [dir, "data".to_string(), "sites.txt".to_string()]
                        .iter()
                        .collect()
                })
            });
        let sites_path = match sites_path {
            Ok(path) => path,
            Err(err) => {
                log::error!("Failed to determine path to STRF sites.txt: {:?}", err);
                return Task::done(Message::Event(AppEvent::ConfigUpdated));
            }
        };
        Task::future(async move {
            let site = rstrf::orbit::load_strf_sites(&sites_path)
                .await
                .and_then(|sites| {
                    sites
                        .get(&site_id)
                        .cloned()
                        .context(format!("Site ID {} not found in STRF sites.txt", site_id))
                });
            log::debug!("Loaded STRF site with ID {}: {:?}", site_id, site);
            Message::UpdateStrfSite(
                site.inspect_err(|err| {
                    log::error!("Failed to load STRF site with ID {}: {:?}", site_id, err);
                })
                .ok(),
            )
        })
    }

    fn open_window(size: Option<iced::Size<f32>>) -> Task<window::Id> {
        let mut settings = Settings {
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
        };
        if let Some(size) = size {
            settings.size = size;
        }
        let (_, open) = window::open(settings);
        open
    }

    fn open_rfplot_with(
        &mut self,
        id: window::Id,
        spectrograms: Vec<PathBuf>,
        view: InitialView,
    ) -> Task<Message> {
        let rfplot = RFPlot::with_initial_view(spectrograms, view);
        self.windows.insert(id, AnyWindow::RFPlot(Box::new(rfplot)));
        let task = self
            .windows
            .get_mut(&id)
            .unwrap()
            .init(id, &self.shared_state);
        task.map(move |msg| Message::WindowMessage(id, msg))
    }

    fn load_catalog(
        catalog: Option<PathBuf>,
        freqs: Option<PathBuf>,
        initial_freqs: HashMap<u64, Vec<f64>>,
    ) -> Task<(Vec<(Satellite, bool)>, HashMap<u64, Vec<f64>>)> {
        if catalog.is_none() && freqs.is_none() {
            return Task::none();
        }
        Task::future(async move {
            let mut frequencies = initial_freqs;
            if let Some(p) = freqs {
                match rstrf::orbit::load_frequencies(&p).await {
                    Ok(f) => frequencies.extend(f),
                    Err(e) => {
                        log::error!("Failed to load frequencies: {e:?}");
                    }
                }
            }
            let satellites = if let Some(p) = catalog {
                match rstrf::orbit::load_tles(&p, frequencies.clone()).await {
                    Ok(sats) => sats.into_iter().map(|s| (s, true)).collect(),
                    Err(e) => {
                        log::error!("Failed to load catalog: {e:?}");
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            (satellites, frequencies)
        })
    }
}
