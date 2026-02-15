use std::{collections::HashMap, path::PathBuf};

use iced::Task;
use rstrf::orbit::Satellite;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    app::AppEvent,
    panes::{Pane, PaneTree, SplitAxis, rfplot::RFPlot, sat_manager::SatManager},
};

#[derive(Clone)]
#[allow(clippy::enum_variant_names)]
pub enum Message {
    SatellitesChanged(Vec<(Satellite, bool)>),
    SatelliteChanged(usize, Box<(Satellite, bool)>),
    FrequenciesChanged(HashMap<u64, f64>),
}

#[derive(Debug, Clone)]
pub enum Event {
    SatellitesChanged,
    App(AppEvent),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::SatellitesChanged(sats) => {
                write!(f, "Message::SatellitesChanged(len={})", sats.len())
            }
            Message::FrequenciesChanged(freqs) => {
                write!(f, "Message::FrequenciesChanged(len={})", freqs.len())
            }
            Message::SatelliteChanged(idx, data) => {
                write!(
                    f,
                    "Message::SatelliteChanged(idx={}, sat={:?}, active={})",
                    idx, data.0, data.1
                )
            }
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub panes: PaneTree,
    #[serde(default)]
    pub auto_save: bool,
    #[serde(default)]
    pub shared: WorkspaceShared,
}

impl Workspace {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let reader = std::fs::File::open(path)?;
        let ws = serde_json::from_reader(reader)?;
        Ok(ws)
    }

    pub fn update(&mut self, message: Message) -> Task<Event> {
        match message {
            Message::SatellitesChanged(sats) => {
                self.shared.satellites = sats;
                Task::done(Event::SatellitesChanged)
            }
            Message::SatelliteChanged(idx, data) => {
                log::debug!("SatelliteChanged({}, {:?})", idx, data);
                match self.shared.satellites.get_mut(idx) {
                    Some(sat) => *sat = *data,
                    None => log::error!("Got SatelliteChanged for non-existent index {}", idx),
                };
                Task::done(Event::SatellitesChanged)
            }
            Message::FrequenciesChanged(freqs) => {
                self.shared.satellites.iter_mut().for_each(|(sat, _)| {
                    if let Some(freq) = freqs.get(&sat.norad_id()) {
                        sat.tx_freq = *freq;
                    }
                });
                self.shared.frequencies = freqs;
                Task::none()
            }
        }
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            panes: PaneTree::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.7,
                a: Box::new(PaneTree::Leaf(Pane::RFPlot(Box::new(RFPlot::new())))),
                b: Box::new(PaneTree::Leaf(Pane::SatManager(
                    Box::new(SatManager::new()),
                ))),
            },
            auto_save: true,
            shared: WorkspaceShared::default(),
        }
    }
}

#[serde_as]
#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorkspaceShared {
    pub satellites: Vec<(Satellite, bool)>,
    #[serde_as(as = "HashMap<DisplayFromStr, _>")]
    pub frequencies: HashMap<u64, f64>,
}

impl WorkspaceShared {
    pub fn active_satellites(&self) -> Vec<Satellite> {
        self.satellites
            .iter()
            .filter_map(|(sat, active)| active.then(|| sat.clone()))
            .collect()
    }
}
