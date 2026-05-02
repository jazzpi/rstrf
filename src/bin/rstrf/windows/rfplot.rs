use iced::{Element, Length, Task, widget::column, widget::responsive};
use iced_aw::{menu_bar, menu_items};
use rstrf::menu::{checkbox, sublevel, submenu, toplevel, view_menu};

use crate::{
    app::{self, AppEvent, AppShared},
    panes::rfplot::{self, overlay, RFPlot},
    workspace::WindowSpec,
};

pub struct Window {
    rfplot: RFPlot,
}

impl Window {
    pub fn new(rfplot: RFPlot) -> Self {
        Self { rfplot }
    }

    pub fn init(&mut self, app: &AppShared) -> Task<rfplot::Message> {
        self.rfplot.init(app)
    }

    fn menu_bar<'a>(&'a self, app: &'a AppShared) -> Element<'a, super::Message> {
        view_menu(menu_bar!(
            (
                toplevel("Workspace", None),
                submenu(menu_items!(
                    (sublevel(
                        "New RFPlot window",
                        Some(super::Message::ToApp(Box::new(app::Message::OpenRFPlot(None))))
                    )),
                    (sublevel(
                        "New SatManager window",
                        Some(super::Message::ToApp(Box::new(
                            app::Message::OpenSatManager(None)
                        )))
                    )),
                    (sublevel(
                        "New workspace",
                        Some(super::Message::ToApp(Box::new(app::Message::WorkspaceNew)))
                    )),
                    (sublevel(
                        "Open...",
                        Some(super::Message::ToApp(Box::new(app::Message::WorkspaceOpen)))
                    )),
                    (sublevel(
                        "Save",
                        Some(super::Message::ToApp(Box::new(app::Message::WorkspaceSave)))
                    )),
                    (sublevel(
                        "Save as...",
                        Some(super::Message::ToApp(Box::new(app::Message::WorkspaceSaveAs)))
                    )),
                    (checkbox(
                        "Auto-save",
                        Some(super::Message::ToApp(Box::new(
                            app::Message::WorkspaceToggleAutoSave
                        ))),
                        app.workspace_auto_save
                    )),
                    (sublevel(
                        "Preferences",
                        Some(super::Message::ToApp(Box::new(app::Message::OpenPreferences)))
                    ))
                ))
            ),
            (
                toplevel("File", None),
                submenu(menu_items!((sublevel(
                    "Load spectrogram(s)",
                    Some(super::Message::RFPlot(rfplot::Message::PickSpectrogram))
                ))))
            )
        ))
    }
}

impl super::Window for Window {
    fn title(&self) -> String {
        self.rfplot.title()
    }

    fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, super::Message> {
        let mb = self.menu_bar(app);
        let content = responsive(move |size| {
            self.rfplot.view(size, app).map(super::Message::RFPlot)
        });
        column![mb, content].height(Length::Fill).into()
    }

    fn update(&mut self, msg: super::Message, app: &AppShared) -> Task<super::Message> {
        if let super::Message::RFPlot(m) = msg {
            self.rfplot.update(m, app).map(super::Message::RFPlot)
        } else {
            Task::none()
        }
    }

    fn app_event(&mut self, event: AppEvent, app: &AppShared) -> Task<super::Message> {
        if let AppEvent::WorkspaceSharedChanged = event {
            self.rfplot
                .update(
                    rfplot::Message::Overlay(overlay::Message::RefreshCache),
                    app,
                )
                .map(super::Message::RFPlot)
        } else {
            Task::none()
        }
    }

    fn to_window_spec(&self) -> Option<WindowSpec> {
        Some(WindowSpec::RFPlot(Box::new(self.rfplot.clone())))
    }
}
