use iced::{
    Element, Length, Task,
    alignment::{Horizontal, Vertical},
    widget::{container, text},
};

#[derive(Debug, Clone)]
pub enum Message {
    // TODO
}

pub struct SatManager {
    // TODO
}

impl SatManager {
    pub fn new() -> Self {
        Self {}
    }

    pub fn view(&self) -> Element<'_, Message> {
        container(text("Satellite Manager - TODO"))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    }

    pub fn update(&mut self, _message: Message) -> Task<Message> {
        Task::none()
    }
}
