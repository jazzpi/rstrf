use iced::{
    Border, Color, Element, Length, Renderer, Theme, alignment,
    border::Radius,
    widget::{self, button, container, text},
};
use iced_aw::{
    Menu, MenuBar,
    menu::{DrawPath, Item, Style},
    style as awstyle,
};

// Adapted from the iced_aw example

fn base_button<'a, Message: Clone>(
    content: impl Into<Element<'a, Message>>,
) -> button::Button<'a, Message> {
    button(content).padding([4, 8]).style(|theme, status| {
        use button::{Status, Style};

        let palette = theme.extended_palette();
        let base = Style {
            text_color: palette.background.base.text,
            border: Border::default().rounded(6.0),
            ..Style::default()
        };
        match status {
            Status::Active => base.with_background(Color::TRANSPARENT),
            Status::Hovered => base.with_background(palette.primary.weak.color),
            Status::Disabled => base.with_background(palette.secondary.weak.color),
            Status::Pressed => base.with_background(palette.primary.strong.color),
        }
    })
}

pub fn menu_button<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Option<Message>,
    width: Option<Length>,
    height: Option<Length>,
) -> Element<'a, Message> {
    base_button(
        text(label)
            .height(height.unwrap_or(Length::Shrink))
            .align_y(alignment::Vertical::Center),
    )
    .width(width.unwrap_or(Length::Shrink))
    .height(height.unwrap_or(Length::Shrink))
    .on_press_maybe(msg)
    .into()
}

pub fn button_s<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Option<Message>,
) -> Element<'a, Message> {
    menu_button(label, msg, Some(Length::Shrink), Some(Length::Shrink))
}

pub fn button_f<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Option<Message>,
) -> Element<'a, Message> {
    menu_button(label, msg, Some(Length::Fill), Some(Length::Shrink))
}

pub fn checkbox<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Option<Message>,
    is_checked: bool,
) -> Element<'a, Message> {
    let mut checkbox = widget::checkbox::<Message, Theme, Renderer>(is_checked)
        .style(|theme: &Theme, status| {
            use widget::checkbox::{Status, Style};

            let palette = theme.extended_palette();
            let base = Style {
                background: palette.background.weak.color.into(),
                icon_color: palette.primary.strong.color,
                border: Border::default().rounded(4.0),
                text_color: None,
            };
            match status {
                Status::Active { is_checked: _ } => Style {
                    background: palette.background.neutral.color.into(),
                    ..base
                },
                Status::Hovered { is_checked: _ } => Style {
                    background: palette.background.strong.color.into(),
                    text_color: Some(palette.primary.weak.color),
                    ..base
                },
                Status::Disabled { is_checked: _ } => Style {
                    background: palette.secondary.weak.color.into(),
                    icon_color: palette.secondary.strong.color,
                    ..base
                },
            }
        })
        .label(label)
        .spacing(10)
        .size(20);
    if let Some(msg) = msg.clone() {
        checkbox = checkbox.on_toggle(move |_| msg.clone());
    }
    container(checkbox)
        .padding([4, 8])
        .align_y(alignment::Vertical::Center)
        .align_left(Length::Fill)
        .into()
}

pub fn view_menu<'a, Message: 'a>(
    bar: MenuBar<'a, Message, Theme, Renderer>,
) -> Element<'a, Message> {
    bar.draw_path(DrawPath::Backdrop)
        .close_on_background_click_global(true)
        .close_on_item_click_global(true)
        .padding(5.0)
        .style(|theme: &Theme, status: awstyle::Status| Style {
            path_border: Border {
                radius: Radius::new(6.0),
                ..Default::default()
            },
            path: theme.extended_palette().primary.weak.color.into(),
            ..awstyle::menu_bar::primary(theme, status)
        })
        .width(Length::Fill)
        .into()
}

pub fn submenu<'a, Message: Clone + 'a>(
    items: Vec<Item<'a, Message, Theme, Renderer>>,
) -> Menu<'a, Message, Theme, Renderer> {
    Menu::new(items).width(180.0).offset(6.0).spacing(5.0)
}
