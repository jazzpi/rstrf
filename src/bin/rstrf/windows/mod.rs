use iced::{Element, Task};
use rstrf::menu::MenuItem;

use crate::{
    app::{self, AppEvent, AppShared},
    windows::{rfplot::RFPlot, sat_manager::SatManager},
};

pub mod preferences;
pub mod rfplot;
pub mod sat_manager;

#[derive(Debug, Clone)]
pub enum Message {
    ToApp(Box<app::Message>),
    RFPlot(rfplot::Message),
    SatManager(sat_manager::Message),
    Preferences(preferences::Message),
}

impl From<WindowOut<rfplot::Message>> for Message {
    fn from(out: WindowOut<rfplot::Message>) -> Self {
        match out {
            WindowOut::Msg(msg) => Message::RFPlot(msg),
            WindowOut::Effect(effect) => match effect {
                WindowEffect::ToApp(app_msg) => Message::ToApp(Box::new(app_msg)),
            },
        }
    }
}

impl From<WindowOut<sat_manager::Message>> for Message {
    fn from(out: WindowOut<sat_manager::Message>) -> Self {
        match out {
            WindowOut::Msg(msg) => Message::SatManager(msg),
            WindowOut::Effect(effect) => match effect {
                WindowEffect::ToApp(app_msg) => Message::ToApp(Box::new(app_msg)),
            },
        }
    }
}

impl From<WindowOut<preferences::Message>> for Message {
    fn from(out: WindowOut<preferences::Message>) -> Self {
        match out {
            WindowOut::Msg(msg) => Message::Preferences(msg),
            WindowOut::Effect(effect) => match effect {
                WindowEffect::ToApp(app_msg) => Message::ToApp(Box::new(app_msg)),
            },
        }
    }
}

/// A cross-cutting effect that escapes a window's own message type and must be handled by the parent.
#[derive(Debug, Clone)]
pub enum WindowEffect {
    ToApp(app::Message),
}

/// Return type for window update functions: either a window-local continuation or an escaped effect.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum WindowOut<M> {
    Msg(M),
    // TODO: this allows a window to output an arbitrary app message. Probably this should be a
    // service call instead
    Effect(WindowEffect),
}

impl From<rfplot::Message> for WindowOut<rfplot::Message> {
    fn from(message: rfplot::Message) -> Self {
        WindowOut::Msg(message)
    }
}

impl From<sat_manager::Message> for WindowOut<sat_manager::Message> {
    fn from(message: sat_manager::Message) -> Self {
        WindowOut::Msg(message)
    }
}

impl From<preferences::Message> for WindowOut<preferences::Message> {
    fn from(message: preferences::Message) -> Self {
        WindowOut::Msg(message)
    }
}

pub trait Window<M: Clone> {
    fn title(&self) -> String;
    fn menu_bar(&self) -> Vec<MenuItem<WindowOut<M>>> {
        Vec::new()
    }
    fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, WindowOut<M>>;
    fn update(&mut self, message: M, app: &AppShared) -> Task<WindowOut<M>>;
    fn init(&mut self, _app: &AppShared) -> Task<WindowOut<M>> {
        Task::none()
    }
}

pub enum AnyWindow {
    SatManager(Box<SatManager>),
    RFPlot(Box<RFPlot>),
    Preferences(Box<preferences::Window>),
}

impl AnyWindow {
    pub fn title(&self) -> String {
        match self {
            AnyWindow::SatManager(w) => w.title(),
            AnyWindow::RFPlot(w) => w.title(),
            AnyWindow::Preferences(w) => w.title(),
        }
    }

    pub fn menu_bar(&self) -> Vec<MenuItem<Message>> {
        match self {
            AnyWindow::SatManager(w) => w
                .menu_bar()
                .into_iter()
                .map(|i| i.map_msg(Message::from))
                .collect(),
            AnyWindow::RFPlot(w) => w
                .menu_bar()
                .into_iter()
                .map(|i| i.map_msg(Message::from))
                .collect(),
            AnyWindow::Preferences(w) => w
                .menu_bar()
                .into_iter()
                .map(|i| i.map_msg(Message::from))
                .collect(),
        }
    }

    pub fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, Message> {
        match self {
            AnyWindow::SatManager(w) => w.view(app).map(Message::from),
            AnyWindow::RFPlot(w) => w.view(app).map(Message::from),
            AnyWindow::Preferences(w) => w.view(app).map(Message::from),
        }
    }

    pub fn init(&mut self, app: &AppShared) -> Task<Message> {
        match self {
            AnyWindow::RFPlot(w) => w.init(app).map(Message::from),
            AnyWindow::SatManager(w) => w.init(app).map(Message::from),
            AnyWindow::Preferences(w) => w.init(app).map(Message::from),
        }
    }

    pub fn update(&mut self, message: Message, app: &AppShared) -> Task<Message> {
        match (self, message) {
            (AnyWindow::SatManager(w), Message::SatManager(msg)) => {
                w.update(msg, app).map(Message::from)
            }
            (AnyWindow::RFPlot(w), Message::RFPlot(msg)) => w.update(msg, app).map(Message::from),
            (AnyWindow::Preferences(w), Message::Preferences(msg)) => {
                w.update(msg, app).map(Message::from)
            }
            _ => Task::none(),
        }
    }

    pub fn app_event(&mut self, event: AppEvent, app: &AppShared) -> Task<Message> {
        match self {
            AnyWindow::RFPlot(w) => w.app_event(event, app).map(Message::from),
            _ => Task::none(),
        }
    }
}
