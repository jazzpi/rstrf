// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::Config;
use crate::panes::dummy::Dummy;
use crate::widgets::{Icon, icon_button};
use crate::workspace::{self, Workspace};
use crate::{Args, panes};
use iced::Application;
use iced::widget::{PaneGrid, button, column, pane_grid, responsive, row, text};
use iced::window::Settings;
use iced::window::settings::PlatformSpecific;
use iced::{Element, Program, Subscription, Task, Theme};
use iced_aw::{menu_bar, menu_items};
use rfd::AsyncFileDialog;
use rstrf::menu::{button_f, button_s, checkbox, submenu, view_menu};
use rstrf::util::pick_file;
use std::fmt::Debug;
use std::path::PathBuf;

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    #[allow(dead_code)]
    /// Configuration data that persists between application runs.
    config: Config,
    panes: panes::PaneGridState,
    workspace_path: Option<PathBuf>,
    workspace: Workspace,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    #[allow(dead_code)]
    UpdateConfig(Config),
    #[allow(clippy::enum_variant_names)]
    PaneMessage(pane_grid::Pane, panes::Message),
    ClosePane(pane_grid::Pane),
    ToggleMaximizePane(pane_grid::Pane),
    SplitPane(pane_grid::Pane, pane_grid::Axis),
    PaneDragged(pane_grid::DragEvent),
    PaneResized(pane_grid::ResizeEvent),
    WorkspaceEvent(workspace::Event),
    WorkspaceNew,
    WorkspaceOpen,
    WorkspaceSave,
    WorkspaceSaveAs,
    WorkspaceToggleAutoSave,
    WorkspaceDoLoad(PathBuf),
    WorkspaceDoSave(PathBuf),
}

