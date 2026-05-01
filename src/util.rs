use std::path::PathBuf;

use iced::{Point, Rectangle};
use itertools::izip;
use ndarray::Array1;
use rfd::AsyncFileDialog;
use sgp4::Elements;
use space_track::GeneralPerturbation;

// TODO: How can we implement this for f32 as well?
pub fn minmax(arr: &Array1<f64>) -> (f64, f64) {
    if arr.is_empty() {
        (f64::NAN, f64::NAN)
    } else {
        arr.iter()
            .cloned()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), val| {
                (min.min(val), max.max(val))
            })
    }
}

pub fn to_index(value: f32, max: usize) -> usize {
    value.round().clamp(0.0, (max - 1) as f32) as usize
}

pub fn clip_line(bounds: &Rectangle, a: Point, b: Point) -> Option<(Point, Point)> {
    // https://en.wikipedia.org/wiki/Liang%E2%80%93Barsky_algorithm
    let delta = b - a;

    let pv = [-delta.x, delta.x, -delta.y, delta.y];
    let qv = [
        a.x - bounds.x,
        bounds.x + bounds.width - a.x,
        a.y - bounds.y,
        bounds.y + bounds.height - a.y,
    ];

    let mut u1 = 0f32;
    let mut u2 = 1f32;
    for (&p, &q) in izip!(pv.iter(), qv.iter()) {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            } else {
                continue;
            }
        }
        let u = q / p;
        if p < 0.0 {
            u1 = u1.max(u);
        } else {
            u2 = u2.min(u);
        }
    }

    if u1 > u2 {
        None
    } else {
        Some((a + delta * u1, a + delta * u2))
    }
}

pub async fn pick_file(filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    let mut dialog = AsyncFileDialog::new();
    for &(name, extensions) in filters {
        dialog = dialog.add_filter(name, extensions);
    }
    dialog
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

pub fn spacetrack_to_sgp4(sat: &GeneralPerturbation) -> Option<Elements> {
    serde_json::from_str(&serde_json::to_string(sat).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{Point as IcedPoint, Rectangle, Size};
    use ndarray::arr1;

    #[test]
    fn minmax_empty_returns_nan() {
        let (lo, hi) = minmax(&arr1(&[]));
        assert!(lo.is_nan());
        assert!(hi.is_nan());
    }

    #[test]
    fn minmax_single_element() {
        let (lo, hi) = minmax(&arr1(&[3.0f64]));
        assert_eq!(lo, 3.0);
        assert_eq!(hi, 3.0);
    }

    #[test]
    fn minmax_returns_correct_bounds() {
        let (lo, hi) = minmax(&arr1(&[3.0f64, 1.0, 5.0, -2.0, 0.0]));
        assert_eq!(lo, -2.0);
        assert_eq!(hi, 5.0);
    }

    #[test]
    fn to_index_clamps_below_zero() {
        assert_eq!(to_index(-1.0, 256), 0);
        assert_eq!(to_index(-100.0, 256), 0);
    }

    #[test]
    fn to_index_clamps_above_max() {
        assert_eq!(to_index(300.0, 256), 255);
        assert_eq!(to_index(255.5, 256), 255);
    }

    #[test]
    fn to_index_rounds_in_bounds() {
        assert_eq!(to_index(2.4, 10), 2);
        assert_eq!(to_index(2.5, 10), 3);
        assert_eq!(to_index(0.0, 10), 0);
    }

    fn unit_bounds() -> Rectangle {
        Rectangle::new(IcedPoint::new(0.0, 0.0), Size::new(1.0, 1.0))
    }

    #[test]
    fn clip_line_fully_inside_unchanged() {
        let bounds = unit_bounds();
        let a = IcedPoint::new(0.1, 0.1);
        let b = IcedPoint::new(0.9, 0.9);
        let result = clip_line(&bounds, a, b).unwrap();
        assert!((result.0.x - a.x).abs() < 1e-6);
        assert!((result.1.x - b.x).abs() < 1e-6);
    }

    #[test]
    fn clip_line_fully_outside_returns_none() {
        let bounds = unit_bounds();
        // Horizontal line to the right of bounds
        let a = IcedPoint::new(1.5, 0.5);
        let b = IcedPoint::new(2.0, 0.5);
        assert!(clip_line(&bounds, a, b).is_none());
    }

    #[test]
    fn clip_line_parallel_outside_returns_none() {
        let bounds = unit_bounds();
        // Horizontal line above bounds
        let a = IcedPoint::new(0.0, 2.0);
        let b = IcedPoint::new(1.0, 2.0);
        assert!(clip_line(&bounds, a, b).is_none());
    }

    #[test]
    fn clip_line_crossing_left_edge_clipped() {
        let bounds = unit_bounds();
        let a = IcedPoint::new(-1.0, 0.5);
        let b = IcedPoint::new(0.5, 0.5);
        let (clipped_a, clipped_b) = clip_line(&bounds, a, b).unwrap();
        assert!((clipped_a.x - 0.0).abs() < 1e-6);
        assert!((clipped_b.x - 0.5).abs() < 1e-6);
    }

    #[test]
    fn clip_line_crossing_both_sides_clipped_to_bounds() {
        let bounds = unit_bounds();
        let a = IcedPoint::new(-1.0, 0.5);
        let b = IcedPoint::new(2.0, 0.5);
        let (clipped_a, clipped_b) = clip_line(&bounds, a, b).unwrap();
        assert!((clipped_a.x - 0.0).abs() < 1e-5);
        assert!((clipped_b.x - 1.0).abs() < 1e-5);

        let a = IcedPoint::new(-0.5, 1.0);
        let b = IcedPoint::new(1.0, -0.5);
        let (clipped_a, clipped_b) = clip_line(&bounds, a, b).unwrap();
        assert!((clipped_a.x - 0.0).abs() < 1e-5);
        assert!((clipped_a.y - 0.5).abs() < 1e-5);
        assert!((clipped_b.x - 0.5).abs() < 1e-5);
        assert!((clipped_b.y - 0.0).abs() < 1e-5);
    }
}
