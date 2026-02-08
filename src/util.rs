use std::path::PathBuf;

use iced::{Point, Rectangle};
use itertools::izip;
use ndarray::Array1;
use rfd::AsyncFileDialog;

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