impl AppModel {
    pub fn create(args: Args) -> Application<impl Program<Message = Message, Theme = Theme>> {
        iced::application(move || Self::init(args.clone()), Self::update, Self::view)
            .subscription(Self::subscription)
            .theme(Theme::Dark)
            .title(Self::title)
            .window(Settings {
                platform_specific: PlatformSpecific {
                    application_id: "de.jazzpi.rstrf".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
        // TODO
        // .font()
        // .presets()
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(flags: Args) -> (Self, Task<Message>) {
        let (panes, _) = panes::PaneGridState::new(Box::new(panes::dummy::Dummy));

        let mut tasks: Vec<Task<Message>> = Vec::new();
        if let Some(ref path) = flags.workspace {
            tasks.push(Task::done(Message::WorkspaceDoLoad(path.clone())));
        }

        let mut app = AppModel {
            config: Config::default(),
            panes,
            workspace_path: flags.workspace,
            workspace: Workspace::default(),
        };
        tasks.push(app.reset_workspace());
        let command = Task::batch(tasks);

        (app, command)
    }

    /// Describes the interface based on the current state of the application model.
    ///
    /// Application events will be processed through the view. Any messages emitted by
    /// events received by widgets will be passed to the update method.
    fn view(&self) -> Element<'_, Message> {
        let mb = view_menu(menu_bar!((
            button_s("Workspace", None),
            submenu(menu_items!(
                (button_f("New", Some(Message::WorkspaceNew))),
                (button_f("Open", Some(Message::WorkspaceOpen))),
                (button_f("Save", Some(Message::WorkspaceSave))),
                (button_f("Save as...", Some(Message::WorkspaceSaveAs))),
                (checkbox(
                    "Auto-save",
                    Some(Message::WorkspaceToggleAutoSave),
                    self.workspace.auto_save
                ))
            ))
        )));
        let pane_grid = PaneGrid::new(&self.panes, move |id, pane, is_maximized| {
            let title = text(pane.title());
            let title_bar = pane_grid::TitleBar::new(title)
                .controls(pane_grid::Controls::new(
                    row![
                        icon_button(
                            Icon::SplitHorizontally,
                            "Split horizontally",
                            Message::SplitPane(id, pane_grid::Axis::Horizontal),
                            button::secondary
                        ),
                        icon_button(
                            Icon::SplitVertically,
                            "Split vertically",
                            Message::SplitPane(id, pane_grid::Axis::Vertical),
                            button::secondary
                        ),
                        icon_button(
                            if is_maximized {
                                Icon::Restore
                            } else {
                                Icon::Maximize
                            },
                            "Maximize",
                            Message::ToggleMaximizePane(id),
                            button::primary
                        ),
                        icon_button(Icon::Close, "Close", Message::ClosePane(id), button::danger),
                    ]
                    .spacing(5),
                ))
                .padding(10)
                .style(style::title_bar);
            pane_grid::Content::new(responsive(move |size| {
                pane.view(size, &self.workspace.shared)
                    .map(move |m| Message::PaneMessage(id, m))
            }))
            .title_bar(title_bar)
            .style(style::pane)
        })
        .spacing(10)
        .on_drag(Message::PaneDragged)
        .on_resize(10, Message::PaneResized);
        column![mb, pane_grid].into()
    }

    /// Register subscriptions for this application.
    ///
    /// Subscriptions are long-running async tasks running in the background which
    /// emit messages to the application through a channel. They can be dynamically
    /// stopped and started conditionally based on application state, or persist
    /// indefinitely.
    fn subscription(&self) -> Subscription<Message> {
        if self.workspace.auto_save
            && let Some(ws_path) = self.workspace_path.clone()
        {
            iced::time::every(iced::time::Duration::from_secs(5))
                .with(ws_path)
                .map(|(ws_path, _)| Message::WorkspaceDoSave(ws_path))
        } else {
            Subscription::none()
        }
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
            Message::WorkspaceEvent(event) => {
                let tasks = self.panes.iter_mut().map(|(id, pane)| {
                    let id = *id;
                    pane.workspace_event(event.clone(), &self.workspace.shared)
                        .map(move |m| Message::PaneMessage(id, m))
                });
                return Task::batch(tasks);
            }
            Message::PaneMessage(id, pane_message) => match pane_message {
                panes::Message::ReplacePane(new_pane) => {
                    if let Some(pane) = self.panes.get_mut(id) {
                        *pane = match new_pane {
                            panes::Pane::RFPlot(inner) => inner.clone(),
                            panes::Pane::SatManager(inner) => inner.clone(),
                            panes::Pane::Dummy(inner) => inner.clone(),
                        };
                        return pane
                            .init(&self.workspace.shared)
                            .map(move |msg| Message::PaneMessage(id, msg));
                    }
                }
                panes::Message::ToWorkspace(message) => {
                    return self.workspace.update(message).map(Message::WorkspaceEvent);
                }
                _ => match self.panes.get_mut(id) {
                    Some(pane) => {
                        return pane
                            .update(pane_message, &self.workspace.shared)
                            .map(move |m| Message::PaneMessage(id, m));
                    }
                    None => {
                        log::warn!("Received PaneMessage for unknown pane ID {:?}", id);
                    }
                },
            },
            Message::ClosePane(pane) => {
                if self.panes.len() == 1 {
                    return Task::done(Message::PaneMessage(
                        pane,
                        panes::Message::ReplacePane(panes::Pane::Dummy(Box::new(Dummy))),
                    ));
                }
                if self.panes.close(pane).is_none() {
                    log::warn!("Tried to close unknown pane {:?}", pane);
                    return Task::none();
                };
            }
            Message::ToggleMaximizePane(pane) => {
                if self.panes.maximized().is_some() {
                    self.panes.restore();
                } else {
                    self.panes.maximize(pane);
                }
            }
            Message::SplitPane(pane, axis) => {
                self.panes.split(axis, pane, Box::new(Dummy));
            }
            Message::PaneDragged(pane_grid::DragEvent::Dropped { pane, target }) => {
                self.panes.drop(pane, target);
            }
            Message::PaneDragged(_) => (),
            Message::PaneResized(ev) => {
                self.panes.resize(ev.split, ev.ratio);
            }
            Message::WorkspaceOpen => {
                return Task::future(pick_file(&[("Workspaces", &["json"])]))
                    .and_then(|p| Task::done(Message::WorkspaceDoLoad(p)));
            }
            Message::WorkspaceSave => {
                if let Some(ref path) = self.workspace_path {
                    return Task::done(Message::WorkspaceDoSave(path.clone()));
                } else {
                    return Task::done(Message::WorkspaceSaveAs);
                }
            }
            Message::WorkspaceSaveAs => {
                return Task::future(async {
                    let path = AsyncFileDialog::new()
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
                        });
                    log::debug!("Picked workspace file for saving: {:?}", path);
                    path
                })
                .and_then(|p| Task::done(Message::WorkspaceDoSave(p)));
            }
            Message::WorkspaceToggleAutoSave => {
                self.workspace.auto_save = !self.workspace.auto_save;
            }
            Message::WorkspaceDoLoad(path) => {
                let ws = Workspace::load(path);
                match ws {
                    Ok(ws) => {
                        self.workspace = ws;
                        return self.reset_workspace();
                    }
                    Err(err) => log::error!("Failed to load workspace: {:?}", err),
                }
            }
            Message::WorkspaceDoSave(path) => {
                let result = (|| -> anyhow::Result<Task<Message>> {
                    self.workspace.panes = panes::to_tree(&self.panes, self.panes.layout())
                        .ok_or(anyhow::anyhow!("Failed to generate pane tree"))?;
                    let json = serde_json::to_string(&self.workspace)?;
                    self.workspace_path = Some(path.clone());
                    Ok(Task::future(async move {
                        match tokio::fs::write(path.clone(), json).await {
                            Ok(_) => log::debug!("Saved workspace to {path:?}"),
                            Err(e) => log::error!("Failed to save workspace to {path:?}: {e:?}"),
                        }
                    })
                    .discard())
                })();
                match result {
                    Ok(task) => return task,
                    Err(err) => log::error!("Failed to save workspace: {:?}", err),
                }
            }
            Message::WorkspaceNew => {
                self.workspace_path = None;
                self.workspace = Workspace::default();
                return self.reset_workspace();
            }
        }
        Task::none()
    }

    fn title(&self) -> String {
        "rSTRF".into()
    }

    fn reset_workspace(&mut self) -> Task<Message> {
        log::debug!("Loaded workspace");
        let panes = panes::from_workspace(&self.workspace);
        match panes {
            Ok((state, task)) => {
                self.panes = state;
                task.map(|msg| Message::PaneMessage(msg.id, msg.message))
            }
            Err(err) => {
                log::error!("Failed to generate panes from workspace: {:?}", err);
                Task::none()
            }
        }
    }
}

mod style {
    use iced::{Border, Theme, widget::container::Style};

    pub fn title_bar(theme: &Theme) -> Style {
        let palette = theme.extended_palette();
        Style {
            text_color: Some(palette.background.strong.text),
            background: Some(palette.background.strong.color.into()),
            ..Style::default()
        }
    }

    pub fn pane(theme: &Theme) -> Style {
        let palette = theme.extended_palette();
        Style {
            background: Some(palette.background.weak.color.into()),
            border: Border {
                width: 2.0,
                color: palette.background.strong.color,
                ..Border::default()
            },
            ..Style::default()
        }
    }
}
