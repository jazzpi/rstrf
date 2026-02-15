use iced::{Element, Subscription, Task};

use crate::app::{self, AppShared};

pub mod workspace;

#[derive(Debug, Clone)]
pub enum Message {
    ToApp(Box<app::Message>),
    Workspace(workspace::Message),
}

impl From<workspace::Message> for Message {
    fn from(msg: workspace::Message) -> Self {
        Message::Workspace(msg)
    }
}

pub trait Window {
    fn title(&self) -> String;
    fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, Message>;
    fn update(&mut self, message: Message, app: &AppShared) -> Task<Message>;
    fn subscription(&self) -> Subscription<Message> {
        Subscription::none()
    }
}
