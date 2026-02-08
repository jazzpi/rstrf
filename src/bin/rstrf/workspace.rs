use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::panes::{Pane, PaneTree, SplitAxis, rfplot::RFPlot, sat_manager::SatManager};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub panes: PaneTree,
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
            panes: PaneTree::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.7,
                a: Box::new(PaneTree::Leaf(Pane::RFPlot(RFPlot::new()))),
                b: Box::new(PaneTree::Leaf(Pane::SatManager(SatManager::new()))),
            },
        }
    }
}
