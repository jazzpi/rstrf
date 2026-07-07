use itertools::Itertools;
use ndarray::{ArrayView1, s};
use ndarray_stats::QuantileExt;

use crate::{coord::data_absolute, spectrogram::Spectrogram, util::to_index};

/// Maps between `data_absolute` coordinates and the `(time_idx, freq_idx)` index space of a
/// spectrogram's data array.
struct IndexTransform {
    nt: usize,
    nf: usize,
    t_scale: f32,
    f_scale: f32,
    bw: f32,
}

impl IndexTransform {
    fn new(spectrogram: &Spectrogram) -> Self {
        let (nt, nf) = spectrogram.data().dim();
        let t_scale = nt as f32 / spectrogram.length().as_seconds_f32();
        let bw = spectrogram.bw;
        let f_scale = nf as f32 / bw;
        Self {
            nt,
            nf,
            t_scale,
            f_scale,
            bw,
        }
    }

    /// Converts a point in `data_absolute` coordinates to `(time_idx, freq_idx)`, clamping to
    /// bounds.
    fn point_to_index(&self, point: data_absolute::Point) -> (usize, usize) {
        (
            to_index(point.0.x * self.t_scale, self.nt),
            to_index((point.0.y + self.bw / 2.0) * self.f_scale, self.nf),
        )
    }

