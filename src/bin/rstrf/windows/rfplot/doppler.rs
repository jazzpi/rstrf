//! Doppler correction: shifting every slice so a chosen transmitter appears at a fixed frequency.

use std::{ops::Range, sync::Arc};

use chrono::{DateTime, Utc};
use glam::{Vec2, vec2};
use iced::Task;
use ndarray::Array1;
use rstrf::{
    orbit::{self, Satellite, Site},
    spectrogram::Spectrogram,
};
use uuid::Uuid;

use crate::app::AppShared;

use super::{PredictionsMsg, State};

/// The satellite (and optionally the transmitter) to correct for, as requested by the user.
#[derive(Debug, Clone, PartialEq)]
pub struct DopplerTarget {
    pub norad_id: u64,
    /// Transmitter frequency (Hz). If `None`, the satellite's first transmitter is used.
    pub freq: Option<f64>,
}

/// An entry in the Doppler target pick list; `None` turns correction off.
#[derive(Debug, Clone, PartialEq)]
pub struct DopplerOption {
    pub target: Option<DopplerTarget>,
    label: String,
}

impl DopplerOption {
    const OFF: &str = "No Doppler correction";

    fn new(target: DopplerTarget, name: Option<&str>) -> Self {
        let name = name.map(|n| format!(" {n}")).unwrap_or_default();
        let freq = match target.freq {
            Some(freq) => format!("{:.3} MHz", freq / 1e6),
            None => "first transmitter".to_owned(),
        };
        let label = format!("{}{} @ {}", target.norad_id, name, freq);
        Self {
            target: Some(target),
            label,
        }
    }
}

impl std::fmt::Display for DopplerOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DopplerKey {
    norad_id: u64,
    /// The resolved transmitter frequency (Hz)
    freq: f64,
    spectrogram_id: Uuid,
    time_range: Range<DateTime<Utc>>,
    site: Site,
}

#[derive(Debug, Clone)]
pub struct DopplerCorrection {
    /// Data-normalized bottom/top edge of each slice after correction
    pub y_ranges: Arc<[Vec2]>,
    /// Shift (Hz) to subtract from observed frequencies, sampled at the prediction times
    pub pred_shift_hz: Array1<f64>,
}

impl DopplerCorrection {
    fn compute(sat: &Satellite, spectrogram: &Spectrogram, key: &DopplerKey) -> Self {
        let start = key.time_range.start;
        let slice_times = spectrogram
            .timestamps
            .iter()
            .zip(&spectrogram.lengths)
            .map(|(&t, &len)| (t - start).as_seconds_f64() + len as f64 / 2.0)
            .collect::<Array1<_>>();
        let slice_shift_hz = sat
            .range_rates(start, slice_times.view(), &key.site)
            .mapv(|rr| orbit::doppler_shift(rr, key.freq));
        let bw = spectrogram.bw as f64;
        let y_ranges = slice_shift_hz
            .iter()
            .map(|&shift| {
                // SGP4 failures are NaN; leave those slices where they were recorded
                let offset = if shift.is_finite() {
                    (-shift / bw) as f32
                } else {
                    0.0
                };
                vec2(offset, 1.0 + offset)
            })
            .collect();

        let pred_times = orbit::prediction_times(&key.time_range);
        let pred_shift_hz = sat
            .range_rates(start, pred_times.view(), &key.site)
            .mapv(|rr| orbit::doppler_shift(rr, key.freq));

        Self {
            y_ranges,
            pred_shift_hz,
        }
    }

    /// Lowest and highest data-normalized y covered by any slice
    pub fn extent_y(&self) -> (f32, f32) {
        self.y_ranges
            .iter()
            .fold((0.0_f32, 1.0_f32), |(lo, hi), r| (lo.min(r.x), hi.max(r.y)))
    }
}

/// Picks the transmitter frequency to correct for, or `None` (after logging why) if there is none.
fn resolve_freq(target: &DopplerTarget, sat: &Satellite) -> Option<f64> {
    if let Some(freq) = target.freq {
        return Some(freq);
    }
    match sat.transmitters.as_slice() {
        [] => {
            log::error!(
                "Doppler target {} has no transmitters and no frequency was given",
                target.norad_id
            );
            None
        }
        [freq] => Some(*freq),
        [freq, ..] => {
            log::warn!(
                "Doppler target {} has {} transmitters, using the first ({} Hz)",
                target.norad_id,
                sat.transmitters.len(),
                freq
            );
            Some(*freq)
        }
    }
}

