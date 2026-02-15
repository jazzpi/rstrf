use std::{fmt::Display, str::FromStr};

use iced::{
    Element, Font, Length, Task,
    alignment::Vertical,
    font,
    widget::{Space, button, column, container, pick_list, row, rule, space, text, text_input},
};
use space_track::SpaceTrack;
use strum::VariantArray;

use crate::{
    app::AppShared,
    config::{BuiltinTheme, Config},
    widgets::form::number_input,
};

#[derive(Debug, Clone)]
pub enum Message {
    SpacetrackUpdateUsername(String),
    SpacetrackUpdatePassword(String),
    SpacetrackVerify,
    SpacetrackVerified(bool),
    SpacetrackLogout,
    SiteLatitude(f64),
    SiteLongitude(f64),
    SiteAltitude(f64),
    ThemeSelected(BuiltinTheme),
    Submit,
}

pub struct Window {
    working_copy: Config,
    spacetrack_verifying: bool,
    spacetrack_verified: Option<bool>,
}

const BOLD: Font = Font {
    family: font::Family::SansSerif,
    weight: font::Weight::Bold,
    stretch: font::Stretch::Normal,
    style: font::Style::Normal,
};

impl Window {
    pub fn new(app: &AppShared) -> Self {
        Self {
            working_copy: app.config.clone(),
            spacetrack_verifying: false,
            spacetrack_verified: None,
        }
    }

    fn text_field<'a>(
        label: &'a str,
        value: &str,
        on_input: impl Fn(String) -> Message + 'a,
        secure: bool,
    ) -> Element<'a, Message> {
        let label_text = text(label).font(BOLD).width(Length::FillPortion(1));
        let input = text_input(label, value)
            .secure(secure)
            .on_input(on_input)
            .padding(10)
            .width(Length::FillPortion(3));
        row![label_text, input]
            .spacing(10)
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .into()
    }

    fn number_field<'a, T>(
        label: &'a str,
        value: T,
        precision: usize,
        on_input: impl Fn(T) -> Message + Clone + 'a,
    ) -> Element<'a, Message>
    where
        T: Display + FromStr + Clone + 'a,
    {
        let label_text = text(label).font(BOLD).width(Length::FillPortion(1));
        let input = number_input("", value, precision, on_input)
            .padding(10)
            .width(Length::FillPortion(3));
        row![label_text, input]
            .spacing(10)
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .into()
    }

    fn dropdown_field<'a, T>(
        label: &'a str,
        value: Option<T>,
        options: &'a [T],
        on_selected: impl Fn(T) -> Message + 'a,
    ) -> Element<'a, Message>
    where
        T: ToString + PartialEq + Clone,
    {
        let label_text = text(label).font(BOLD).width(Length::FillPortion(1));
        let input = pick_list(options, value, on_selected)
            .padding(10)
            .width(Length::FillPortion(3));
        row![label_text, input]
            .spacing(10)
            .width(Length::Fill)
            .align_y(Vertical::Center)
            .into()
    }

    fn view_group<'a>(
        title: &'a str,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        container(
            column![
                text(title).font(BOLD).size(20),
                rule::horizontal(2),
                content.into()
            ]
            .padding(10)
            .spacing(5),
        )
        .style(container::bordered_box)
        .into()
    }

    fn view_spacetrack(&self) -> Element<'_, Message> {
        let (username, password) = self
            .working_copy
            .space_track_creds
            .clone()
            .unwrap_or(("".into(), "".into()));
        let verify_button = if self.spacetrack_verifying {
            button("Verifying...").padding(5).style(button::secondary)
        } else {
            button("Verify")
                .on_press(Message::SpacetrackVerify)
                .padding(5)
                .style(button::primary)
        };
        let verification_status: Element<_> = if let Some(verified) = self.spacetrack_verified {
            let c = if verified {
                container(text("Verified")).style(container::success)
            } else {
                container(text("Verification failed")).style(container::danger)
            };
            c.padding(5).into()
        } else {
            space::horizontal().into()
        };
        let logout_button: Element<_> = if self.working_copy.space_track_creds.is_some() {
            button("Logout")
                .on_press(Message::SpacetrackLogout)
                .padding(5)
                .style(button::danger)
                .into()
        } else {
            Space::new().into()
        };
        Self::view_group(
            "Space-Track Credentials",
            column![
                Self::text_field(
                    "Username",
                    &username,
                    Message::SpacetrackUpdateUsername,
                    false
                ),
                Self::text_field(
                    "Password",
                    &password,
                    Message::SpacetrackUpdatePassword,
                    true
                ),
                row![logout_button, verify_button, verification_status]
                    .spacing(10)
                    .align_y(Vertical::Center)
            ],
        )
    }

    fn view_site(&self) -> Element<'_, Message> {
        let site = self.working_copy.site.clone().unwrap_or_default();
        Self::view_group(
            "Ground Site",
            column![
                Self::number_field(
                    "Latitude (°)",
                    site.latitude.to_degrees(),
                    4,
                    Message::SiteLatitude
                ),
                Self::number_field(
                    "Longitude (°)",
                    site.longitude.to_degrees(),
                    4,
                    Message::SiteLongitude
                ),
                Self::number_field("Altitude (km)", site.altitude, 3, Message::SiteAltitude),
            ],
        )
    }

    fn view_appearance(&self) -> Element<'_, Message> {
        Self::view_group(
            "Appearance",
            column![Self::dropdown_field(
                "Theme",
                Some(self.working_copy.theme),
                BuiltinTheme::VARIANTS,
                Message::ThemeSelected
            )],
        )
    }
}

