use std::{collections::HashMap, path::PathBuf};

use iced::{
    Element, Font, Length, Size, Task,
    alignment::Horizontal,
    font,
    widget::{
        Column, Grid, button, checkbox, column, container, grid::Sizing, row, scrollable, table,
        text, text_input,
    },
};
use iced_aw::{card, menu_bar, menu_items};
use rstrf::{
    menu::{button_f, button_s, submenu, view_menu},
    orbit::Satellite,
    util::pick_file,
};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use strum::{EnumIter, IntoEnumIterator};

use crate::{
    panes::{Message as PaneMessage, Pane, PaneTree, PaneWidget},
    widgets::{Icon, icon_button},
    workspace::{self, Message as WorkspaceMessage, WorkspaceShared},
};

#[derive(Debug, Clone)]
pub enum Message {
    LoadTLEs,
    LoadFrequencies,
    DoLoadTLEs(PathBuf),
    DoLoadFrequencies(PathBuf),
    SatelliteToggled(usize, bool),
    ToggleAllSatellites,
    SatelliteEdited(usize, Box<Satellite>),
    SatelliteEditCommited(usize),
    ToggleColumnControls,
    ToggleColumn(TableColumn, bool),
    Nop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, Serialize, Deserialize)]
pub enum TableColumn {
    NoradId,
    Epoch,
    Name,
    Frequency,
    Show,
}

impl TableColumn {
    pub fn header(&self) -> &'static str {
        match self {
            TableColumn::NoradId => "Norad ID",
            TableColumn::Epoch => "Epoch",
            TableColumn::Name => "Name",
            TableColumn::Frequency => "Frequency (MHz)",
            TableColumn::Show => "Show",
        }
    }

    pub fn view(self, idx: usize, sat: &Satellite, active: bool) -> Element<'static, Message> {
        match self {
            TableColumn::NoradId => text(sat.norad_id().to_string()).into(),
            TableColumn::Epoch => {
                text(sat.elements.datetime.format("%Y-%m-%d %H:%M").to_string()).into()
            }
            TableColumn::Name => text(
                sat.elements
                    .object_name
                    .clone()
                    .unwrap_or("N/A".to_string()),
            )
            .into(),
            TableColumn::Frequency => {
                let sat = sat.clone();
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
                                Message::SatelliteEdited(idx, Box::new(sat))
                            })
                            .unwrap_or(Message::Nop)
                    })
                    .on_submit(Message::SatelliteEditCommited(idx))
                    .width(Length::Fixed(100.0))
                    .into()
            }
            TableColumn::Show => checkbox(active)
                .on_toggle(move |new_state| Message::SatelliteToggled(idx, new_state))
                .into(),
        }
    }
}

#[serde_as]
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct SatManager {
    #[serde(default)]
    show_all: bool,
    #[serde(default)]
    show_column_controls: bool,
    #[serde(default)]
    #[serde_as(as = "HashMap<DisplayFromStr, _>")]
    sat_buffer: HashMap<usize, Satellite>,
    #[serde(default = "SatManager::default_columns")]
    columns: HashMap<TableColumn, bool>,
}

impl SatManager {
    fn default_columns() -> HashMap<TableColumn, bool> {
        TableColumn::iter().map(|col| (col, true)).collect()
    }

    pub fn new() -> Self {
        Self {
            show_all: false,
            show_column_controls: false,
            sat_buffer: HashMap::new(),
            columns: Self::default_columns(),
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
                Message::ToggleAllSatellites => {
                    self.show_all = !self.show_all;
                    Task::done(PaneMessage::ToWorkspace(
                        WorkspaceMessage::SatellitesChanged(
                            workspace
                                .satellites
                                .iter()
                                .map(|(sat, _)| (sat.clone(), self.show_all))
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
                Message::ToggleColumnControls => {
                    self.show_column_controls = !self.show_column_controls;
                    Task::none()
                }
                Message::ToggleColumn(column, visible) => {
                    self.columns.insert(column, visible);
                    Task::none()
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
        let columns = TableColumn::iter().filter_map(|col| {
            self.columns.get(&col).and_then(|visible| {
                visible.then(|| {
                    table::column(
                        text(col.header()),
                        move |(idx, (sat, active)): (usize, (Satellite, bool))| {
                            col.view(idx, &sat, active).map(Message::from)
                        },
                    )
                })
            })
        });
        let table = table(
            columns,
            workspace
                .satellites
                .iter()
                .enumerate()
                .map(|(id, (sat, active))| {
                    let sat = self.sat_buffer.get(&id).unwrap_or(sat);
                    (id, (sat.clone(), *active))
                }),
        );
        let table: Element<'_, Message> = scrollable(table)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        let show_all = if self.show_all {
            (Icon::EyeOff, "Hide all satellites")
        } else {
            (Icon::Eye, "Show all satellites")
        };
        let toggle_columns_label = if self.show_column_controls {
            "Hide column controls"
        } else {
            "Show column controls"
        };
        let buttons = row![
            icon_button(
                show_all.0,
                show_all.1,
                Message::ToggleAllSatellites,
                button::primary
            ),
            icon_button(
                Icon::ViewColumns,
                toggle_columns_label,
                Message::ToggleColumnControls,
                button::primary
            )
        ]
        .padding([4, 10])
        .spacing(10);
        let mut content = Column::new().spacing(4).padding(8);
        if let Some(onboarding) = onboarding {
            content = content.push(onboarding);
        }
        content = content.push(buttons);
        if self.show_column_controls {
            content = content.push(
                container(
                    column![
                        text("Show columns:").font(Font {
                            weight: font::Weight::Bold,
                            ..Font::default()
                        }),
                        Grid::from_iter(TableColumn::iter().map(|col| {
                            container(
                                checkbox(self.columns.get(&col).copied().unwrap_or_default())
                                    .label(col.header())
                                    .on_toggle(move |visible| Message::ToggleColumn(col, visible)),
                            )
                            .center_y(Length::Shrink)
                            .into()
                        }))
                        .height(Sizing::EvenlyDistribute(Length::Shrink))
                        .fluid(200.0)
                        .spacing(4)
                    ]
                    .spacing(6)
                    .padding(6),
                )
                .style(container::secondary),
            );
        }
        content = content.push(table);
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