/// Pick list entries: "off", one per transmitter with a predicted pass inside the band, and the
/// current target if it isn't among those.
fn doppler_options(
    current: Option<&DopplerTarget>,
    predictions_in_band: Option<(&orbit::Predictions, Range<f64>)>,
    satellites: &[(Satellite, bool)],
) -> Vec<DopplerOption> {
    let find_sat = |norad_id| satellites.iter().find(|(s, _)| s.norad_id() == norad_id);
    let mut targets = Vec::new();
    if let Some((predictions, band)) = predictions_in_band {
        for (norad_id, passes) in predictions.iter_satellites() {
            let Some((sat, _)) = find_sat(norad_id) else {
                continue;
            };
            for (i, &freq) in sat.transmitters.iter().enumerate() {
                let in_band = passes.iter().any(|pass| {
                    pass.frequencies
                        .get(i)
                        .is_some_and(|f| f.iter().any(|f| band.contains(f)))
                });
                if in_band {
                    let target = DopplerTarget {
                        norad_id,
                        freq: Some(freq),
                    };
                    targets.push((target, sat.elements.object_name.as_deref()));
                }
            }
        }
    }
    targets.sort_by(|(a, _), (b, _)| {
        (a.norad_id, a.freq)
            .partial_cmp(&(b.norad_id, b.freq))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut options = vec![DopplerOption {
        target: None,
        label: DopplerOption::OFF.to_owned(),
    }];
    if let Some(current) = current
        && !targets.iter().any(|(t, _)| t == current)
    {
        let name = find_sat(current.norad_id).and_then(|(s, _)| s.elements.object_name.as_deref());
        options.push(DopplerOption::new(current.clone(), name));
    }
    options.extend(
        targets
            .into_iter()
            .map(|(t, name)| DopplerOption::new(t, name)),
    );
    options
}

impl State {
    /// The Doppler correction to apply, if one is requested and has been computed for the current
    /// spectrogram.
    pub(super) fn doppler_correction(&self) -> Option<&DopplerCorrection> {
        let spectrogram = self.spectrogram()?;
        self.doppler_target.as_ref()?;
        let (key, correction) = self.doppler_cache.get_stored()?;
        (key.spectrogram_id == spectrogram.id).then_some(correction)
    }

    /// Checks whether the Doppler correction is stale for the current inputs. If so, starts an
    /// async recomputation.
    pub(super) fn check_doppler_cache(&mut self, app: &AppShared) -> Task<super::Message> {
        let Some((key, sat)) = self.doppler_key(app) else {
            self.doppler_cache.reset();
            self.viewport.set_extent_y(0.0, 1.0);
            return Task::none();
        };
        let spectrogram = self.spectrogram().cloned();
        self.doppler_cache.request(key, |key| {
            Task::future(async move {
                let key_for_msg = key.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let spectrogram = spectrogram.expect("key requires a spectrogram");
                    DopplerCorrection::compute(&sat, &spectrogram, &key)
                })
                .await;
                match result {
                    Ok(correction) => PredictionsMsg::DopplerReady(key_for_msg, correction).into(),
                    Err(e) => {
                        log::error!("Failed to compute Doppler correction: {}", e);
                        PredictionsMsg::PredictionFailed.into()
                    }
                }
            })
        })
    }

    pub(super) fn doppler_options(&self, app: &AppShared) -> Vec<DopplerOption> {
        let predictions = self.prediction_cache.get_stored().map(|(_, p)| p);
        let band = self.spectrogram().map(|s| {
            let band = s.absolute_bounds().freq_range;
            band.start as f64..band.end as f64
        });
        doppler_options(
            self.doppler_target.as_ref(),
            predictions.zip(band),
            &app.satellites,
        )
    }

    pub(super) fn selected_doppler_option(
        &self,
        options: &[DopplerOption],
    ) -> Option<DopplerOption> {
        options
            .iter()
            .find(|o| o.target == self.doppler_target)
            .cloned()
    }

    fn doppler_key(&self, app: &AppShared) -> Option<(DopplerKey, Satellite)> {
        let target = self.doppler_target.as_ref()?;
        let spectrogram = self.spectrogram()?;
        let site = self.site(app)?.clone();
        let Some((sat, _)) = app
            .satellites
            .iter()
            .find(|(sat, _)| sat.norad_id() == target.norad_id)
        else {
            if !app.satellites.is_empty() {
                log::error!("Doppler target {} not found in catalog", target.norad_id);
            }
            return None;
        };
        let freq = resolve_freq(target, sat)?;
        let key = DopplerKey {
            norad_id: target.norad_id,
            freq,
            spectrogram_id: spectrogram.id,
            time_range: spectrogram.absolute_bounds().time_range,
            site,
        };
        Some((key, sat.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::TimeZone;
    use ndarray::{ArcArray2, s};

    use super::*;

    const TX_FREQ: f64 = 437.525e6;

    fn iss() -> Satellite {
        iss_with_transmitters(vec![TX_FREQ])
    }

    fn iss_with_transmitters(transmitters: Vec<f64>) -> Satellite {
        // ISS TLE from Wikipedia (epoch 2008-09-20)
        let line1 = "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927";
        let line2 = "2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537";
        let freqs = HashMap::from([(25544u64, transmitters)]);
        Satellite::from_tle(Some("ISS (ZARYA)".to_string()), line1, line2, &freqs).unwrap()
    }

    /// A day of 10 s slices, starting at the TLE epoch
    fn spectrogram() -> Spectrogram {
        let start = Utc.with_ymd_and_hms(2008, 9, 20, 12, 25, 40).unwrap();
        let nslices = 8640;
        let nchan = 4;
        Spectrogram {
            id: Uuid::new_v4(),
            nchan,
            nslices,
            freq: TX_FREQ as f32,
            bw: 100e3,
            power_bounds: (0.0, 1.0),
            data: ArcArray2::zeros((nslices, nchan)),
            timestamps: (0..nslices)
                .map(|i| start + chrono::Duration::seconds(10 * i as i64))
                .collect(),
            lengths: vec![10.0; nslices],
        }
    }

    fn key(spectrogram: &Spectrogram) -> DopplerKey {
        DopplerKey {
            norad_id: 25544,
            freq: TX_FREQ,
            spectrogram_id: spectrogram.id,
            time_range: spectrogram.absolute_bounds().time_range,
            site: Site::default(),
        }
    }

    #[test]
    fn target_prediction_is_flat_after_correction() {
        let sat = iss();
        let spectrogram = spectrogram();
        let key = key(&spectrogram);
        let correction = DopplerCorrection::compute(&sat, &spectrogram, &key);

        let predictions = orbit::predict_satellites(
            std::slice::from_ref(&sat),
            key.time_range.clone(),
            f64::NEG_INFINITY..f64::INFINITY,
            &key.site,
        );
        assert_eq!(correction.pred_shift_hz.len(), predictions.times.len());
        let passes = predictions.for_id(25544);
        assert!(!passes.is_empty());
        for pass in passes {
            let shift = correction.pred_shift_hz.slice(s![pass.time_range.clone()]);
            let corrected = &pass.frequencies[0] - &shift;
            for f in corrected {
                assert!((f - TX_FREQ).abs() < 1e-3, "{} != {}", f, TX_FREQ);
            }
        }
    }

    #[test]
    fn slices_shift_against_doppler() {
        let sat = iss();
        let spectrogram = spectrogram();
        let key = key(&spectrogram);
        let correction = DopplerCorrection::compute(&sat, &spectrogram, &key);
        assert_eq!(correction.y_ranges.len(), spectrogram.nslices);

        let start = key.time_range.start;
        let mid_times = spectrogram
            .timestamps
            .iter()
            .map(|&t| (t - start).as_seconds_f64() + 5.0)
            .collect::<Array1<_>>();
        let range_rates = sat.range_rates(start, mid_times.view(), &key.site);
        for (r, rr) in correction.y_ranges.iter().zip(range_rates) {
            assert!((r.y - r.x - 1.0).abs() < 1e-6);
            // Approaching (rr < 0) means observed above f_tx, so the slice moves down
            let observed_offset = orbit::doppler_shift(rr, TX_FREQ) / spectrogram.bw as f64;
            assert!(
                (r.x as f64 + observed_offset).abs() < 1e-6,
                "{:?} vs {}",
                r,
                rr
            );
        }
        let (lo, hi) = correction.extent_y();
        assert!(lo < 0.0 && hi > 1.0, "{lo}..{hi}");
    }

    #[test]
    fn options_list_in_band_transmitters_and_current_target() {
        let sat = iss_with_transmitters(vec![145.8e6, TX_FREQ]);
        let spectrogram = spectrogram();
        let bounds = spectrogram.absolute_bounds();
        let band = bounds.freq_range.start as f64..bounds.freq_range.end as f64;
        let predictions = orbit::predict_satellites(
            std::slice::from_ref(&sat),
            bounds.time_range,
            band.clone(),
            &Site::default(),
        );
        let satellites = [(sat, true)];
        let targets = |current: Option<&DopplerTarget>| {
            doppler_options(current, Some((&predictions, band.clone())), &satellites)
                .into_iter()
                .map(|o| o.target)
                .collect::<Vec<_>>()
        };
        let in_band = DopplerTarget {
            norad_id: 25544,
            freq: Some(TX_FREQ),
        };

        assert_eq!(targets(None), [None, Some(in_band.clone())]);
        assert_eq!(targets(Some(&in_band)), [None, Some(in_band.clone())]);

        let from_cli = DopplerTarget {
            norad_id: 25544,
            freq: None,
        };
        assert_eq!(
            targets(Some(&from_cli)),
            [None, Some(from_cli), Some(in_band)]
        );
    }
}
