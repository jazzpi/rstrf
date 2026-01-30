use std::path::PathBuf;

use iced::{
    Element, Length, Task,
    widget::{checkbox, scrollable, table, text},
};
use rstrf::orbit::Satellite;

#[derive(Debug, Clone)]
pub enum Message {
    LoadTLEs {
        tle_path: PathBuf,
        freqs_path: PathBuf,
    },
    SatellitesLoaded(Result<Vec<Satellite>, String>),
    /// For notifying the parent that the satellite list has changed
    SatellitesChanged(Vec<Satellite>),
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

    pub fn view(&self) -> Element<'_, Message> {
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
        scrollable(table(columns, self.satellites.iter().enumerate()))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoadTLEs {
                tle_path,
                freqs_path,
            } => Task::future(async move {
                let satellites: anyhow::Result<_> = async {
                    let freqs = rstrf::orbit::load_frequencies(&freqs_path).await?;
                    rstrf::orbit::load_tles(&tle_path, freqs).await
                }
                .await;
                Message::SatellitesLoaded(satellites.map_err(|e| format!("{e:?}")))
            }),
            Message::SatellitesLoaded(satellites) => match satellites {
                Ok(satellites) => {
                    log::info!("Loaded {} satellites", satellites.len());
                    self.satellites = satellites.into_iter().map(|sat| (sat, true)).collect();
                    Task::done(Message::SatellitesChanged(self.satellites().to_vec()))
                }
                Err(err) => {
                    log::error!("Failed to load satellites: {}", err);
                    Task::none()
                }
            },
            Message::SatellitesChanged(_) => Task::none(),
            Message::SatelliteToggled(idx, active) => {
                let Some((_, sat_active)) = self.satellites.get_mut(idx) else {
                    log::error!("Invalid satellite index toggled: {}", idx);
                    return Task::none();
                };
                *sat_active = active;
                Task::done(Message::SatellitesChanged(self.satellites()))
            }
        }
    }

    pub fn satellites(&self) -> Vec<Satellite> {
        self.satellites
            .iter()
            .filter_map(|(sat, active)| if *active { Some(sat.clone()) } else { None })
            .collect::<Vec<_>>()
    }
}
