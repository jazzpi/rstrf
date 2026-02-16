use std::path::PathBuf;

use iced::{
    Element, Subscription, Task,
    widget::{PaneGrid, button, column, container, pane_grid, responsive, row, text},
};
use iced_aw::{menu_bar, menu_items};
use rfd::AsyncFileDialog;
use rstrf::{
    menu::{checkbox, sublevel, submenu, toplevel, view_menu},
    util::pick_file,
};

use crate::{
    app::{self, AppShared},
    panes::{self, dummy::Dummy},
    widgets::{Icon, icon_button},
    workspace::{self, Workspace},
};

#[derive(Debug, Clone)]
pub enum Message {
    Nop,
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

pub struct Window {
    panes: panes::PaneGridState,
    workspace_path: Option<PathBuf>,
    workspace: Workspace,
}

impl Window {
    pub fn init(path: Option<PathBuf>) -> (Self, Task<super::Message>) {
        let mut tasks: Vec<Task<super::Message>> = Vec::new();

        let (panes, _) = panes::PaneGridState::new(Box::new(panes::dummy::Dummy));

        if let Some(ref path) = path {
            tasks.push(Task::done(Message::WorkspaceDoLoad(path.clone()).into()));
        }

        let mut window = Window {
            panes,
            workspace_path: path,
            workspace: Workspace::default(),
        };
        tasks.push(
            window
                .reset_workspace(&AppShared::default())
                .map(super::Message::Workspace),
        );
        let command = Task::batch(tasks);

        (window, command)
    }

    fn reset_workspace(&mut self, app: &AppShared) -> Task<Message> {
        log::debug!("Loaded workspace");
        let panes = panes::from_workspace(&self.workspace, app);
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

impl super::Window for Window {
    fn title(&self) -> String {
        match &self.workspace_path {
            Some(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into())
                .unwrap_or("Unknown workspace".into()),
            None => "New Workspace".into(),
        }
    }

    fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, super::Message> {
        let mb = view_menu(menu_bar!(
            (
                toplevel("Workspace", Some(Message::Nop.into())),
                submenu(menu_items!(
                    (sublevel(
                        "New window",
                        Some(super::Message::ToApp(Box::new(
                            app::Message::OpenWorkspace(None)
                        )))
                    )),
                    (sublevel("New", Some(Message::WorkspaceNew.into()))),
                    (sublevel("Open", Some(Message::WorkspaceOpen.into()))),
                    (sublevel("Save", Some(Message::WorkspaceSave.into()))),
                    (sublevel("Save as...", Some(Message::WorkspaceSaveAs.into()))),
                    (checkbox(
                        "Auto-save",
                        Some(Message::WorkspaceToggleAutoSave.into()),
                        self.workspace.auto_save
                    ))
                ))
            ),
            (
                toplevel("Edit", Some(Message::Nop.into())),
                submenu(menu_items!(
                    (sublevel(
                        "Preferences",
                        Some(super::Message::ToApp(Box::new(
                            app::Message::OpenPreferences
                        )))
                    )),
                ))
            )
        ));
        let pane_grid: Element<Message> =
            PaneGrid::new(&self.panes, move |id, pane, is_maximized| {
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
                            icon_button(
                                Icon::Close,
                                "Close",
                                Message::ClosePane(id),
                                button::danger
                            ),
                        ]
                        .spacing(5),
                    ))
                    .padding(10)
                    .style(style::title_bar);
                pane_grid::Content::new(
                    container(responsive(move |size| {
                        pane.view(size, &self.workspace.shared, app)
                            .map(move |m| Message::PaneMessage(id, m))
                    }))
                    .padding(2),
                )
                .title_bar(title_bar)
                .style(style::pane)
            })
            .spacing(10)
            .on_drag(Message::PaneDragged)
            .on_resize(10, Message::PaneResized)
            .into();
        column![
            mb,
            container(pane_grid.map(super::Message::Workspace)).padding(4)
        ]
        .into()
    }

