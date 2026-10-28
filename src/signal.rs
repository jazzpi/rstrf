use itertools::Itertools;
use ndarray::{ArrayView1, s};
use ndarray_stats::QuantileExt;

use crate::{
    coord::{
        DataNormalizedToObservedNormalized, DataRelativeToDataNormalized, data_relative,
        observed_normalized,
    },
    spectrogram::Spectrogram,
    util::{norm_to_index, to_index},
};

/// Maps between `observed_normalized` coordinates and the `(time_idx, freq_idx)` index space of a
/// spectrogram's data array.
struct IndexTransform {
    nt: usize,
    nf: usize,
}

impl IndexTransform {
    fn new(spectrogram: &Spectrogram) -> Self {
        let (nt, nf) = spectrogram.data().dim();
        Self { nt, nf }
    }

    /// Converts a point in `observed_normalized` coordinates to `(time_idx, freq_idx)`, clamping to
    /// bounds.
    fn point_to_index(&self, point: observed_normalized::Point) -> (usize, usize) {
        (
            norm_to_index(point.0.x, self.nt),
            norm_to_index(point.0.y, self.nf),
        )
    }

    /// Converts a (possibly fractional) `(time_idx, freq_idx)` back to `observed_normalized`
    /// coordinates.
    fn index_to_point(&self, t_idx: f32, f_idx: f32) -> observed_normalized::Point {
        let t_idx = t_idx / (self.nt - 1) as f32;
        let f_idx = f_idx / (self.nf - 1) as f32;
        observed_normalized::Point::new(t_idx, f_idx)
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
    track_points: &[observed_normalized::Point],
    track_bw: f32,
    method: SignalDetectionMethod,
) -> anyhow::Result<Vec<observed_normalized::Point>> {
    let tf = IndexTransform::new(spectrogram);
    let data = spectrogram.data();
    let nf = tf.nf;

    // TODO: This will clamp x/y to bounds individually -> might change slope for out-of-bounds
    // points
    // let track_points = track_points.iter().map(|p| tf.point_to_index(*p));

    let half_bw = track_bw * 0.5 * nf as f32;
    let mut signals = Vec::new();
    // We iterate over each pair of track points...
    for (a, b) in track_points.iter().tuple_windows() {
        let dt = b.0.x - a.0.x;
        let df = b.0.y - a.0.y;
        anyhow::ensure!(dt > 0.0, "Track points must be in increasing time order");
        let slope = df as f32 / dt as f32;

        let a_idx = tf.point_to_index(*a).0;
        let b_idx = tf.point_to_index(*b).0;
        let at = a.0.x * tf.nt as f32;
        let af = a.0.y * tf.nf as f32;
        for t_idx in a_idx..=b_idx {
            // ... and slice on the frequency axis to the window around the track line...
            let center_f = af + slope * (t_idx as f32 - at);
            let fmin = to_index(center_f - half_bw, nf);
            let fmax = to_index(center_f + half_bw, nf);
            let f_range = fmin..=fmax;
            let slice = data.slice(s![t_idx, f_range.clone()]);

            // ... We find signals in each slice independently...
            let slice_signals = match method {
                SignalDetectionMethod::FitTrace { sigma } => find_signals_ft(slice, sigma),
            }?;

            // ... before converting the indices back to `observed_normalized` coordinates.
            signals.extend(
                slice_signals
                    .iter()
                    .map(|&f_idx| tf.index_to_point(t_idx as f32, (f_idx + fmin) as f32)),
            );
        }
    }
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
    rect: observed_normalized::Rectangle,
) -> Option<observed_normalized::Point> {
    let idx = IndexTransform::new(spectrogram);
    let data = spectrogram.data();

    let min_corner = observed_normalized::Point::new(rect.0.x, rect.0.y);
    let max_corner =
        observed_normalized::Point::new(rect.0.x + rect.0.width, rect.0.y + rect.0.height);
    let (t_min, f_min) = idx.point_to_index(min_corner);
    let (t_max, f_max) = idx.point_to_index(max_corner);
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
    use crate::{
        coord::{DataNormalizedToDataRelative, ObservedNormalizedToDataNormalized},
        util::sec_to_duration,
    };

    use super::*;
    use chrono::{DateTime, NaiveDate, Utc};
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
                .map(|i| test_start() + sec_to_duration(i as f64))
                .collect(),
            lengths: vec![1.0; nslices],
        }
    }

    fn on_to_dr(p: observed_normalized::Point, spec: &Spectrogram) -> data_relative::Point {
        let t1 = ObservedNormalizedToDataNormalized::new(None);
        let t2 = DataNormalizedToDataRelative::new(&spec.bounds());
        p * t1 * t2
    }

    #[test]
    fn index_transform_roundtrips_from_index() {
        let tf = IndexTransform { nt: 10, nf: 20 };
        let points = vec![(0, 0), (9, 19), (5, 10), (1, 18)];
        for (t_idx, f_idx) in points {
            let p = tf.index_to_point(t_idx as f32, f_idx as f32);
            let (t_idx2, f_idx2) = tf.point_to_index(p);
            assert_eq!(t_idx, t_idx2);
            assert_eq!(f_idx, f_idx2);
        }
    }

    #[test]
    fn index_transform_roundtrips_from_point_and_nice_size() {
        // n=11/21 means each index corresponds to 0.1/0.05 in normalized coordinates, so the points
        // below should roundtrip fine.
        let tf = IndexTransform { nt: 11, nf: 21 };
        let points = vec![
            observed_normalized::Point::new(0.0, 0.0),
            observed_normalized::Point::new(1.0, 1.0),
            observed_normalized::Point::new(0.5, 0.5),
            observed_normalized::Point::new(0.1, 0.9),
        ];
        for p in points {
            let (t_idx, f_idx) = tf.point_to_index(p);
            let p2 = tf.index_to_point(t_idx as f32, f_idx as f32);
            assert!((p.0.x - p2.0.x).abs() < 1e-6);
            assert!((p.0.y - p2.0.y).abs() < 1e-6);
        }
    }

    #[test]
    fn index_transform_does_not_roundtrip_from_point_and_ugly_size() {
        let tf = IndexTransform { nt: 10, nf: 20 };
        let points = vec![
            observed_normalized::Point::new(0.5, 0.5),
            observed_normalized::Point::new(0.1, 0.9),
        ];
        for p in points {
            let (t_idx, f_idx) = tf.point_to_index(p);
            let p2 = tf.index_to_point(t_idx as f32, f_idx as f32);
            assert!((p.0.x - p2.0.x).abs() > 1e-3);
            assert!((p.0.y - p2.0.y).abs() > 1e-3);
        }
    }

    #[test]
    fn point_to_index_clamps_to_bounds() {
        let tf = IndexTransform { nt: 10, nf: 20 };
        let p = observed_normalized::Point::new(-1.0, -1.0);
        let (t_idx, f_idx) = tf.point_to_index(p);
        assert_eq!(t_idx, 0);
        assert_eq!(f_idx, 0);
        let p = observed_normalized::Point::new(2.0, 2.0);
        let (t_idx, f_idx) = tf.point_to_index(p);
        assert_eq!(t_idx, 9);
        assert_eq!(f_idx, 19);
    }

    #[test]
    fn point_to_index_uses_middle_idx_for_half() {
        let tf = IndexTransform { nt: 5, nf: 7 };
        let p = observed_normalized::Point::new(0.5, 0.5);
        let (t_idx, f_idx) = tf.point_to_index(p);
        assert_eq!(t_idx, 2);
        assert_eq!(f_idx, 3);

        let tf = IndexTransform { nt: 6, nf: 8 };
        let (t_idx, f_idx) = tf.point_to_index(p);
        assert_eq!(t_idx, 3);
        assert_eq!(f_idx, 4);
    }

    #[test]
    fn index_to_point_uses_half_for_middle_idx_and_uneven_n() {
        let tf = IndexTransform { nt: 5, nf: 7 };
        let p = tf.index_to_point(2.0, 3.0);
        assert!((p.0.x - 0.5).abs() < 1e-6);
        assert!((p.0.y - 0.5).abs() < 1e-6);
    }

    #[test]
    fn index_to_point_does_not_use_half_for_middle_idx_and_even_n() {
        let tf = IndexTransform { nt: 6, nf: 8 };
        let p = tf.index_to_point(3.0, 4.0);
        assert!((p.0.x - (3.0 * 1.0 / (6.0 - 1.0))).abs() < 1e-6);
        assert!((p.0.y - (4.0 * 1.0 / (8.0 - 1.0))).abs() < 1e-6);
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
    fn find_signals_finds_peaks_along_track() {
        // 5 time slices x 6 channels, all bins at -inf dB (zero linear power) except one per slice.
        let mut data = Array2::from_elem((5, 6), f32::NEG_INFINITY);
        data[[0, 0]] = 0.0; // linear power 1.0
        data[[1, 1]] = 0.0;
        data[[2, 2]] = 0.0;
        data[[3, 3]] = 0.0;
        data[[4, 4]] = 0.0;
        let spec = spec_from_db(data, 6.0);
        let tf = IndexTransform::new(&spec);

        // t in [0.9, 3.1] -> t_idx in [1, 3] due to rounding. Slope at those indices:
        // 1 -> 0.32
        // 2 -> 2.55
        // 3 -> 4.78
        // -> with half bw of 1.0, only the peaks at (1,1) and (2,2) should be detected.
        let track_points = vec![tf.index_to_point(0.9, 0.1), tf.index_to_point(3.1, 5.0)];
        let track_bw = 1.0 * 2.0 / tf.nf as f32; // 2 bins wide in normalized coordinates
        let result = find_signals(
            &spec,
            &track_points,
            track_bw,
            SignalDetectionMethod::FitTrace { sigma: 5.0 },
        )
        .unwrap();

        assert_eq!(result.len(), 2);
        let (t0, f0) = tf.point_to_index(result[0]);
        let (t1, f1) = tf.point_to_index(result[1]);
        assert_eq!(t0, 1);
        assert_eq!(f0, 1);
        assert_eq!(t1, 2);
        assert_eq!(f1, 2);
    }

    #[test]
    fn centroid_of_single_peak_is_at_that_bin() {
        // 5 time slices x 5 channels, all bins at -inf dB (zero linear power) except one.
        let mut data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        data[[2, 3]] = 0.0; // linear power 1.0
        let spec = spec_from_db(data, 5.0);

        let rect = observed_normalized::Rectangle::new(
            observed_normalized::Point::new(0.0, 0.0),
            observed_normalized::Size::new(1.0, 1.0),
        );
        let result = centroid(&spec, rect).unwrap();

        // nt = nf = 5 -> each index corresponds to 1/(5-1) = 1/4 = 0.25 in normalized coordinates
        // index 2 -> x = 2 * 1/4 = 0.5
        // index 3 -> y = 3 * 1/4 = 0.75
        assert!((result.0.x - 0.5).abs() < 1e-3, "x = {}", result.0.x);
        assert!((result.0.y - 0.75).abs() < 1e-3, "y = {}", result.0.y);
    }

    #[test]
    fn centroid_weights_by_power_between_two_peaks() {
        // 5x5 grid, all bins at -inf dB except two peaks of different power.
        let mut data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        data[[0, 0]] = 0.0; // linear power 1.0
        data[[4, 4]] = 10.0 * 3.0f32.log10(); // linear power 3.0
        let spec = spec_from_db(data, 5.0);

        let rect = observed_normalized::Rectangle::new(
            observed_normalized::Point::new(0.0, 0.0),
            observed_normalized::Size::new(1.0, 1.0),
        );
        let result = centroid(&spec, rect).unwrap();

        // Weighted index = (1*0 + 3*4) / (1+3) = 3.0 for both t and f.
        // index 3 -> 3 * 1/4 = 0.75 in normalized coordinates
        assert!((result.0.x - 0.75).abs() < 1e-6, "x = {}", result.0.x);
        assert!((result.0.y - 0.75).abs() < 1e-6, "y = {}", result.0.y);
    }

    #[test]
    fn centroid_restricts_to_rect_bounds() {
        // Peak outside the queried rect must not influence the result.
        let mut data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        data[[0, 0]] = 0.0; // inside the rect below
        data[[4, 4]] = 40.0; // huge power, but outside the rect
        let spec = spec_from_db(data, 5.0);

        // Rect covering only indices 0..=1 in both dimensions
        let rect = observed_normalized::Rectangle::new(
            observed_normalized::Point::new(0.0, 0.0),
            observed_normalized::Size::new(0.2, 0.2),
        );
        let result = centroid(&spec, rect).unwrap();

        assert!((result.0.x - 0.0).abs() < 1e-6, "x = {}", result.0.x);
        assert!((result.0.y - 0.0).abs() < 1e-6, "y = {}", result.0.y);
    }

    #[test]
    fn centroid_returns_none_when_rect_has_zero_power() {
        let data = Array2::from_elem((5, 5), f32::NEG_INFINITY);
        let spec = spec_from_db(data, 5.0);

        let rect = observed_normalized::Rectangle::new(
            observed_normalized::Point::new(0.0, 0.0),
            observed_normalized::Size::new(1.0, 1.0),
        );
        assert!(centroid(&spec, rect).is_none());
    }
}
