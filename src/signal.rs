use itertools::Itertools;
use ndarray::{ArrayView1, s};
use ndarray_stats::QuantileExt;

use crate::{coord::data_absolute, spectrogram::Spectrogram, util::to_index};

#[derive(Debug, Clone, Copy)]
pub enum SignalDetectionMethod {
    /// Use rfplot's `fit_trace()` algorithm to find signals.
    ///
    /// This finds the frequency with the maximum power at each time slice. If this power deviates
    /// from the mean (over the track window) by more than the threshold, the point is marked as a
    /// signal.
    FitTrace,
}

/// Finds signals in a spectrogram.
pub fn find_signals(
    spectrogram: &Spectrogram,
    track_points: &[data_absolute::Point],
    track_bw: f32,
    method: SignalDetectionMethod,
) -> anyhow::Result<Vec<data_absolute::Point>> {
    let data = spectrogram.data();
    let (nt, nf) = data.dim();
    let t_scale = nt as f32 / spectrogram.length().as_seconds_f32();
    let bw = spectrogram.bw;
    let f_scale = nf as f32 / bw;
    let half_bw_idx = (track_bw * 0.5 * f_scale) as usize;
    let track_points = track_points
        .iter()
        .map(|p| {
            (
                // TODO: This will clamp x/y to bounds individually -> might change slope
                // for out-of-bounds points
                to_index(p.0.x * t_scale, nt),
                to_index((p.0.y + bw / 2.0) * f_scale, nf),
            )
        })
        .collect_vec();
    let t_range = track_points.first().unwrap().0..(track_points.last().unwrap().0 + 1);
    let data = data.slice(s![t_range.clone(), ..]).to_owned();

    let signals = track_points
        .into_iter()
        .map(|(t_idx, f_idx)| (t_idx - t_range.start, f_idx))
        .tuple_windows()
        .flat_map(|(a, b)| -> anyhow::Result<Vec<data_absolute::Point>> {
            let slope = (b.1 as f32 - a.1 as f32) / (b.0 as f32 - a.0 as f32);
            let signals_nested: anyhow::Result<Vec<Vec<data_absolute::Point>>> = (a.0..=b.0)
                .map(|t_idx| {
                    let center_f = (a.1 as f32 + slope * (t_idx - a.0) as f32).round() as usize;
                    let f_range =
                        center_f.saturating_sub(half_bw_idx)..(center_f + half_bw_idx).min(nf - 1);
                    let slice = data.slice(s![t_idx, f_range.clone()]);

                    let slice_signals = match method {
                        SignalDetectionMethod::FitTrace => find_signals_ft(slice),
                    }?;

                    let signals_abs = slice_signals
                        .iter()
                        .map(|&f_idx| {
                            data_absolute::Point::new(
                                (t_idx + t_range.start) as f32 / t_scale,
                                (f_idx + f_range.start) as f32 / f_scale - bw / 2.0,
                            )
                        })
                        .collect();
                    Ok(signals_abs)
                })
                .collect();
            let signals = signals_nested?.into_iter().flatten().collect_vec();
            Ok(signals)
        })
        .flatten()
        .collect_vec();
    Ok(signals)
}

fn find_signals_ft(data: ArrayView1<f32>) -> anyhow::Result<Vec<usize>> {
    // fit_trace works on non-log data, so we need to convert back here
    let data = data.mapv(|v| 10.0_f32.powf(v / 10.0));
    let max_idx = data.argmax()?;
    let max = data[max_idx];
    let sum = data.sum() - max;
    let sq_sum = data.mapv(|v| v * v).sum() - max * max;
    let mean = sum / (data.len() as f32 - 1.0);
    let std_dev = ((sq_sum / (data.len() as f32 - 1.0)) - (mean * mean)).sqrt();
    let sigma = (max - mean) / std_dev;
    // TODO: make this configurable
    if sigma > 5.0 {
        Ok(vec![max_idx])
    } else {
        Ok(Vec::new())
    }
}
