use anyhow::bail;
use iced::{Element, Size, Task, widget::pane_grid};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppShared,
    panes::{dummy::Dummy, rfplot::RFPlot, sat_manager::SatManager},
    workspace::{self, Workspace, WorkspaceShared},
};

pub mod dummy;
pub mod rfplot;
pub mod sat_manager;

#[derive(Debug, Clone)]
pub enum Message {
    RFPlot(rfplot::Message),
    SatManager(sat_manager::Message),
    ToWorkspace(workspace::Message),
    ToApp(Box<crate::app::Message>),
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

/// A cross-cutting effect that escapes a pane's own message type and must be handled by the parent.
#[derive(Debug, Clone)]
pub enum PaneEffect {
    ToWorkspace(workspace::Message),
}

/// Return type for pane update functions: either a pane-local continuation or an escaped effect.
#[derive(Debug, Clone)]
pub enum PaneOut<M> {
    Msg(M),
    Effect(PaneEffect),
}

/// Shared interface for pane rendering and serialization. Does not include update logic, which is
/// dispatched concretely by `AnyPane` so each pane can use its own message type without lifting.
pub trait PaneWidget {
    fn view(
        &self,
        size: Size,
        workspace: &WorkspaceShared,
        app: &AppShared,
    ) -> Element<'_, Message>;
    fn title(&self) -> String;
    fn to_tree(&self) -> PaneTree;
}

/// A concrete enum over all pane types, used as the element type for `pane_grid::State`.
///
/// Unlike `Box<dyn PaneWidget>`, this allows `update`, `init`, and `workspace_event` to dispatch
/// directly to each pane's own message type, so pane-internal code never references `panes::Message`.
pub enum AnyPane {
    RFPlot(Box<RFPlot>),
    SatManager(Box<SatManager>),
    Dummy(Box<Dummy>),
}

impl AnyPane {
    fn as_dyn(&self) -> &dyn PaneWidget {
        match self {
            AnyPane::RFPlot(p) => p.as_ref(),
            AnyPane::SatManager(p) => p.as_ref(),
            AnyPane::Dummy(p) => p.as_ref(),
        }
    }

    pub fn title(&self) -> String {
        self.as_dyn().title()
    }

    pub fn view<'a>(
        &'a self,
        size: Size,
        workspace: &'a WorkspaceShared,
        app: &'a AppShared,
    ) -> Element<'a, Message> {
        self.as_dyn().view(size, workspace, app)
    }

    pub fn to_tree(&self) -> PaneTree {
        self.as_dyn().to_tree()
    }

    pub fn init(&mut self, workspace: &WorkspaceShared, app: &AppShared) -> Task<Message> {
        match self {
            AnyPane::RFPlot(p) => p.init(workspace, app).map(Message::RFPlot),
            AnyPane::SatManager(_) | AnyPane::Dummy(_) => Task::none(),
        }
    }

    pub fn workspace_event(
        &mut self,
        event: workspace::Event,
        workspace: &WorkspaceShared,
        app: &AppShared,
    ) -> Task<Message> {
        match self {
            AnyPane::RFPlot(p) => p
                .workspace_event(event, workspace, app)
                .map(Message::RFPlot),
            AnyPane::SatManager(_) | AnyPane::Dummy(_) => Task::none(),
        }
    }

    /// Dispatches the message to the correct pane's typed `update()`. The single lift from a
    /// pane-local message type to `panes::Message` happens here, not inside each pane.
    pub fn update(
        &mut self,
        message: Message,
        workspace: &WorkspaceShared,
        app: &AppShared,
    ) -> Task<Message> {
        match (self, message) {
            (AnyPane::RFPlot(p), Message::RFPlot(m)) => {
                p.update(m, workspace, app).map(Message::RFPlot)
            }
            (AnyPane::SatManager(p), Message::SatManager(m)) => {
                p.update(m, workspace, app).map(|out| match out {
                    PaneOut::Msg(m) => Message::SatManager(m),
                    PaneOut::Effect(PaneEffect::ToWorkspace(w)) => Message::ToWorkspace(w),
                })
            }
            (AnyPane::Dummy(p), m) => p.update(m, workspace, app),
            _ => Task::none(),
        }
    }
}

impl From<&Pane> for AnyPane {
    fn from(pane: &Pane) -> Self {
        match pane {
            Pane::RFPlot(p) => AnyPane::RFPlot(p.clone()),
            Pane::SatManager(p) => AnyPane::SatManager(p.clone()),
            Pane::Dummy(p) => AnyPane::Dummy(p.clone()),
        }
    }
}

pub type PaneGridState = pane_grid::State<AnyPane>;

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
    RFPlot(Box<RFPlot>),
    SatManager(Box<SatManager>),
    Dummy(Box<Dummy>),
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
pub fn from_workspace(
    workspace: &Workspace,
    app: &AppShared,
) -> anyhow::Result<(PaneGridState, Task<PaneMessage>)> {
    let leftmost = workspace.panes.leftmost_leaf();
    let mut pane = AnyPane::from(leftmost);
    let task = pane.init(&workspace.shared, app);
    let (mut state, initial_pane) = PaneGridState::new(pane);
    let mut tasks = vec![task.map(move |message| PaneMessage {
        id: initial_pane,
        message,
    })];

    build_rest(
        &workspace.shared,
        app,
        &workspace.panes,
        &mut state,
        initial_pane,
        &mut tasks,
        leftmost,
    )?;

    Ok((state, Task::batch(tasks)))
}

fn build_rest(
    workspace: &WorkspaceShared,
    app: &AppShared,
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
            let mut pane = AnyPane::from(right_leftmost);
            let task = pane.init(workspace, app);
            let (right_pane, split) = state
                .split((*axis).into(), left_pane, pane)
                .ok_or(anyhow::anyhow!("Could not split pane"))?;
            state.resize(split, *ratio);
            tasks.push(task.map(move |message| PaneMessage {
                id: right_pane,
                message,
            }));

            build_rest(workspace, app, a, state, left_pane, tasks, left_leftmost)?;
            build_rest(workspace, app, b, state, right_pane, tasks, right_leftmost)?;

            Ok(())
        }
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
