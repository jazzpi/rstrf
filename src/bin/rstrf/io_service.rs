use std::{path::PathBuf, pin::Pin};

use futures_util::{SinkExt, Stream, StreamExt, stream};
use iced::Subscription;
use rstrf::spectrogram::Spectrogram;

pub enum Event {
    Progress { loaded: usize, total: usize },
    Done(Result<(Vec<PathBuf>, Spectrogram), String>),
}

// run_with(D, fn(&D) -> S) requires fn(&Vec<PathBuf>) since D == Vec<PathBuf>
#[allow(clippy::ptr_arg)]
fn load_worker(paths: &Vec<PathBuf>) -> Pin<Box<dyn Stream<Item = Event> + Send>> {
    let paths = paths.clone();
    Box::pin(iced::stream::channel(16, async move |mut sender| {
        let total = paths.len();
        let mut file_stream = stream::iter(paths.clone())
            .map(rstrf::spectrogram::load_single)
            .buffer_unordered(8);
        let mut spectrograms = Vec::with_capacity(total);

        while let Some(result) = file_stream.next().await {
            match result {
                Ok(spec) => {
                    spectrograms.push(spec);
                    sender
                        .send(Event::Progress {
                            loaded: spectrograms.len(),
                            total,
                        })
                        .await
                        .ok();
                }
                Err(e) => {
                    sender.send(Event::Done(Err(format!("{e:?}")))).await.ok();
                    return;
                }
            }
        }

        spectrograms.sort_by_key(|s| s.start_time());
        let result = Spectrogram::concatenate(spectrograms)
            .map(|s| (paths, s))
            .map_err(|e| format!("{e:?}"));
        sender.send(Event::Done(result)).await.ok();
        std::future::pending::<()>().await;
    }))
}

pub fn load_subscription(paths: Vec<PathBuf>) -> Subscription<Event> {
    Subscription::run_with(paths, load_worker)
}
