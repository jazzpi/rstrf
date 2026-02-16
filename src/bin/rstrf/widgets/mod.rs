use std::time::Duration;

use iced::Border;
use iced::Element;
use iced::Length;
use iced::Renderer;
use iced::Theme;
use iced::border::Radius;
use iced::widget::container;
use iced::widget::row;
use iced::widget::space;
use iced::widget::svg;
use iced::widget::tooltip;
use iced::widget::{button, text};

pub mod form;

use iced_aw::Menu;
use iced_aw::MenuBar;
use iced_aw::menu;
use rstrf::colormap::Colormap;

#[derive(Clone, Copy)]
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
    Colormap(Colormap),
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
            Icon::Colormap(colormap) => match colormap {
                Colormap::Magma => include_bytes!("../../../../resources/icons/cmap-magma.svg"),
                Colormap::Inferno => include_bytes!("../../../../resources/icons/cmap-inferno.svg"),
                Colormap::Plasma => include_bytes!("../../../../resources/icons/cmap-plasma.svg"),
                Colormap::Viridis => include_bytes!("../../../../resources/icons/cmap-viridis.svg"),
                Colormap::Cividis => include_bytes!("../../../../resources/icons/cmap-cividis.svg"),
                Colormap::Rocket => include_bytes!("../../../../resources/icons/cmap-rocket.svg"),
                Colormap::Mako => include_bytes!("../../../../resources/icons/cmap-mako.svg"),
                Colormap::Turbo => include_bytes!("../../../../resources/icons/cmap-turbo.svg"),
            },
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
    tooltip_button(
        responsive_icon(icon, style.clone()),
        tooltip_label,
        msg,
        style,
        26,
    )
}

pub fn labeled_icon_button<'a, Message: Clone + 'a>(
    icon: Icon,
    label: &'a str,
    tooltip_label: &'a str,
    msg: Message,
    style: impl Fn(&Theme, button::Status) -> button::Style + Clone + 'a,
) -> Element<'a, Message> {
    tooltip_button(
        row![
            container(responsive_icon(icon, style.clone())).width(Length::FillPortion(1)),
            row![text(label), space::horizontal()].width(Length::FillPortion(2)),
        ]
        .spacing(5),
        tooltip_label,
        msg,
        style,
        Length::Fill,
    )
}

pub fn responsive_icon<'a, Message: Clone + 'a>(
    icon: Icon,
    style: impl Fn(&Theme, button::Status) -> button::Style + Clone + 'a,
) -> Element<'a, Message> {
    svg(icon)
        .style(move |theme, status| {
            if let Icon::Colormap(_) = icon {
                // Don't override colormap colors
                return svg::Style { color: None };
            }
            let button_status = match status {
                svg::Status::Idle => button::Status::Active,
                svg::Status::Hovered => button::Status::Hovered,
            };
            svg::Style {
                color: Some(style(theme, button_status).text_color),
            }
        })
        .into()
}

pub fn tooltip_button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    tooltip_label: &'a str,
    msg: Message,
    style: impl Fn(&Theme, button::Status) -> button::Style + Clone + 'a,
    width: impl Into<Length>,
) -> Element<'a, Message> {
    tooltip(
        button(content)
            .width(width)
            .height(26)
            .padding(4)
            .style(style)
            .on_press(msg),
        container(text(tooltip_label))
            .padding(5)
            .style(container::dark),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(500))
    .into()
}

pub enum ToolbarButton<Message: Clone> {
    Icon {
        icon: Icon,
        tooltip: &'static str,
        msg: Message,
        style: fn(&Theme, button::Status) -> button::Style,
    },
    LabeledIcon {
        icon: Icon,
        label: &'static str,
        tooltip: &'static str,
        msg: Message,
        style: fn(&Theme, button::Status) -> button::Style,
    },
    Submenu {
        toplevel: Box<ToolbarButton<Message>>,
        submenu: Vec<ToolbarButton<Message>>,
    },
}

impl<'a, Message: Clone + 'a> ToolbarButton<Message> {
    pub fn view(&self) -> Element<'a, Message> {
        match self {
            ToolbarButton::Icon {
                icon,
                tooltip,
                msg,
                style,
            } => icon_button(*icon, tooltip, msg.clone(), *style),
            ToolbarButton::LabeledIcon {
                icon,
                label,
                tooltip,
                msg,
                style,
            } => labeled_icon_button(*icon, label, tooltip, msg.clone(), *style),
            ToolbarButton::Submenu { toplevel, .. } => toplevel.view(),
        }
    }
}

impl<'a, Message: Clone + 'a> From<ToolbarButton<Message>>
    for menu::Item<'a, Message, Theme, Renderer>
{
    fn from(button: ToolbarButton<Message>) -> Self {
        let view = button.view();
        match button {
            ToolbarButton::Icon { .. } => menu::Item::new(view),
            ToolbarButton::LabeledIcon { .. } => menu::Item::new(view),
            ToolbarButton::Submenu { submenu, .. } => menu::Item::with_menu(
                view,
                Menu::new(submenu.into_iter().map(|b| b.into()).collect())
                    .width(120.0)
                    .offset(6.0)
                    .spacing(5.0),
            ),
        }
    }
}

pub fn toolbar<'a, Message: Clone + 'a>(
    buttons: impl IntoIterator<Item = ToolbarButton<Message>>,
) -> Element<'a, Message> {
    MenuBar::new(buttons.into_iter().map(|b| b.into()).collect())
        .draw_path(menu::DrawPath::Backdrop)
        .close_on_background_click_global(true)
        .close_on_item_click_global(true)
        .padding(5.0)
        .style(
            |theme: &Theme, status: iced_aw::style::Status| menu::Style {
                path_border: Border {
                    radius: Radius::new(6.0),
                    ..Default::default()
                },
                path: theme.extended_palette().primary.weak.color.into(),
                ..iced_aw::style::menu_bar::primary(theme, status)
            },
        )
        .spacing(8)
        .width(Length::Fill)
        .into()
}
