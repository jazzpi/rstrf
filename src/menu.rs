use iced::{
    Border, Color, Element, Length, Theme, alignment,
    border::Radius,
    widget::{button, text},
};
use iced_aw::{
    Menu,
    menu::{DrawPath, Style},
    menu_bar, menu_items, style as awstyle,
};

#[derive(Debug, Clone)]
pub enum Message {
    WorkspacePick,
    WorkspaceSave,
}

// Adapter from the iced_aw example

fn base_button<'a>(content: impl Into<Element<'a, Message>>) -> button::Button<'a, Message> {
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

fn debug_button(
    label: &str,
    msg: Option<Message>,
    width: Option<Length>,
    height: Option<Length>,
) -> Element<'_, Message> {
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

fn button_s(label: &str, msg: Option<Message>) -> Element<'_, Message> {
    debug_button(label, msg, Some(Length::Shrink), Some(Length::Shrink))
}

fn button_f(label: &str, msg: Option<Message>) -> Element<'_, Message> {
    debug_button(label, msg, Some(Length::Fill), Some(Length::Shrink))
}

pub fn view<'a>() -> Element<'a, Message> {
    let menu = |items| Menu::new(items).width(180.0).offset(6.0).spacing(5.0);
    menu_bar!((
        button_s("Workspace", None),
        menu(menu_items!(
            (button_f("Open", Some(Message::WorkspacePick))),
            (button_f("Save", Some(Message::WorkspaceSave))),
        ))
    ))
    .draw_path(DrawPath::Backdrop)
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
    .into()
}
