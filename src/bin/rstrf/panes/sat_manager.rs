use std::{collections::HashMap, path::PathBuf};

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
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    app::WorkspaceEvent,
    panes::{Message as PaneMessage, Pane, PaneTree, PaneWidget},
};

#[derive(Debug, Clone)]
pub enum Message {
    LoadTLEs,
    LoadFrequencies,
    DoLoadTLEs(PathBuf),
    DoLoadFrequencies(PathBuf),
    SatellitesLoaded(Result<Vec<Satellite>, String>),
    FrequenciesLoaded(Result<HashMap<u64, f64>, String>),
    SatelliteToggled(usize, bool),
}

#[serde_as]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct SatManager {
    satellites: Vec<(Satellite, bool)>,
    #[serde_as(as = "HashMap<DisplayFromStr, _>")]
    frequencies: HashMap<u64, f64>,
}

impl SatManager {
    pub fn new() -> Self {
        Self {
            satellites: Vec::new(),
            frequencies: HashMap::new(),
        }
    }

    pub fn satellites(&self) -> Vec<Satellite> {
        self.satellites
            .iter()
            .filter_map(|(sat, active)| if *active { Some(sat.clone()) } else { None })
            .collect::<Vec<_>>()
    }
}

impl PaneWidget for SatManager {
    fn update(&mut self, message: PaneMessage) -> Task<PaneMessage> {
        match message {
            PaneMessage::SatManager(message) => match message {
                Message::LoadTLEs => Task::future(pick_file(&[("TLEs", &["tle", "txt"])]))
                    .and_then(|p| Task::done(PaneMessage::SatManager(Message::DoLoadTLEs(p)))),
                Message::LoadFrequencies => Task::future(pick_file(&[("Frequencies", &["txt"])]))
                    .and_then(|p| {
                        Task::done(PaneMessage::SatManager(Message::DoLoadFrequencies(p)))
                    }),
                Message::DoLoadTLEs(path) => {
                    let frequencies = self.frequencies.clone();
                    Task::future(async move {
                        let satellites: anyhow::Result<_> =
                            rstrf::orbit::load_tles(&path, frequencies).await;
                        PaneMessage::SatManager(Message::SatellitesLoaded(
                            satellites.map_err(|e| format!("{e:?}")),
                        ))
                    })
                }
                Message::DoLoadFrequencies(path) => Task::future(async move {
                    let freqs = rstrf::orbit::load_frequencies(&path).await;
                    PaneMessage::SatManager(Message::FrequenciesLoaded(
                        freqs.map_err(|e| format!("{e:?}")),
                    ))
                }),
                Message::SatellitesLoaded(satellites) => match satellites {
                    Ok(satellites) => {
                        log::info!("Loaded {} satellites", satellites.len());
                        self.satellites = satellites.into_iter().map(|sat| (sat, true)).collect();
                        Task::done(PaneMessage::Workspace(WorkspaceEvent::SatellitesChanged(
                            self.satellites(),
                        )))
                    }
                    Err(err) => {
                        log::error!("Failed to load satellites: {}", err);
                        Task::none()
                    }
                },
                Message::FrequenciesLoaded(frequencies) => match frequencies {
                    Ok(frequencies) => {
                        log::info!("Loaded frequencies for {} satellites", frequencies.len());
                        self.frequencies = frequencies;
                        self.satellites.iter_mut().for_each(|(sat, _)| {
                            if let Some(freq) = self.frequencies.get(&sat.norad_id()) {
                                sat.tx_freq = *freq;
                            }
                        });
                        Task::done(PaneMessage::Workspace(WorkspaceEvent::SatellitesChanged(
                            self.satellites(),
                        )))
                    }
                    Err(err) => {
                        log::error!("Failed to load frequencies: {}", err);
                        Task::none()
                    }
                },
                Message::SatelliteToggled(idx, active) => {
                    let Some((_, sat_active)) = self.satellites.get_mut(idx) else {
                        log::error!("Invalid satellite index toggled: {}", idx);
                        return Task::none();
                    };
                    *sat_active = active;
                    Task::done(PaneMessage::Workspace(WorkspaceEvent::SatellitesChanged(
                        self.satellites(),
                    )))
                }
            },
            _ => Task::none(),
        }
    }

    fn view(&self, _size: Size) -> Element<'_, PaneMessage> {
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
                |(_, (sat, _)): (usize, &(Satellite, bool))| text(sat.norad_id().to_string()),
            ),
            table::column(
                text("Name"),
                |(_, (sat, _)): (usize, &(Satellite, bool))| {
                    text(
                        sat.elements
                            .object_name
                            .clone()
                            .unwrap_or("N/A".to_string()),
                    )
                },
            ),
            table::column(
                text("Frequency (MHz)"),
                |(_, (sat, _)): (usize, &(Satellite, bool))| {
                    text(format!("{:3}", sat.tx_freq / 1e6))
                },
            ),
            table::column(
                text("Show"),
                |(idx, (_, active)): (usize, &(Satellite, bool))| {
                    checkbox(*active)
                        .on_toggle(move |new_state| Message::SatelliteToggled(idx, new_state))
                },
            ),
        ];
        let table: Element<'_, Message> =
            scrollable(table(columns, self.satellites.iter().enumerate()))
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
        PaneTree::Leaf(Pane::SatManager(self.clone()))
    }
}
