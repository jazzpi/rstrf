use iced::{Element, Size, Task};

use crate::app::WorkspaceEvent;

pub mod rfplot;
pub mod sat_manager;

#[derive(Debug, Clone)]
pub enum Message {
    RFPlot(rfplot::Message),
    SatManager(sat_manager::Message),
    Workspace(WorkspaceEvent),
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
