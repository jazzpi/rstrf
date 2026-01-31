use std::path::PathBuf;

use iced::{
    Element, Length, Size, Task,
    widget::{checkbox, scrollable, table, text},
};
use rstrf::orbit::Satellite;

use crate::{
    app::WorkspaceEvent,
    panes::{Message as PaneMessage, PaneWidget},
};

#[derive(Debug, Clone)]
pub enum Message {
    LoadTLEs {
        tle_path: PathBuf,
        freqs_path: PathBuf,
    },
    SatellitesLoaded(Result<Vec<Satellite>, String>),
    SatelliteToggled(usize, bool),
}

pub struct SatManager {
    satellites: Vec<(Satellite, bool)>,
}

impl SatManager {
    pub fn new() -> Self {
        Self {
            satellites: Vec::new(),
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
                Message::LoadTLEs {
                    tle_path,
                    freqs_path,
                } => Task::future(async move {
                    let satellites: anyhow::Result<_> = async {
                        let freqs = rstrf::orbit::load_frequencies(&freqs_path).await?;
                        rstrf::orbit::load_tles(&tle_path, freqs).await
                    }
                    .await;
                    PaneMessage::SatManager(Message::SatellitesLoaded(
                        satellites.map_err(|e| format!("{e:?}")),
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
        let result: Element<'_, Message> =
            scrollable(table(columns, self.satellites.iter().enumerate()))
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        result.map(PaneMessage::from)
    }

    fn title(&self) -> &str {
        "Satellites"
    }
}
