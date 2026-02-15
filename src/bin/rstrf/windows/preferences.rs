use iced::{
    Element, Font, Length, Task,
    alignment::Vertical,
    font,
    widget::{Space, button, column, container, row, rule, space, text, text_input},
};
use space_track::SpaceTrack;

use crate::{app::AppShared, config::Config};

#[derive(Debug, Clone)]
pub enum Message {
    SpacetrackUpdateUsername(String),
    SpacetrackUpdatePassword(String),
    SpacetrackVerify,
    SpacetrackVerified(bool),
    SpacetrackLogout,
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
        container(
            column![
                text("SpaceTrack Credentials").font(BOLD).size(20),
                rule::horizontal(2),
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
            ]
            .padding(10)
            .spacing(5),
        )
        .style(container::bordered_box)
        .into()
    }
}

impl super::Window for Window {
    fn title(&self) -> String {
        "Preferences".into()
    }

    fn view<'a>(&'a self, _: &'a crate::app::AppShared) -> Element<'a, super::Message> {
        let result: Element<Message> = column![
            self.view_spacetrack(),
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
                Message::Submit => Task::done(super::Message::ToApp(Box::new(
                    crate::app::Message::UpdateConfig(self.working_copy.clone()),
                ))),
            },
            _ => Task::none(),
        }
    }
}
