use std::{collections::VecDeque, path::PathBuf};

use iced::{Subscription, Task, window};
use ndarray::Array1;
use rstrf::{
    coord::{DataAbsoluteToDataNormalized, data_absolute, data_normalized},
    orbit::{PassPrediction, predict_satellites},
    spectrogram::SpectrogramBounds,
    util::minmax,
};

use crate::{
    PassPngArgs,
    app::{self, AppShared},
    windows,
};

#[derive(Clone)]
struct PassJob {
    view: data_normalized::Rectangle,
    path: PathBuf,
}

enum State {
    WaitingForRFPlot,
    WaitingForPredictions,
    WaitingForView(PathBuf, VecDeque<PassJob>, usize),
    WaitingForCapture(PathBuf, VecDeque<PassJob>),
}

pub struct PassPngMode {
    window_id: window::Id,
    args: PassPngArgs,
    state: State,
}

#[derive(Debug, Clone)]
pub enum Message {
    // TODO: DRY with app::Message::RFPlotReady
    RFPlotReady(window::Id, SpectrogramBounds),
    PredictionsReady {
        spec_bounds: SpectrogramBounds,
        times: Array1<f64>,
        transmitters: Vec<f64>,
        passes: Vec<PassPrediction>,
    },
    FrameReady,
    ScreenshotSaved(PathBuf),
}

impl From<Message> for app::Message {
    fn from(msg: Message) -> Self {
        app::Message::PassPng(msg)
    }
}

impl PassPngMode {
    pub fn new(window_id: window::Id, args: PassPngArgs) -> Self {
        Self {
            window_id,
            args,
            state: State::WaitingForRFPlot,
        }
    }

    pub fn subscription(&self) -> Option<Subscription<Message>> {
        if matches!(self.state, State::WaitingForView(_, _, _)) {
            Some(window::frames().map(|_| Message::FrameReady))
        } else {
            None
        }
    }

