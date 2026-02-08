use std::path::{Path, PathBuf};

use iced::widget::pane_grid;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
                a: Box::new(PaneTree::Leaf(Pane::RFPlot {
                    spectrogram: Vec::new(),
                })),
                b: Box::new(PaneTree::Leaf(Pane::SatManager {
                    elements: Path::new("/dev/null").into(),
                    frequencies: Path::new("/dev/null").into(),
                })),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneTree {
    Split {
        axis: SplitAxis,
        ratio: f32,
        a: Box<PaneTree>,
        b: Box<PaneTree>,
    },
    Leaf(Pane),
}

impl PaneTree {
    pub fn leftmost_leaf(&self) -> &Pane {
        match self {
            PaneTree::Leaf(pane) => pane,
            PaneTree::Split { a, .. } => a.leftmost_leaf(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitAxis {
    #[serde(rename = "h")]
    Horizontal,
    #[serde(rename = "v")]
    Vertical,
}

impl From<SplitAxis> for pane_grid::Axis {
    fn from(value: SplitAxis) -> Self {
        match value {
            SplitAxis::Horizontal => pane_grid::Axis::Horizontal,
            SplitAxis::Vertical => pane_grid::Axis::Vertical,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "pane", rename_all = "snake_case")]
pub enum Pane {
    #[serde(rename = "rfplot")]
    RFPlot { spectrogram: Vec<PathBuf> },
    SatManager {
        elements: PathBuf,
        frequencies: PathBuf,
    },
}
