use iced::{
    Element, Length, Size, Task,
    widget::{button, column, container, text},
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppShared,
    panes::{self, Pane, PaneTree, PaneWidget, rfplot::RFPlot, sat_manager::SatManager},
    workspace::WorkspaceShared,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dummy;

impl PaneWidget for Dummy {
    fn update(
        &mut self,
        _: panes::Message,
        _: &WorkspaceShared,
        _: &AppShared,
    ) -> Task<panes::Message> {
        Task::none()
    }

    fn view(&self, _: Size, _: &WorkspaceShared, _: &AppShared) -> Element<'_, panes::Message> {
        let pane = |name, pane| {
            button(text(name))
                .width(Length::Fill)
                .style(button::primary)
                .on_press(panes::Message::ReplacePane(pane))
        };
        let content: Element<'_, panes::Message> = column![
            pane("RFPlot", Pane::RFPlot(Box::new(RFPlot::new()))),
            pane("SatManager", Pane::SatManager(Box::new(SatManager::new()))),
        ]
        .spacing(20)
        .into();
        let content = container(content)
            .center_x(Length::Fixed(300.0))
            .center_y(Length::Fill);
        container(content).center(Length::Fill).into()
    }

    fn title(&self) -> String {
        "Loading...".into()
    }

    fn to_tree(&self) -> PaneTree {
        PaneTree::Leaf(Pane::Dummy(Box::new(self.clone())))
    }
}
