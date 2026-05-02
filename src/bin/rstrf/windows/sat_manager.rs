use iced::{Element, Length, Task, widget::column};
use iced_aw::{menu_bar, menu_items};
use rstrf::menu::{checkbox, sublevel, submenu, toplevel, view_menu};

use crate::{
    app::{self, AppShared},
    panes::sat_manager::{Message as SatMsg, Out, SatManager},
    workspace::WindowSpec,
};

pub struct Window {
    sat_manager: SatManager,
}

impl Window {
    pub fn new(sat_manager: SatManager) -> Self {
        Self { sat_manager }
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
                submenu(menu_items!(
                    (sublevel(
                        "Load TLEs",
                        Some(super::Message::SatManager(SatMsg::LoadTLEs))
                    )),
                    (sublevel(
                        "Load frequencies",
                        Some(super::Message::SatManager(SatMsg::LoadFrequencies))
                    ))
                ))
            )
        ))
    }
}

impl super::Window for Window {
    fn title(&self) -> String {
        self.sat_manager.title()
    }

    fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, super::Message> {
        let mb = self.menu_bar(app);
        let content = self.sat_manager.view(app).map(super::Message::SatManager);
        column![mb, content].height(Length::Fill).into()
    }

    fn update(&mut self, msg: super::Message, app: &AppShared) -> Task<super::Message> {
        let super::Message::SatManager(m) = msg else {
            return Task::none();
        };
        self.sat_manager.update(m, app).map(|out| match out {
            Out::Msg(m) => super::Message::SatManager(m),
            Out::SatellitesChanged(sats) => super::Message::ToApp(Box::new(
                app::Message::WorkspaceSatellitesChanged(sats),
            )),
            Out::SatelliteChanged(idx, data) => super::Message::ToApp(Box::new(
                app::Message::WorkspaceSatelliteChanged(idx, data),
            )),
            Out::FrequenciesChanged(freqs) => super::Message::ToApp(Box::new(
                app::Message::WorkspaceFrequenciesChanged(freqs),
            )),
            Out::OpenPreferences => {
                super::Message::ToApp(Box::new(app::Message::OpenPreferences))
            }
        })
    }

    fn to_window_spec(&self) -> Option<WindowSpec> {
        Some(WindowSpec::SatManager(Box::new(self.sat_manager.clone())))
    }
}
