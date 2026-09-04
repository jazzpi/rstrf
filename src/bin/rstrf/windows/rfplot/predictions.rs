//! Satellite pass predictions and the cache that keeps them fresh.

use chrono::{DateTime, Utc};
use iced::Task;
use rstrf::orbit::{self, Site};

use crate::app::AppShared;

use super::{PredictionsMsg, State};

/// All inputs that determine the satellite pass predictions.
///
/// To avoid having to explicitly keep track of when the predictions are stale, we use this as the
/// key for an `AsyncCache` and compare the stored predictions against a freshly built key whenever
/// one of the inputs may have changed.
///
/// This involves creating a copy of the key & comparing it, so we don't want the key to be too big.
/// Thus, we don't include the full `Satellite` structs and instead just include the satellite IDs.
/// That breaks the automatic staleness detection if the satellites are changed (e.g. new TLEs
/// loaded or transmitters modified), and these cases need to be handled manually (via
/// `PredictionsMsg::RefreshCache`). This is a bit annoying, but keeping the full satellite data in
/// the key comes with a severe performance penalty for large catalogs.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PredictionKey {
    satellites: Vec<u64>,
    time_range: std::ops::Range<DateTime<Utc>>,
    freq_range: std::ops::Range<f32>,
    site: Site,
}

fn prediction_key(state: &State, app: &AppShared) -> Option<PredictionKey> {
    let spectrogram = state.spectrogram()?;
    let site = app.site()?;
    let satellites = app.active_satellite_ids();
    if satellites.is_empty() {
        return None;
    }
    let bounds = spectrogram.absolute_bounds();
    Some(PredictionKey {
        satellites,
        time_range: bounds.time_range,
        freq_range: bounds.freq_range,
        site,
    })
}

impl State {
    pub(super) fn status(&self, app: &AppShared) -> Option<&str> {
        if !self.display.show_predictions {
            return None;
        }
        if app.satellites.is_empty() {
            Some("No satellites")
        } else if self.prediction_cache.busy() {
            Some("Predicting satellite passes...")
        } else if app.site().is_none() {
            Some("No site configured")
        } else if self.prediction_cache.get_stored().is_none() {
            Some("No passes predicted")
        } else {
            None
        }
    }

    /// Checks whether the prediction cache is stale for the current inputs. If so, starts an async
    /// recomputation.
    pub(super) fn check_cache(&mut self, app: &AppShared) -> Task<super::Message> {
        let Some(key) = prediction_key(self, app) else {
            self.prediction_cache.reset();
            return Task::none();
        };
        self.prediction_cache.request(key, |key| {
            let satellites = app.active_satellites();
            Task::future(async move {
                let key_for_msg = key.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let freq_range = (key.freq_range.start as f64)..(key.freq_range.end as f64);
                    orbit::predict_satellites(&satellites, key.time_range, freq_range, &key.site)
                })
                .await;
                match result {
                    Ok(predictions) => {
                        PredictionsMsg::PredictionsReady(key_for_msg, predictions).into()
                    }
                    Err(e) => {
                        log::error!("Failed to predict satellite passes: {}", e);
                        PredictionsMsg::PredictionFailed.into()
                    }
                }
            })
        })
    }
}
