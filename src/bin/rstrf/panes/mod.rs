use anyhow::bail;
use iced::{
    Element, Length, Size, Task,
    widget::{container, pane_grid, text},
};

use crate::{
    app::WorkspaceEvent,
    panes::rfplot::RFPlot,
    workspace::{Pane, PaneTree},
};

pub mod rfplot;
pub mod sat_manager;

#[derive(Debug, Clone)]
pub enum Message {
    RFPlot(rfplot::Message),
    SatManager(sat_manager::Message),
    Workspace(WorkspaceEvent),
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
    fn update(&mut self, message: Message) -> Task<Message>;
    fn view(&self, size: Size) -> Element<'_, Message>;
    fn title(&self) -> &str;
}

pub struct Dummy {}

impl PaneWidget for Dummy {
    fn update(&mut self, _: Message) -> Task<Message> {
        Task::none()
    }

    fn view(&self, _: Size) -> Element<'_, Message> {
        // TODO: Show buttons to open other widgets?
        container(text("Loading...")).center(Length::Fill).into()
    }

    fn title(&self) -> &str {
        "Loading..."
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
pub fn from_tree(tree: PaneTree) -> anyhow::Result<(PaneGridState, Task<PaneMessage>)> {
    let leftmost = tree.leftmost_leaf();
    let (mut state, initial_pane) = PaneGridState::new(widget_for(leftmost));
    let mut tasks = vec![task_for(leftmost).map(move |message| PaneMessage {
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
        PaneTree::Leaf(leaf) => bail!("Unexpected leaf: {leaf:?}"),
        PaneTree::Split { split, a, b } => {
            let right_leftmost = b.leftmost_leaf();
            let (right_pane, _) = state
                .split((*split).into(), left_pane, widget_for(right_leftmost))
                .ok_or(anyhow::anyhow!("Could not split pane"))?;
            tasks.push(task_for(right_leftmost).map(move |message| PaneMessage {
                id: right_pane,
                message,
            }));

            build_rest(a, state, left_pane, tasks, left_leftmost)?;
            build_rest(b, state, right_pane, tasks, right_leftmost)?;

            Ok(())
        }
    }
}

fn widget_for(pane: &Pane) -> Box<dyn PaneWidget> {
    match pane {
        Pane::RFPlot { spectrogram: _ } => Box::new(RFPlot::new()),
        Pane::SatManager { .. } => Box::new(sat_manager::SatManager::new()),
    }
}

fn task_for(pane: &Pane) -> Task<Message> {
    match pane {
        Pane::RFPlot { spectrogram } => {
            RFPlot::new().update(rfplot::Message::LoadSpectrogram(spectrogram.clone()).into())
        }
        Pane::SatManager {
            elements,
            frequencies,
        } => sat_manager::SatManager::new().update(
            sat_manager::Message::LoadTLEs {
                tle_path: elements.clone(),
                freqs_path: frequencies.clone(),
            }
            .into(),
        ),
    }
}
