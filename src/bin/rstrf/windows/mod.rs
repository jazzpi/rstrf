use iced::{Element, Subscription, Task};

use crate::app::{self, AppShared};
use crate::workspace::WindowSpec;

pub mod preferences;
pub mod rfplot;
pub mod sat_manager;

#[derive(Debug, Clone)]
pub enum Message {
    ToApp(Box<app::Message>),
    RFPlot(crate::panes::rfplot::Message),
    SatManager(crate::panes::sat_manager::Message),
    Preferences(preferences::Message),
}

impl From<preferences::Message> for Message {
    fn from(msg: preferences::Message) -> Self {
        Message::Preferences(msg)
    }
}

pub trait Window {
    fn title(&self) -> String;
    fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, Message>;
    fn update(&mut self, message: Message, app: &AppShared) -> Task<Message>;
    fn subscription(&self) -> Subscription<Message> {
        Subscription::none()
    }
    fn app_event(&mut self, _event: app::AppEvent, _app: &AppShared) -> Task<Message> {
        Task::none()
    }
    /// Return the serializable spec for this window, used when saving the workspace.
    /// Returns None for windows that aren't part of the workspace (e.g. Preferences).
    fn to_window_spec(&self) -> Option<WindowSpec> {
        None
    }
}