    fn update(&mut self, message: super::Message, app: &AppShared) -> Task<super::Message> {
        match message {
            super::Message::Workspace(message) => {
                match message {
                    Message::Nop => (),
                    Message::WorkspaceEvent(event) => {
                        let tasks = self.panes.iter_mut().map(|(id, pane)| {
                            let id = *id;
                            pane.workspace_event(event.clone(), &self.workspace.shared, app)
                                .map(move |m| Message::PaneMessage(id, m).into())
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
                                    .init(&self.workspace.shared, app)
                                    .map(move |msg| Message::PaneMessage(id, msg).into());
                            }
                        }
                        panes::Message::ToWorkspace(message) => {
                            return self
                                .workspace
                                .update(message)
                                .map(|e| Message::WorkspaceEvent(e).into());
                        }
                        panes::Message::ToApp(msg) => {
                            return Task::done(super::Message::ToApp(msg));
                        }
                        _ => match self.panes.get_mut(id) {
                            Some(pane) => {
                                return pane
                                    .update(pane_message, &self.workspace.shared, app)
                                    .map(move |m| Message::PaneMessage(id, m).into());
                            }
                            None => {
                                log::warn!("Received PaneMessage for unknown pane ID {:?}", id);
                            }
                        },
                    },
                    Message::ClosePane(pane) => {
                        if self.panes.len() == 1 {
                            return Task::done(
                                Message::PaneMessage(
                                    pane,
                                    panes::Message::ReplacePane(panes::Pane::Dummy(Box::new(
                                        Dummy,
                                    ))),
                                )
                                .into(),
                            );
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
                            .and_then(|p| Task::done(Message::WorkspaceDoLoad(p).into()));
                    }
                    Message::WorkspaceSave => {
                        if let Some(ref path) = self.workspace_path {
                            return Task::done(Message::WorkspaceDoSave(path.clone()).into());
                        } else {
                            return Task::done(Message::WorkspaceSaveAs.into());
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
                        .and_then(|p| Task::done(Message::WorkspaceDoSave(p).into()));
                    }
                    Message::WorkspaceToggleAutoSave => {
                        self.workspace.auto_save = !self.workspace.auto_save;
                    }
                    Message::WorkspaceDoLoad(path) => {
                        let ws = Workspace::load(path.clone());
                        match ws {
                            Ok(ws) => {
                                self.workspace = ws;
                                self.workspace_path = Some(path);
                                return self.reset_workspace(app).map(super::Message::Workspace);
                            }
                            Err(err) => log::error!("Failed to load workspace: {:?}", err),
                        }
                    }
                    Message::WorkspaceDoSave(path) => {
                        let result = (|| -> anyhow::Result<Task<super::Message>> {
                            self.workspace.panes = panes::to_tree(&self.panes, self.panes.layout())
                                .ok_or(anyhow::anyhow!("Failed to generate pane tree"))?;
                            let json = serde_json::to_string(&self.workspace)?;
                            self.workspace_path = Some(path.clone());
                            Ok(Task::future(async move {
                                match tokio::fs::write(path.clone(), json).await {
                                    Ok(_) => log::debug!("Saved workspace to {path:?}"),
                                    Err(e) => {
                                        log::error!("Failed to save workspace to {path:?}: {e:?}")
                                    }
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
                        return self.reset_workspace(app).map(super::Message::Workspace);
                    }
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    fn app_event(&mut self, event: app::AppEvent, _app: &AppShared) -> Task<super::Message> {
        Task::done(Message::WorkspaceEvent(workspace::Event::App(event)).into())
    }

    fn subscription(&self) -> Subscription<super::Message> {
        if self.workspace.auto_save
            && let Some(ws_path) = self.workspace_path.clone()
        {
            iced::time::every(iced::time::Duration::from_secs(5))
                .with(ws_path)
                .map(|(ws_path, _)| Message::WorkspaceDoSave(ws_path).into())
        } else {
            Subscription::none()
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
