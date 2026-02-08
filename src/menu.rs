use iced::{
    Border, Color, Element, Length, Renderer, Theme, alignment,
    border::Radius,
    widget::{button, text},
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
