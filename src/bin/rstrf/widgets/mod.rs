use std::time::Duration;

use iced::Element;
use iced::Theme;
use iced::widget::container;
use iced::widget::tooltip;
use iced::widget::{button, text};

pub fn square_button<'a, Message: Clone + 'a>(
    label: &'a str,
    tooltip_label: &'a str,
    msg: Message,
    style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
) -> Element<'a, Message> {
    tooltip(
        button(text(label).size(14)).style(style).on_press(msg),
        container(text(tooltip_label))
            .padding(5)
            .style(|theme| container::dark(theme)),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_secs(1))
    .into()
}
