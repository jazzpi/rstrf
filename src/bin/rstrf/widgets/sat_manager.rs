use std::path::PathBuf;

use iced::{
    Element, Length, Task,
    alignment::{Horizontal, Vertical},
    widget::{container, text},
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
}

pub struct SatManager {
    satellites: Vec<Satellite>,
}

impl SatManager {
    pub fn new() -> Self {
        Self {
            satellites: Vec::new(),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        container(text("Satellite Manager - TODO"))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
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
                    self.satellites = satellites;
                    Task::done(Message::SatellitesChanged(self.satellites().to_vec()))
                }
                Err(err) => {
                    log::error!("Failed to load satellites: {}", err);
                    Task::none()
                }
            },
            Message::SatellitesChanged(_) => Task::none(), // This message is intended for the parent
        }
    }

    pub fn satellites(&self) -> &[Satellite] {
        &self.satellites
    }
}
