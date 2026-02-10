use std::path::PathBuf;

use iced::{
    Element, Length, Size, Task,
    widget::{checkbox, column, scrollable, table, text},
};
use iced_aw::{menu_bar, menu_items};
use rstrf::{
    menu::{button_f, button_s, submenu, view_menu},
    orbit::Satellite,
    util::pick_file,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{
    panes::{Message as PaneMessage, Pane, PaneTree, PaneWidget},
    workspace::{self, Message as WorkspaceMessage, WorkspaceShared},
};

#[derive(Debug, Clone)]
pub enum Message {
    LoadTLEs,
    LoadFrequencies,
    DoLoadTLEs(PathBuf),
    DoLoadFrequencies(PathBuf),
    SatelliteToggled(usize, bool),
}

#[serde_as]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct SatManager {
    // In the future, this will hold e.g. column visibility settings
}

impl SatManager {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneWidget for SatManager {
    fn update(
        &mut self,
        message: PaneMessage,
        workspace: &workspace::WorkspaceShared,
    ) -> Task<PaneMessage> {
        match message {
            PaneMessage::SatManager(message) => match message {
                Message::LoadTLEs => Task::future(pick_file(&[("TLEs", &["tle", "txt"])]))
                    .and_then(|p| Task::done(PaneMessage::SatManager(Message::DoLoadTLEs(p)))),
                Message::LoadFrequencies => Task::future(pick_file(&[("Frequencies", &["txt"])]))
                    .and_then(|p| {
                        Task::done(PaneMessage::SatManager(Message::DoLoadFrequencies(p)))
                    }),
                Message::DoLoadTLEs(path) => {
                    let frequencies = workspace.frequencies.clone();
                    Task::future(async move {
                        let satellites: anyhow::Result<_> =
                            rstrf::orbit::load_tles(&path, frequencies).await;
                        satellites.map_err(|e| format!("{e:?}"))
                    })
                    .then(|result| match result {
                        Ok(sats) => {
                            log::info!("Loaded {} satellites", sats.len());
                            Task::done(PaneMessage::ToWorkspace(
                                WorkspaceMessage::SatellitesChanged(
                                    sats.into_iter().map(|sat| (sat, true)).collect(),
                                ),
                            ))
                        }
                        Err(err) => {
                            log::error!("Failed to load satellites: {}", err);
                            Task::none()
                        }
                    })
                }
                Message::DoLoadFrequencies(path) => Task::future(async move {
                    rstrf::orbit::load_frequencies(&path)
                        .await
                        .map_err(|e| format!("{e:?}"))
                    // PaneMessage::SatManager(Message::FrequenciesLoaded(
                    //     freqs,
                    // ))
                })
                .then(|result| match result {
                    Ok(freqs) => {
                        log::info!("Loaded frequencies for {} satellites", freqs.len());
                        Task::done(PaneMessage::ToWorkspace(
                            WorkspaceMessage::FrequenciesChanged(freqs),
                        ))
                    }
                    Err(err) => {
                        log::error!("Failed to load frequencies: {}", err);
                        Task::none()
                    }
                }),
                Message::SatelliteToggled(idx, active) => Task::done(PaneMessage::ToWorkspace(
                    WorkspaceMessage::SatellitesChanged(
                        workspace
                            .satellites
                            .iter()
                            .enumerate()
                            .map(|(i, (sat, was_active))| {
                                if i == idx {
                                    (sat.clone(), active)
                                } else {
                                    (sat.clone(), *was_active)
                                }
                            })
                            .collect(),
                    ),
                )),
            },
            _ => Task::none(),
        }
    }

    fn view(&self, _size: Size, workspace: &WorkspaceShared) -> Element<'_, PaneMessage> {
        let mb = view_menu(menu_bar!((
            button_s("File", None),
            submenu(menu_items!(
                (button_f("Load frequencies", Some(Message::LoadFrequencies))),
                (button_f("Load TLEs", Some(Message::LoadTLEs))),
            ))
        )));
        let columns = [
            table::column(
                text("Norad ID"),
                |(_, (sat, _)): (usize, (Satellite, bool))| text(sat.norad_id().to_string()),
            ),
            table::column(text("Name"), |(_, (sat, _)): (usize, (Satellite, bool))| {
                text(
                    sat.elements
                        .object_name
                        .clone()
                        .unwrap_or("N/A".to_string()),
                )
            }),
            table::column(
                text("Frequency (MHz)"),
                |(_, (sat, _)): (usize, (Satellite, bool))| {
                    text(format!("{:3}", sat.tx_freq / 1e6))
                },
            ),
            table::column(
                text("Show"),
                |(idx, (_, active)): (usize, (Satellite, bool))| {
                    checkbox(active)
                        .on_toggle(move |new_state| Message::SatelliteToggled(idx, new_state))
                },
            ),
        ];
        let table: Element<'_, Message> = scrollable(table(
            columns,
            workspace.satellites.iter().cloned().enumerate(),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        let result: Element<'_, Message> = column![mb, table].into();
        result.map(PaneMessage::from)
    }

    fn title(&self) -> String {
        "Satellites".into()
    }

    fn to_tree(&self) -> PaneTree {
        // TODO: turn this into into_tree(self)?
        PaneTree::Leaf(Pane::SatManager(Box::new(self.clone())))
    }
}
