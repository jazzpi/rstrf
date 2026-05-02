use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::panes::{rfplot::RFPlot, sat_manager::SatManager};
use rstrf::orbit::Satellite;

#[derive(Serialize, Deserialize, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WindowSpec {
    RFPlot(Box<RFPlot>),
    SatManager(Box<SatManager>),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub version: String,
    pub windows: Vec<WindowSpec>,
    pub auto_save: bool,
    pub shared: WorkspaceShared,
}

impl Workspace {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let reader = std::fs::File::open(path)?;
        let ws = serde_json::from_reader(reader)?;
        Ok(ws)
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            windows: vec![
                WindowSpec::RFPlot(Box::new(RFPlot::new())),
                WindowSpec::SatManager(Box::new(SatManager::new())),
            ],
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
    fn active_satellites_filters_by_active_flag() {
        let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
        let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
        let sat = Satellite::from_tle(Some("V1".to_string()), line1, line2, &HashMap::new()).unwrap();
        let mut ws = Workspace::default();
        ws.shared.satellites = vec![(sat.clone(), true), (sat.clone(), false)];
        assert_eq!(ws.shared.active_satellites().len(), 1);
    }
}
