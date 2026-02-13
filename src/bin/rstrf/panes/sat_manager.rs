use std::{collections::HashMap, path::PathBuf};

use iced::{
    Element, Length, Size, Task,
    alignment::Horizontal,
    widget::{Column, button, checkbox, column, container, scrollable, table, text, text_input},
};
use iced_aw::{card, menu_bar, menu_items};
use rstrf::{
    menu::{button_f, button_s, submenu, view_menu},
    orbit::Satellite,
    util::pick_file,
};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

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
    ToggleAllSatellites(bool),
    SatelliteEdited(usize, Box<Satellite>),
    SatelliteEditCommited(usize),
    Nop,
}

#[serde_as]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct SatManager {
    #[serde(default)]
    show_all: bool,
    #[serde(default)]
    #[serde_as(as = "HashMap<DisplayFromStr, _>")]
    sat_buffer: HashMap<usize, Satellite>,
}

impl SatManager {
    pub fn new() -> Self {
        Self {
            show_all: false,
            sat_buffer: HashMap::new(),
        }
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
                Message::SatelliteToggled(idx, active) => match workspace.satellites.get(idx) {
                    Some((sat, _)) => Task::done(PaneMessage::ToWorkspace(
                        WorkspaceMessage::SatelliteChanged(idx, Box::new((sat.clone(), active))),
                    )),
                    None => {
                        log::error!("Got SatelliteToggle for non-existend index {}", idx);
                        Task::none()
                    }
                },
                Message::ToggleAllSatellites(active) => {
                    self.show_all = active;
                    Task::done(PaneMessage::ToWorkspace(
                        WorkspaceMessage::SatellitesChanged(
                            workspace
                                .satellites
                                .iter()
                                .map(|(sat, _)| (sat.clone(), active))
                                .collect(),
                        ),
                    ))
                }
                Message::SatelliteEdited(id, sat) => {
                    self.sat_buffer.insert(id, *sat);
                    Task::none()
                }
                Message::SatelliteEditCommited(idx) => {
                    match (self.sat_buffer.remove(&idx), workspace.satellites.get(idx)) {
                        (Some(buf_data), Some(old_data)) => Task::done(PaneMessage::ToWorkspace(
                            WorkspaceMessage::SatelliteChanged(
                                idx,
                                Box::new((buf_data, old_data.1)),
                            ),
                        )),
                        _ => Task::none(),
                    }
                }
                Message::Nop => Task::none(),
            },
            _ => Task::none(),
        }
    }

    fn view(&self, _size: Size, workspace: &WorkspaceShared) -> Element<'_, PaneMessage> {
        let mb = view_menu(menu_bar!((
            button_s("File", None),
            submenu(menu_items!(
                (button_f("Load TLEs", Some(Message::LoadTLEs))),
                (button_f("Load frequencies", Some(Message::LoadFrequencies))),
            ))
        )));
        let onboarding = if workspace.satellites.is_empty() {
            let head: Element<'_, Message> = text("TIP").into();
            let content: Element<'_, Message> = column![
                text("You don't have any satellites loaded yet. Try loading some TLEs from the File menu or the button below."),
                button(text("Load TLEs")).style(button::primary).width(200.0).on_press(Message::LoadTLEs)
            ].spacing(10).width(Length::Fill).align_x(Horizontal::Center).into();
            Some(card(head, content).style(iced_aw::style::card::info))
        } else if workspace
            .satellites
            .iter()
            .all(|(sat, _)| sat.tx_freq == 0.0)
        {
            let head: Element<'_, Message> = text("TIP").into();
            let content: Element<'_, Message> = column![
                text("You don't have any transmit frequencies set for the satellites. Try editing the frequency fields, or loading an STRF frequencies.txt file from the File menu or the button below."),
                button(text("Load Frequencies")).style(button::primary).width(200.0).on_press(Message::LoadFrequencies)
            ].spacing(10).width(Length::Fill).align_x(Horizontal::Center).into();
            Some(card(head, content).style(iced_aw::style::card::info))
        } else {
            None
        };
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
                |(id, (sat, _)): (usize, (Satellite, bool))| {
                    text_input("...", format!("{:.3}", sat.tx_freq / 1e6).as_str())
                        .on_input(move |freq| {
                            let sat = sat.clone();
                            freq.parse::<f64>()
                                .ok()
                                .map(move |freq| {
                                    let sat = Satellite {
                                        tx_freq: freq * 1e6,
                                        ..sat.clone()
                                    };
                                    Message::SatelliteEdited(id, Box::new(sat))
                                })
                                .unwrap_or(Message::Nop)
                        })
                        .on_submit(Message::SatelliteEditCommited(id))
                        .width(Length::Fixed(100.0))
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
            workspace.satellites.iter().enumerate().map(|(id, data)| {
                (
                    id,
                    self.sat_buffer
                        .get(&id)
                        .map(|sat| (sat.clone(), data.1))
                        .unwrap_or(data.clone()),
                )
            }),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        let toggle_all = container(
            checkbox(self.show_all)
                .label("Show all satellites")
                .on_toggle(Message::ToggleAllSatellites),
        )
        .padding([4, 10]);
        let mut content = Column::new().spacing(4).padding(8);
        if let Some(onboarding) = onboarding {
            content = content.push(onboarding);
        }
        content = content.push(toggle_all).push(table);
        let result: Element<'_, Message> = column![mb, content].into();
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