    /// Converts a (possibly fractional) `(time_idx, freq_idx)` back to `data_absolute`
    /// coordinates.
    fn index_to_point(&self, t_idx: f32, f_idx: f32) -> data_absolute::Point {
        data_absolute::Point::new(t_idx / self.t_scale, f_idx / self.f_scale - self.bw / 2.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SignalDetectionMethod {
    /// Use rfplot's `fit_trace()` algorithm to find signals.
    ///
    /// This finds the frequency with the maximum power at each time slice. If this power deviates
    /// from the mean (over the track window) by more than the threshold, the point is marked as a
    /// signal.
    FitTrace { sigma: f32 },
}

/// Finds signals in a spectrogram.
pub fn find_signals(
    spectrogram: &Spectrogram,
    track_points: &[data_absolute::Point],
    track_bw: f32,
    method: SignalDetectionMethod,
) -> anyhow::Result<Vec<data_absolute::Point>> {
    let idx = IndexTransform::new(spectrogram);
    let data = spectrogram.data();
    let nf = idx.nf;
    let half_bw_idx = (track_bw * 0.5 * idx.f_scale) as usize;
    // TODO: This will clamp x/y to bounds individually -> might change slope for out-of-bounds
    // points
    let track_points = track_points
        .iter()
        .map(|p| idx.point_to_index(*p))
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
                        SignalDetectionMethod::FitTrace { sigma } => find_signals_ft(slice, sigma),
                    }?;

                    let signals_abs = slice_signals
                        .iter()
                        .map(|&f_idx| {
                            idx.index_to_point(
                                (t_idx + t_range.start) as f32,
                                (f_idx + f_range.start) as f32,
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

fn find_signals_ft(data: ArrayView1<f32>, sigma_threshold: f32) -> anyhow::Result<Vec<usize>> {
    // fit_trace works on non-log data, so we need to convert back here
    let data = data.mapv(|v| 10.0_f32.powf(v / 10.0));
    let max_idx = data.argmax()?;
    let max = data[max_idx];
    let sum = data.sum() - max;
    let sq_sum = data.mapv(|v| v * v).sum() - max * max;
    let mean = sum / (data.len() as f32 - 1.0);
    let std_dev = ((sq_sum / (data.len() as f32 - 1.0)) - (mean * mean)).sqrt();
    let sigma = (max - mean) / std_dev;
    if sigma > sigma_threshold {
        Ok(vec![max_idx])
    } else {
        Ok(Vec::new())
    }
}

pub fn centroid(
    spectrogram: &Spectrogram,
    rect: data_absolute::Rectangle,
) -> Option<data_absolute::Point> {
    let idx = IndexTransform::new(spectrogram);
    let data = spectrogram.data();

    let (t_min, f_min) = idx.point_to_index(data_absolute::Point::new(rect.0.x, rect.0.y));
    let (t_max, f_max) = idx.point_to_index(data_absolute::Point::new(
        rect.0.x + rect.0.width,
        rect.0.y + rect.0.height,
    ));
    let t_range = t_min..=t_max;
    let f_range = f_min..=f_max;

    let mut weight_sum = 0.0f32;
    let mut t_weighted_sum = 0.0f32;
    let mut f_weighted_sum = 0.0f32;
    for t_idx in t_range {
        for f_idx in f_range.clone() {
            let weight = 10.0_f32.powf(data[[t_idx, f_idx]] / 10.0);
            weight_sum += weight;
            t_weighted_sum += weight * t_idx as f32;
            f_weighted_sum += weight * f_idx as f32;
        }
    }

    if weight_sum <= 0.0 || !weight_sum.is_finite() {
        return None;
    }

    Some(idx.index_to_point(t_weighted_sum / weight_sum, f_weighted_sum / weight_sum))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, NaiveDate, Utc};
    use ndarray::{Array2, arr1};
    use uuid::Uuid;

    fn test_start() -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
    }

    /// Builds a `Spectrogram` from dB values, one slice per second, so that `t_scale == 1.0`.
    fn spec_from_db(data_db: Array2<f32>, bw: f32) -> Spectrogram {
        let (nslices, nchan) = data_db.dim();
        let power_bounds = data_db
            .iter()
            .cloned()
            .filter(|v| v.is_finite())
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            });
        Spectrogram {
            id: Uuid::new_v4(),
            freq: 437e6,
            bw,
            nchan,
            nslices,
            power_bounds,
            data: data_db.into(),
            timestamps: (0..nslices)
                .map(|i| test_start() + Duration::milliseconds(1000 * i as i64))
                .collect(),
            lengths: vec![1.0; nslices],
        }
    }

    #[test]
    fn flat_data_yields_no_signal() {
        // All bins at 0 dB → std_dev = 0 → sigma = NaN → no signal
        let data = arr1(&[0.0f32; 10]);
        let result = find_signals_ft(data.view(), 5.0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn strong_peak_is_detected_at_correct_index() {
        // 9 bins at 0 dB, 1 bin at 30 dB → sigma = ∞ → signal detected
        let mut data = vec![0.0f32; 10];
        data[5] = 30.0;
        let data = arr1(&data);
        let result = find_signals_ft(data.view(), 5.0).unwrap();
        assert_eq!(result, vec![5]);
    }

    #[test]
    fn threshold_filters_weak_peaks() {
        // Use data with real variance where max is only slightly above mean
        // Values in dB → linear: vary around 1.0 with small spread
        let data = arr1(&[0.0f32, 0.5, -0.3, 0.2, -0.1, 0.4, -0.2, 0.3, 0.1, 0.6]);
        // With high threshold (20 sigma), this moderate peak should not be detected
        let result = find_signals_ft(data.view(), 20.0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn centroid_of_single_peak_is_at_that_bin() {
        // 5 time slices x 5 channels, all bins at -inf dB (zero linear power) except one.
        let mut data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        data[[2, 3]] = 0.0; // linear power 1.0
        let spec = spec_from_db(data, 5.0);

        let rect = spec.bounds();
        let result = centroid(&spec, rect).unwrap();

        // t_scale == 1, f_scale == 1 (nchan / bw = 5 / 5.0), so index 2 -> x = 2.0
        // and index 3 -> y = 3.0 - bw / 2 = 0.5
        assert!((result.0.x - 2.0).abs() < 1e-3, "x = {}", result.0.x);
        assert!((result.0.y - 0.5).abs() < 1e-3, "y = {}", result.0.y);
    }

    #[test]
    fn centroid_weights_by_power_between_two_peaks() {
        // 5x5 grid, all bins at -inf dB except two peaks of different power.
        let mut data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        data[[0, 0]] = 0.0; // linear power 1.0
        data[[4, 4]] = 10.0 * 3.0f32.log10(); // linear power 3.0
        let spec = spec_from_db(data, 5.0);

        let rect = spec.bounds();
        let result = centroid(&spec, rect).unwrap();

        // Weighted index = (1*0 + 3*4) / (1+3) = 3.0 for both t and f.
        // x = 3.0 (t_scale == 1); y = 3.0 - bw/2 = 0.5
        assert!((result.0.x - 3.0).abs() < 1e-3, "x = {}", result.0.x);
        assert!((result.0.y - 0.5).abs() < 1e-3, "y = {}", result.0.y);
    }

    #[test]
    fn centroid_restricts_to_rect_bounds() {
        // Peak outside the queried rect must not influence the result.
        let mut data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        data[[0, 0]] = 0.0; // inside the rect below
        data[[4, 4]] = 40.0; // huge power, but outside the rect
        let spec = spec_from_db(data, 5.0);

        // Rect covering only indices 0..=1 in both dimensions (t in [0, 1], y in [-2.5, -1.5]).
        let rect = data_absolute::Rectangle::new(
            data_absolute::Point::new(0.0, -2.5),
            data_absolute::Size::new(1.0, 1.0),
        );
        let result = centroid(&spec, rect).unwrap();

        assert!((result.0.x - 0.0).abs() < 1e-3, "x = {}", result.0.x);
        assert!((result.0.y - (-2.5)).abs() < 1e-3, "y = {}", result.0.y);
    }

    #[test]
    fn centroid_returns_none_when_rect_has_zero_power() {
        let data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        let spec = spec_from_db(data, 5.0);

        let rect = spec.bounds();
        assert!(centroid(&spec, rect).is_none());
    }
}
