use std::time::Duration;

use iced::Element;
use iced::Length;
use iced::Theme;
use iced::widget::Row;
use iced::widget::container;
use iced::widget::svg;
use iced::widget::tooltip;
use iced::widget::{button, text};

pub mod form;

pub use form::Form;

pub enum Icon {
    Close,
    Maximize,
    Restore,
    SplitHorizontally,
    SplitVertically,
    Sliders,
    ZoomReset,
    TogglePredictions,
    Eye,
    EyeOff,
    ViewColumns,
    Download,
    Grid,
    Crosshair,
}

impl From<Icon> for svg::Handle {
    fn from(icon: Icon) -> Self {
        let bytes: &[u8] = match icon {
            Icon::Close => {
                include_bytes!("../../../../resources/icons/material-symbols--close.svg")
            }
            Icon::Maximize => include_bytes!(
                "../../../../resources/icons/majesticons--arrows-expand-full-line.svg"
            ),
            Icon::Restore => include_bytes!(
                "../../../../resources/icons/majesticons--arrows-collapse-full-line.svg"
            ),
            Icon::SplitHorizontally => include_bytes!(
                "../../../../resources/icons/material-symbols--splitscreen-bottom.svg"
            ),
            Icon::SplitVertically => include_bytes!(
                "../../../../resources/icons/material-symbols--splitscreen-right.svg"
            ),
            Icon::Sliders => include_bytes!("../../../../resources/icons/octicon--sliders-16.svg"),
            Icon::ZoomReset => {
                include_bytes!("../../../../resources/icons/bytesize--zoom-reset.svg")
            }
            Icon::TogglePredictions => {
                include_bytes!("../../../../resources/icons/toggle-predictions.svg")
            }
            Icon::Eye => include_bytes!("../../../../resources/icons/majesticons--eye.svg"),
            Icon::EyeOff => include_bytes!("../../../../resources/icons/majesticons--eye-off.svg"),
            Icon::ViewColumns => {
                include_bytes!("../../../../resources/icons/majesticons--view-columns.svg")
            }
            Icon::Download => include_bytes!("../../../../resources/icons/bytesize--download.svg"),
            Icon::Grid => {
                include_bytes!(
                    "../../../../resources/icons/material-symbols--grid-on-outline-sharp.svg"
                )
            }
            Icon::Crosshair => include_bytes!("../../../../resources/icons/toggle-crosshair.svg"),
        };
        svg::Handle::from_memory(bytes)
    }
}

pub fn icon_button<'a, Message: Clone + 'a>(
    icon: Icon,
    tooltip_label: &'a str,
    msg: Message,
    style: impl Fn(&Theme, button::Status) -> button::Style + Clone + 'a,
) -> Element<'a, Message> {
    let svg_style = style.clone();
    tooltip(
        button(svg(icon).style(move |theme, status| {
            let button_status = match status {
                svg::Status::Idle => button::Status::Active,
                svg::Status::Hovered => button::Status::Hovered,
            };
            svg::Style {
                color: Some(svg_style(theme, button_status).text_color),
            }
        }))
        .width(26)
        .height(26)
        .padding(4)
        .style(style)
        .on_press(msg),
        container(text(tooltip_label))
            .padding(5)
            .style(container::dark),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_secs(1))
    .into()
}

pub fn toolbar<'a, Message: Clone + 'a>(
    buttons: impl IntoIterator<Item = Element<'a, Message>>,
) -> Element<'a, Message> {
    Row::with_children(buttons)
        .spacing(8)
        .width(Length::Fill)
        .into()
}
