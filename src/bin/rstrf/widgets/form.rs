use iced::{
    Element, Length,
    widget::{Column, TextInput, button, container, row, text, text_input},
};
use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

#[derive(Debug, Clone)]
pub enum Message {
    Submit,
    UpdateField(usize, String),
}

#[derive(Clone, PartialEq)]
pub enum Field {
    Text(String),
    Password(String),
}

impl Field {
    pub fn view(&self, idx: usize) -> Element<'_, Message> {
        match self {
            Field::Text(value) => text_input("", value)
                .on_input(move |s| Message::UpdateField(idx, s))
                .on_submit(Message::Submit)
                .into(),
            Field::Password(value) => text_input("", value)
                .secure(true)
                .on_input(move |s| Message::UpdateField(idx, s))
                .on_submit(Message::Submit)
                .into(),
        }
    }

    fn value(&self) -> String {
        match self {
            Field::Text(s) | Field::Password(s) => s.clone(),
        }
    }
}

impl Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Field::Text(s) => f.debug_tuple("Text").field(&s).finish(),
            Field::Password(_) => f.debug_tuple("Password").field(&"********").finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    fields: Vec<(String, Field)>,
    submit_label: String,
}

impl Form {
    pub fn new(fields: Vec<(String, Field)>, submit_label: String) -> Self {
        Self {
            fields,
            submit_label,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        Column::new()
            .extend(
                self.fields
                    .iter()
                    .enumerate()
                    .map(|(idx, field)| -> Element<'_, Message> {
                        row![text(&field.0).width(Length::Shrink), field.1.view(idx)]
                            .spacing(10)
                            .into()
                    }),
            )
            .push(
                container(button(text(&self.submit_label)).on_press(Message::Submit))
                    .center_x(Length::Fill)
                    .padding(10),
            )
            .spacing(10)
            .into()
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::UpdateField(idx, value) => {
                if let Some((_, field)) = self.fields.get_mut(idx) {
                    match field {
                        Field::Text(s) | Field::Password(s) => *s = value,
                    }
                }
            }
            Message::Submit => {
                log::warn!("Form submission should be handled by the parent widget");
            }
        }
    }

    pub fn field_values(&self) -> Vec<String> {
        self.fields.iter().map(|(_, field)| field.value()).collect()
    }
}

pub fn number_input<'a, T, Message>(
    placeholder: &str,
    value: T,
    precision: usize,
    on_input: impl Fn(T) -> Message + Clone + 'a,
) -> TextInput<'a, Message>
where
    T: Display + FromStr + Clone + 'a,
    Message: Clone,
{
    text_input(placeholder, format!("{:.1$}", value, precision).as_str()).on_input(move |s| {
        s.parse::<T>().map(on_input.clone()).unwrap_or_else({
            let value = value.clone();
            let on_input = on_input.clone();
            move |_| on_input(value)
        })
    })
}
