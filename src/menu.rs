use iced::{
    Border, Element, Length, Renderer, Theme, alignment,
    border::Radius,
    widget::{self, button, container, text},
};
use iced_aw::{
    Menu, MenuBar,
    menu::{DrawPath, Item, Style},
    style as awstyle,
};

// Adapted from the iced_aw example

mod style {
    use iced::{
        Border, Color, Theme,
        theme::palette::Extended,
        widget::button::{Status, Style},
    };

    fn base(palette: &Extended, status: Status) -> Style {
        let base = Style {
            text_color: palette.background.base.text,
            ..Style::default()
        };
        match status {
            Status::Active => base.with_background(Color::TRANSPARENT),
            Status::Hovered => base.with_background(palette.primary.weak.color),
            Status::Disabled => base.with_background(palette.secondary.weak.color),
            Status::Pressed => base.with_background(palette.primary.strong.color),
        }
    }

    pub(crate) fn toplevel(theme: &Theme, status: Status) -> Style {
        let palette = theme.extended_palette();
        let base = base(palette, status);
        match status {
            Status::Active => base.with_background(palette.background.neutral.color),
            _ => base,
        }
    }

    pub(crate) fn sublevel(theme: &Theme, status: Status) -> Style {
        let palette = theme.extended_palette();
        Style {
            border: Border::default().rounded(6.0),
            ..base(palette, status)
        }
    }
}

fn base_button<'a, Message: Clone>(
    content: impl Into<Element<'a, Message>>,
) -> button::Button<'a, Message> {
    button(content).padding([4, 8])
}

fn menu_button<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Option<Message>,
    width: Option<Length>,
    height: Option<Length>,
) -> button::Button<'a, Message> {
    base_button(
        text(label)
            .height(height.unwrap_or(Length::Shrink))
            .align_y(alignment::Vertical::Center),
    )
    .width(width.unwrap_or(Length::Shrink))
    .height(height.unwrap_or(Length::Shrink))
    .on_press_maybe(msg)
}

pub fn toplevel<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Option<Message>,
) -> Element<'a, Message> {
    menu_button(label, msg, Some(Length::Shrink), Some(Length::Shrink))
        .style(style::toplevel)
        .into()
}

pub fn sublevel<'a, Message: Clone + 'a>(
    label: &'a str,
    msg: Option<Message>,
) -> Element<'a, Message> {
    menu_button(label, msg, Some(Length::Fill), Some(Length::Shrink))
        .style(style::sublevel)
        .into()
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
    // MenuBar seems to ignore the .width(Length::Fill) call
    container(
        bar.draw_path(DrawPath::FakeHovering)
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
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .style(|theme| container::Style {
        background: Some(theme.extended_palette().background.base.color.into()),
        ..container::Style::default()
    })
    .into()
}

pub fn submenu<'a, Message: Clone + 'a>(
    items: Vec<Item<'a, Message, Theme, Renderer>>,
) -> Menu<'a, Message, Theme, Renderer> {
    Menu::new(items).width(180.0).offset(6.0).spacing(5.0)
}
