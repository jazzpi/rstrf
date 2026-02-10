use anyhow::bail;
use iced::{Element, Size, Task, widget::pane_grid};
use serde::{Deserialize, Serialize};

use crate::{
    app::WorkspaceEvent,
    panes::{dummy::Dummy, rfplot::RFPlot, sat_manager::SatManager},
};

pub mod dummy;
pub mod rfplot;
pub mod sat_manager;

#[derive(Debug, Clone)]
pub enum Message {
    RFPlot(rfplot::Message),
    SatManager(sat_manager::Message),
    Workspace(WorkspaceEvent),
    ReplacePane(Pane),
}

#[derive(Debug, Clone)]
pub struct PaneMessage {
    pub id: pane_grid::Pane,
    pub message: Message,
}

impl From<rfplot::Message> for Message {
    fn from(message: rfplot::Message) -> Self {
        Message::RFPlot(message)
    }
}

impl From<sat_manager::Message> for Message {
    fn from(message: sat_manager::Message) -> Self {
        Message::SatManager(message)
    }
}

pub trait PaneWidget {
    fn init(&mut self) -> Task<Message> {
        Task::none()
    }
    fn update(&mut self, message: Message) -> Task<Message>;
    fn view(&self, size: Size) -> Element<'_, Message>;
    fn title(&self) -> String;
    fn to_tree(&self) -> PaneTree;
}

#[derive(Serialize, Deserialize, PartialEq, Clone)]
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

impl From<pane_grid::Axis> for SplitAxis {
    fn from(value: pane_grid::Axis) -> Self {
        match value {
            pane_grid::Axis::Horizontal => SplitAxis::Horizontal,
            pane_grid::Axis::Vertical => SplitAxis::Vertical,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Clone)]
#[serde(tag = "pane", rename_all = "snake_case")]
pub enum Pane {
    #[serde(rename = "rfplot")]
    RFPlot(RFPlot),
    SatManager(SatManager),
    Dummy(Dummy),
}

impl std::fmt::Debug for Pane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pane::RFPlot(_) => write!(f, "Pane::RFPlot"),
            Pane::SatManager(_) => write!(f, "Pane::SatManager"),
            Pane::Dummy(_) => write!(f, "Pane::Dummy"),
        }
    }
}

pub type PaneGridState = pane_grid::State<Box<dyn PaneWidget>>;

/// Generate a pane_grid::State from a (serializable) PaneTree
///
/// Ideally, we could just uses pane_grid::State::from_configuration -- but we need the panes' IDs
/// to map the task messages correctly. And unfortunately, `from_configuration` does not provide any
/// way to access the pane IDs it generates.
///
/// So, instead, we iteratively build the pane grid by splitting panes repeatedly. You might think
/// we could just do an in-order traversal and split the subtrees as we go, but the API doesn't
/// allow splitting subtrees -- only existing panes (leafs). So we first find the leftmost leaf of
/// both subtrees, and split the leftmost leaf of the left subtree with the leftmost leaf of the
/// right subtree. Afterwards, we recursively build the subtrees.
pub fn from_tree(tree: &PaneTree) -> anyhow::Result<(PaneGridState, Task<PaneMessage>)> {
    let leftmost = tree.leftmost_leaf();
    let mut widget = build_widget(leftmost);
    let task = widget.init();
    let (mut state, initial_pane) = PaneGridState::new(widget);
    let mut tasks = vec![task.map(move |message| PaneMessage {
        id: initial_pane,
        message,
    })];

    build_rest(&tree, &mut state, initial_pane, &mut tasks, leftmost)?;

    Ok((state, Task::batch(tasks)))
}

fn build_rest(
    tree: &PaneTree,
    state: &mut PaneGridState,
    left_pane: pane_grid::Pane,
    tasks: &mut Vec<Task<PaneMessage>>,
    left_leftmost: &Pane,
) -> anyhow::Result<()> {
    match tree {
        PaneTree::Leaf(leaf) if leaf == left_leftmost => Ok(()),
        PaneTree::Leaf(_) => bail!("Unexpected leaf"),
        PaneTree::Split { axis, ratio, a, b } => {
            let right_leftmost = b.leftmost_leaf();
            let mut widget = build_widget(right_leftmost);
            let task = widget.init();
            let (right_pane, split) = state
                .split((*axis).into(), left_pane, widget)
                .ok_or(anyhow::anyhow!("Could not split pane"))?;
            state.resize(split, *ratio);
            tasks.push(task.map(move |message| PaneMessage {
                id: right_pane,
                message,
            }));

            build_rest(a, state, left_pane, tasks, left_leftmost)?;
            build_rest(b, state, right_pane, tasks, right_leftmost)?;

            Ok(())
        }
    }
}

fn build_widget(pane: &Pane) -> Box<dyn PaneWidget> {
    match pane {
        Pane::RFPlot(widget) => Box::new(widget.clone()),
        Pane::SatManager(widget) => Box::new(widget.clone()),
        Pane::Dummy(widget) => Box::new(widget.clone()),
    }
}

/// Generate a (serializable) PaneTree from a pane_grid::State
pub fn to_tree(state: &PaneGridState, layout: &pane_grid::Node) -> Option<PaneTree> {
    let node = match layout {
        pane_grid::Node::Split {
            id: _,
            axis,
            ratio,
            a,
            b,
        } => PaneTree::Split {
            axis: (*axis).into(),
            ratio: *ratio,
            a: Box::new(to_tree(state, a)?),
            b: Box::new(to_tree(state, b)?),
        },
        pane_grid::Node::Pane(pane) => state.get(*pane)?.to_tree(),
    };
    Some(node)
}