impl super::Window for Window {
    fn title(&self) -> String {
        "Preferences".into()
    }

    fn view<'a>(&'a self, _: &'a crate::app::AppShared) -> Element<'a, super::Message> {
        let result: Element<Message> = column![
            self.view_spacetrack(),
            self.view_site(),
            self.view_appearance(),
            button("Apply")
                .on_press(Message::Submit)
                .padding(10)
                .style(button::primary),
        ]
        .spacing(10)
        .padding(10)
        .into();
        result.map(super::Message::Preferences)
    }

    fn update(
        &mut self,
        message: super::Message,
        _app: &crate::app::AppShared,
    ) -> Task<super::Message> {
        match message {
            super::Message::Preferences(message) => match message {
                Message::SpacetrackUpdateUsername(name) => {
                    self.working_copy.space_track_creds = Some((
                        name,
                        self.working_copy
                            .space_track_creds
                            .as_ref()
                            .map(|(_, pass)| pass.clone())
                            .unwrap_or_default(),
                    ));
                    self.spacetrack_verified = None;
                    Task::none()
                }
                Message::SpacetrackUpdatePassword(pass) => {
                    self.working_copy.space_track_creds = Some((
                        self.working_copy
                            .space_track_creds
                            .as_ref()
                            .map(|(user, _)| user.clone())
                            .unwrap_or_default(),
                        pass,
                    ));
                    self.spacetrack_verified = None;
                    Task::none()
                }
                Message::SpacetrackVerify => {
                    let Some((user, pass)) = self.working_copy.space_track_creds.clone() else {
                        log::error!("No credentials provided");
                        return Task::none();
                    };
                    log::debug!("Verifying SpaceTrack credentials for user '{}'", user);
                    self.spacetrack_verifying = true;
                    let mut space_track = SpaceTrack::new(space_track::Credentials {
                        identity: user,
                        password: pass,
                    });
                    Task::future(async move {
                        let verified = match space_track
                            .boxscore(space_track::Config {
                                limit: Some(1),
                                ..space_track::Config::new()
                            })
                            .await
                        {
                            Ok(b) => {
                                log::debug!("got boxscore: {:?}", b);
                                true
                            }
                            Err(err) => {
                                log::error!("Failed to verify SpaceTrack credentials: {:?}", err);
                                false
                            }
                        };
                        Message::SpacetrackVerified(verified).into()
                    })
                }
                Message::SpacetrackVerified(verified) => {
                    self.spacetrack_verifying = false;
                    self.spacetrack_verified = Some(verified);
                    Task::none()
                }
                Message::SpacetrackLogout => {
                    self.working_copy.space_track_creds = None;
                    self.spacetrack_verified = None;
                    Task::none()
                }
                Message::SiteLatitude(lat) => {
                    self.working_copy.site.get_or_insert_default().latitude = lat.to_radians();
                    Task::none()
                }
                Message::SiteLongitude(lon) => {
                    self.working_copy.site.get_or_insert_default().longitude = lon.to_radians();
                    Task::none()
                }
                Message::SiteAltitude(alt) => {
                    self.working_copy.site.get_or_insert_default().altitude = alt;
                    Task::none()
                }
                Message::ThemeSelected(theme) => {
                    self.working_copy.theme = theme;
                    Task::none()
                }
                Message::Submit => Task::done(super::Message::ToApp(Box::new(
                    crate::app::Message::UpdateConfig(self.working_copy.clone()),
                ))),
            },
            _ => Task::none(),
        }
    }
}
