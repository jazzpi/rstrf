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
    pub version: String,
    pub panes: PaneTree,
    pub auto_save: bool,
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
                Task::done(Event::SatellitesChanged)
            }
        }
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            panes: PaneTree::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.7,
                a: Box::new(PaneTree::Leaf(Pane::RFPlot(Box::new(RFPlot::new())))),
                b: Box::new(PaneTree::Leaf(Pane::SatManager(
                    Box::new(SatManager::new()),
                ))),
            },
            auto_save: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_serializes_round_trip() {
        let ws = Workspace::default();
        let json = serde_json::to_string(&ws).unwrap();
        let ws2: Workspace = serde_json::from_str(&json).unwrap();
        assert!(ws == ws2);
    }

    #[test]
    fn frequencies_changed_stores_map() {
        let mut ws = Workspace::default();
        let mut freqs = HashMap::new();
        freqs.insert(25544u64, 437.525e6);
        freqs.insert(5u64, 108.03e6);
        let _task = ws.update(Message::FrequenciesChanged(freqs.clone()));
        assert_eq!(ws.shared.frequencies.get(&25544), Some(&437.525e6));
        assert_eq!(ws.shared.frequencies.get(&5), Some(&108.03e6));
    }

    #[test]
    fn satellite_changed_out_of_bounds_does_not_panic() {
        let mut ws = Workspace::default();
        // Empty satellites list — index 999 doesn't exist, should log error and not panic
        let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
        let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
        let sat = Satellite::from_tle(Some("V1".to_string()), line1, line2, &HashMap::new()).unwrap();
        let _task = ws.update(Message::SatelliteChanged(999, Box::new((sat, true))));
        assert!(ws.shared.satellites.is_empty());
    }

    #[test]
    fn active_satellites_filters_by_active_flag() {
        let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
        let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
        let sat = Satellite::from_tle(Some("V1".to_string()), line1, line2, &HashMap::new()).unwrap();
        let mut ws = Workspace::default();
        let _task = ws.update(Message::SatellitesChanged(vec![
            (sat.clone(), true),
            (sat.clone(), false),
        ]));
        assert_eq!(ws.shared.active_satellites().len(), 1);
    }
}
