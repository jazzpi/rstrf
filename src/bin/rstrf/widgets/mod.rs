use iced::Element;
use iced::Theme;
use iced::widget::{button, text};

pub fn square_button<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Message,
    style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
) -> Element<'a, Message> {
    button(text(label).size(14))
        .style(style)
        .on_press(msg)
        .into()
}