    pub fn update(&mut self, message: Message, app: &AppShared) -> Task<app::Message> {
        match message {
            Message::RFPlotReady(window_id, spec_bounds) => {
                if window_id != self.window_id {
                    return Task::none();
                }
                let norad_id = self.args.norad_id;

                let satellite = app
                    .satellites
                    .iter()
                    .find(|(sat, _)| sat.norad_id() == norad_id)
                    .map(|(sat, _)| sat.clone());
                let Some(satellite) = satellite else {
                    log::error!("pass-png: satellite {norad_id} not found in catalog");
                    return iced::exit();
                };
                if satellite.transmitters.is_empty() {
                    log::error!("pass-png: satellite {norad_id} has no transmitters");
                    return iced::exit();
                }

                self.state = State::WaitingForPredictions;

                let Some(site) = app.site() else {
                    log::error!("pass-png: no site available");
                    return iced::exit();
                };
                let time_range = spec_bounds.time_range.clone();
                let transmitters = satellite.transmitters.clone();
                let predict_task = Task::future(async move {
                    tokio::task::spawn_blocking(move || {
                        predict_satellites(&[satellite], time_range, &site)
                    })
                    .await
                })
                .then(move |result| {
                    let Ok(predictions) = result else {
                        log::error!("pass-png: failed to compute predictions");
                        return iced::exit();
                    };
                    let passes = predictions.for_id(norad_id).to_owned();
                    if passes.is_empty() {
                        log::info!(
                            "pass-png: no passes for satellite {norad_id} in spectrogram window"
                        );
                        return iced::exit();
                    }
                    Task::done(
                        Message::PredictionsReady {
                            spec_bounds: spec_bounds.clone(),
                            times: predictions.times,
                            transmitters: transmitters.clone(),
                            passes,
                        }
                        .into(),
                    )
                });
                Task::batch(vec![
                    predict_task,
                    Task::done(app::Message::WindowMessage(
                        self.window_id,
                        windows::Message::RFPlot(windows::rfplot::Message::Control(
                            windows::rfplot::control::Message::SetControlsVisible(false),
                        )),
                    )),
                ])
            }
            Message::PredictionsReady {
                spec_bounds,
                times,
                transmitters,
                passes,
            } => {
                let to_norm = DataAbsoluteToDataNormalized::from_absolute(&spec_bounds);
                let center_freq = spec_bounds.freq_range.start
                    + (spec_bounds.freq_range.end - spec_bounds.freq_range.start) / 2.0;
                let mut output = self.args.output.clone();
                if output.is_dir() {
                    output = output.join("pass");
                }
                let stem = output
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("pass")
                    .to_owned();
                let parent = output
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_owned();

                let queue = passes
                    .iter()
                    .enumerate()
                    .flat_map(|(pass_idx, pass)| {
                        let t_start = times[pass.time_range.start] as f32;
                        let t_end = times[pass.time_range.end.saturating_sub(1)] as f32;
                        let stem = stem.clone();
                        let parent = parent.clone();
                        pass.frequencies
                            .iter()
                            .enumerate()
                            .map(|(tx_idx, f)| (transmitters[tx_idx], f))
                            .map(move |(tx_freq, f)| {
                                let (f_lo, f_hi) = minmax(f);
                                if f_hi < spec_bounds.freq_range.start.into()
                                    || f_lo > spec_bounds.freq_range.end.into()
                                {
                                    log::info!(
                                        "pass-png: skipping pass {pass_idx} transmitter {tx_freq} \
                                        at [{f_lo}, {f_hi}] Hz (out of spectrogram bounds)"
                                    );
                                    return None;
                                }

                                let freq_margin = (f_hi - f_lo) * 0.25;

                                let f_min = (f_lo as f32 - center_freq) - freq_margin as f32;
                                let f_max = (f_hi as f32 - center_freq) + freq_margin as f32;

                                let rect_da = data_absolute::Rectangle::new(
                                    data_absolute::Point::new(t_start, f_min),
                                    data_absolute::Size::new(
                                        (t_end - t_start).max(1.0),
                                        (f_max - f_min).max(1.0),
                                    ),
                                );
                                Some(PassJob {
                                    view: rect_da * to_norm,
                                    path: parent.join(format!(
                                        "{stem}_{pass_idx:03}_tx_{tx_freq:.0}Hz.png"
                                    )),
                                })
                            })
                            .flatten()
                    })
                    .collect();

                self.process_next_pass(queue)
            }
            Message::FrameReady => {
                let State::WaitingForView(path, queue, delay_frames) = &mut self.state else {
                    return Task::none();
                };
                if *delay_frames > 0 {
                    *delay_frames -= 1;
                    return Task::none();
                }
                let id = self.window_id;
                let path = path.clone();
                self.state = State::WaitingForCapture(path.clone(), queue.clone());
                Task::done(app::Message::WindowMessage(
                    id,
                    windows::Message::RFPlot(windows::rfplot::Message::CaptureScreenshot(Some(
                        path.clone(),
                    ))),
                ))
            }
            Message::ScreenshotSaved(saved_path) => {
                let State::WaitingForCapture(path, queue) = &self.state else {
                    return Task::none();
                };
                if path != &saved_path {
                    log::warn!(
                        "pass-png: received screenshot {saved_path:?} does not match {path:?}"
                    );
                } else {
                    log::info!("pass-png: saved screenshot to {saved_path:?}");
                }
                self.process_next_pass(queue.clone())
            }
        }
    }

    fn process_next_pass(&mut self, mut queue: VecDeque<PassJob>) -> Task<app::Message> {
        let Some(PassJob { view, path }) = queue.pop_front() else {
            log::info!("pass-png: completed all passes");
            return iced::exit();
        };
        log::debug!("pass-png: processing pass with view {view:?} and output path {path:?}");
        let id = self.window_id;
        self.state = State::WaitingForView(path, queue, 1);
        Task::done(app::Message::WindowMessage(
            id,
            windows::Message::RFPlot(windows::rfplot::Message::SetView(view)),
        ))
    }
}
