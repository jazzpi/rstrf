use std::{collections::HashMap, path::PathBuf, sync::Arc};

use iced::{
    Element, Font, Length, Size, Task,
    alignment::Horizontal,
    font,
    widget::{
        Column, Grid, button, checkbox, column, container, grid::Sizing, scrollable, table, text,
        text_input,
    },
};
use iced_aw::{card, menu_bar, menu_items};
use rstrf::{
    menu::{button_f, button_s, submenu, view_menu},
    orbit::Satellite,
    util::{pick_file, spacetrack_to_sgp4},
};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use space_track::{GeneralPerturbationField, Predicate, SpaceTrack};
use strum::{EnumIter, IntoEnumIterator};
use tokio::sync::Mutex;

use crate::{
    app::AppShared,
    config::Config,
    panes::{Message as PaneMessage, Pane, PaneTree, PaneWidget},
    widgets::{Form, Icon, ToolbarButton, form, toolbar},
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
    SpaceTrackToggle,
    SpaceTrackUpdateAll,
    SpaceTrackUpdateVisible,
    SpaceTrackLogOut,
    SpaceTrackForm(form::Message),
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
    show_spacetrack: bool,
    #[serde(skip)]
    #[serde(default = "SatManager::create_spacetrack_form")]
    spacetrack_form: Form,
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
            show_spacetrack: false,
            spacetrack_form: SatManager::create_spacetrack_form(),
            sat_buffer: HashMap::new(),
            columns: Self::default_columns(),
        }
    }

    fn create_spacetrack_form() -> Form {
        Form::new(
            vec![
                ("Username".into(), form::Field::Text(String::new())),
                ("Password".into(), form::Field::Password(String::new())),
            ],
            "Log in".into(),
        )
    }

    fn spacetrack_update(
        space_track: Option<Arc<Mutex<SpaceTrack>>>,
        mut satellites: Vec<(Satellite, bool)>,
        active_only: bool,
    ) -> Task<PaneMessage> {
        let Some(space_track) = space_track else {
            return Task::none();
        };
        let space_track = space_track.clone();
        let mut norad_ids = Vec::new();
        let mut id_to_idx = HashMap::new();
        for (idx, (sat, active)) in satellites.iter().enumerate() {
            if !active_only || *active {
                let norad_id = sat.norad_id() as u32;
                norad_ids.push(norad_id);
                id_to_idx.insert(norad_id, idx);
            }
        }
        Task::future(async move {
            let mut space_track = space_track.lock().await;
            let cfg = space_track::Config::empty()
                .predicate(Predicate::build_range_list(
                    GeneralPerturbationField::NoradCatId,
                    norad_ids,
                ))
                .predicate(Predicate {
                    field: GeneralPerturbationField::Epoch,
                    value: ">now-10".to_string()
                })
                .predicate(Predicate {
                    field: GeneralPerturbationField::DecayDate,
                    value: "null-val".to_string()
                });
            space_track.gp(cfg).await
        }).then(move |result| match result {
            Ok(sats) => {
                for sat in sats {
                    let Some(elements) = spacetrack_to_sgp4(&sat) else {
                        log::error!("Failed to convert Space-Track data to SGP4 elements for satellite with NORAD ID {}", sat.norad_cat_id);
                        continue;
                    };
                    let Some(idx) = id_to_idx.get(&(sat.norad_cat_id as u32)) else {
                        log::error!("Got Space-Track data for NORAD ID {} which is not in the current satellite list", sat.norad_cat_id);
                        continue;
                    };
                    satellites[*idx].0.elements = elements;
                }
                Task::done(PaneMessage::ToWorkspace(
                    WorkspaceMessage::SatellitesChanged(satellites.clone()),
                ))
            },
            Err(err) => {
                log::error!("Failed to fetch data from Space-Track: {err}");
                Task::none()
            }
        })
    }
}

impl PaneWidget for SatManager {
    fn update(
        &mut self,
        message: PaneMessage,
        workspace: &workspace::WorkspaceShared,
        app: &AppShared,
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
                Message::SpaceTrackToggle => {
                    self.show_spacetrack = !self.show_spacetrack;
                    Task::none()
                }
                Message::Nop => Task::none(),
                Message::SpaceTrackUpdateAll => Self::spacetrack_update(
                    app.space_track.clone(),
                    workspace.satellites.clone(),
                    false,
                ),
                Message::SpaceTrackUpdateVisible => Self::spacetrack_update(
                    app.space_track.clone(),
                    workspace.satellites.clone(),
                    true,
                ),
                Message::SpaceTrackLogOut => Task::done(PaneMessage::UpdateConfig(Config {
                    space_track_creds: None,
                })),
                Message::SpaceTrackForm(form::Message::Submit) => {
                    let values = self.spacetrack_form.field_values();
                    Task::done(PaneMessage::UpdateConfig(Config {
                        space_track_creds: Some((values[0].clone(), values[1].clone())),
                    }))
                }
                Message::SpaceTrackForm(form_msg) => {
                    self.spacetrack_form.update(form_msg);
                    Task::none()
                }
            },
            _ => Task::none(),
        }
    }

    fn view(
        &self,
        _size: Size,
        workspace: &WorkspaceShared,
        app_state: &AppShared,
    ) -> Element<'_, PaneMessage> {
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
        let mut content = Column::new().spacing(4).padding(8);
        if let Some(onboarding) = onboarding {
            content = content.push(onboarding);
        }
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
        let buttons = toolbar([
            ToolbarButton::IconButton {
                icon: show_all.0,
                tooltip: show_all.1,
                msg: Message::ToggleAllSatellites.into(),
                style: button::primary,
            },
            ToolbarButton::IconButton {
                icon: Icon::ViewColumns,
                tooltip: toggle_columns_label,
                msg: Message::ToggleColumnControls.into(),
                style: button::primary,
            },
            ToolbarButton::IconButton {
                icon: Icon::Download,
                tooltip: "Fetch orbital elements",
                msg: Message::SpaceTrackToggle.into(),
                style: button::primary,
            },
        ]);
        let mut controls = column![buttons].spacing(8);
        if self.show_column_controls {
            controls = controls.push(
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
                .spacing(6),
            );
        }
        if self.show_spacetrack {
            let space_track: Element<'_, Message> = match app_state.space_track {
                Some(_) => container(
                    column![
                        button("Update all satellites from Space-Track")
                            .style(button::primary)
                            .on_press(Message::SpaceTrackUpdateAll)
                            .width(Length::Fill),
                        button("Update visible satellites from Space-Track")
                            .style(button::primary)
                            .on_press(Message::SpaceTrackUpdateVisible)
                            .width(Length::Fill),
                        button("Log out of Space-Track")
                            .style(button::danger)
                            .on_press(Message::SpaceTrackLogOut)
                            .width(Length::Fill),
                    ]
                    .padding([0, 50])
                    .spacing(6)
                )
                .center_x(Length::Fill)
                .into(),
                None =>
                    card(
                        "Missing Credentials", column![
                            text("To fetch orbital elements from Space-Track, please enter your credentials."),
                            self.spacetrack_form.view().map(Message::SpaceTrackForm)
                        ]
                        .spacing(10)
                        .width(Length::Fill)
                    )
                    .style(iced_aw::style::card::warning)
                    .into()
            };
            controls = controls.push(space_track);
        }
        let controls = container(controls)
            .padding(8)
            .width(Length::Fill)
            .style(container::bordered_box);
        content = content.push(controls).push(table);
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
