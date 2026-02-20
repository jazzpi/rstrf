use std::path::PathBuf;

use chrono::{DateTime, Utc};
use iced::{
    Element, Font, Task,
    widget::{button, column, pick_list, row, text},
};
use rstrf::{
    spectrogram::{self, IqFormat, SampleFormat},
    util::pick_file,
};
use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::{
    app::AppShared,
    panes::PaneWidget,
    widgets::form::{date_input, number_input},
    workspace::WorkspaceShared,
};

#[derive(Debug, Clone)]
pub enum Message {
    PickFile,
    SetFile(PathBuf),
    DoImport,
    SetSampleFormat(SampleFormat),
    SetSampleRate(f32),
    SetCenterFrequency(f32),
    SetStartTime(DateTime<Utc>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recordings {
    format: IqFormat,
    header: spectrogram::Header,
    path: Option<PathBuf>,
}

impl Default for Recordings {
    fn default() -> Self {
        Self {
            format: IqFormat {
                samples: SampleFormat::CS8,
                sample_rate: 1e6,
            },
            header: spectrogram::Header {
                start_time: Utc::now(),
                freq: 0.0,
                bw: 1e6,
                length: 1.0,
                nchan: 8192,
            },
            path: None,
        }
    }
}

impl PaneWidget for Recordings {
    fn update(
        &mut self,
        message: super::Message,
        _workspace: &WorkspaceShared,
        _app: &AppShared,
    ) -> Task<super::Message> {
        match message {
            super::Message::Recordings(message) => match message {
                Message::PickFile => Task::future(pick_file(&[(
                    "IQ files",
                    &[
                        "iq", "bin", "cs", "cf", "cs8", "cs16", "cs32", "cs64", "cf16", "cf32",
                        "cf64",
                    ],
                )]))
                .and_then(|p| Task::done(Message::SetFile(p).into())),
                Message::DoImport => {
                    let Some(input) = self.path.clone() else {
                        log::error!("No file selected");
                        return Task::none();
                    };
                    let format = self.format.clone();
                    let header = self.header.clone();
                    Task::future(
                        async move { spectrogram::load_iq_file(&input, format, &header).await },
                    )
                    .then(|result| match result {
                        Ok(spec) => {
                            // TODO
                            log::info!("Loaded spectrogram: {:?}", spec);
                            Task::none()
                        }
                        Err(err) => {
                            log::error!("Failed to load IQ file: {}", err);
                            Task::none()
                        }
                    })
                }
                Message::SetFile(path) => {
                    self.path = Some(path);
                    // TODO: Try to detect format, start time etc. from file name
                    Task::none()
                }
                Message::SetSampleFormat(sample_format) => {
                    self.format.samples = sample_format;
                    Task::none()
                }
                Message::SetSampleRate(sample_rate) => {
                    self.format.sample_rate = sample_rate;
                    self.header.bw = sample_rate;
                    Task::none()
                }
                Message::SetCenterFrequency(center_freq) => {
                    self.header.freq = center_freq;
                    Task::none()
                }
                Message::SetStartTime(start_time) => {
                    self.header.start_time = start_time;
                    Task::none()
                }
            },
            _ => Task::none(),
        }
    }

    fn view(
        &self,
        _size: iced::Size,
        _workspace: &WorkspaceShared,
        _app: &AppShared,
    ) -> Element<'_, super::Message> {
        let path = match &self.path {
            Some(p) => p.to_string_lossy().to_string(),
            None => "No file selected".into(),
        };
        let path = row![
            text("Path:"),
            text(path).font(Font::MONOSPACE),
            button("Browse")
                .on_press(Message::PickFile.into())
                .style(button::primary)
        ]
        .spacing(10);
        let sample_format = row![
            text("Sample Format:"),
            pick_list(SampleFormat::VARIANTS, Some(self.format.samples), |f| {
                Message::SetSampleFormat(f).into()
            })
        ]
        .spacing(10);
        let sample_rate = row![
            text("Sample Rate:"),
            number_input("", self.format.sample_rate, 0, |r| {
                Message::SetSampleRate(r).into()
            })
        ]
        .spacing(10);
        let center_freq = row![
            text("Center Frequency:"),
            number_input("", self.header.freq, 0, |f| {
                Message::SetCenterFrequency(f).into()
            })
        ]
        .spacing(10);
        let start_time = row![
            text("Start Time:"),
            date_input("", self.header.start_time, |d| {
                Message::SetStartTime(d).into()
            })
        ]
        .spacing(10);

        column![
            path,
            sample_format,
            sample_rate,
            center_freq,
            start_time,
            button("Import").on_press(Message::DoImport.into())
        ]
        .spacing(20)
        .into()
    }

    fn title(&self) -> String {
        "Recordings".into()
    }

    fn to_tree(&self) -> super::PaneTree {
        super::PaneTree::Leaf(super::Pane::Recordings(Box::new(self.clone())))
    }
}
