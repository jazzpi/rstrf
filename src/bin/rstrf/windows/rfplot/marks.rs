//! The marks the user places on the plot: track points and detected signals.

use chrono::{DateTime, Utc};
use rstrf::{coord::data_absolute, util::sec_to_duration};
use serde::{Deserialize, Serialize};

/// Which of the two mark collections a mark belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkAction {
    Trackpoint,
    Signal,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct Marks {
    /// Sorted by time.
    track_points: Vec<data_absolute::Point>,
    signals: Vec<data_absolute::Point>,
}

impl Marks {
    pub fn track_points(&self) -> &[data_absolute::Point] {
        &self.track_points
    }

    pub fn signals(&self) -> &[data_absolute::Point] {
        &self.signals
    }

    /// Signals carry no ordering invariant, so callers may mutate them freely.
    pub fn signals_mut(&mut self) -> &mut Vec<data_absolute::Point> {
        &mut self.signals
    }

    /// Inserts a track point in time order. A point at an already-marked time replaces it.
    pub fn insert_track_point(&mut self, point: data_absolute::Point) {
        match self
            .track_points
            .binary_search_by(|p| p.0.x.partial_cmp(&point.0.x).unwrap())
        {
            Ok(idx) => self.track_points[idx] = point,
            Err(idx) => self.track_points.insert(idx, point),
        }
    }

    pub fn remove(&mut self, action: MarkAction, point: data_absolute::Point) {
        let collection = match action {
            MarkAction::Trackpoint => &mut self.track_points,
            MarkAction::Signal => &mut self.signals,
        };
        if let Some(idx) = collection.iter().position(|p| *p == point) {
            collection.remove(idx);
        }
    }

    /// Drops marks failing `keep` from both collections.
    pub fn retain(&mut self, keep: impl Fn(&data_absolute::Point) -> bool) {
        self.track_points.retain(&keep);
        self.signals.retain(&keep);
    }

    pub fn clear(&mut self) {
        self.track_points.clear();
        self.signals.clear();
    }
}

/// Suggests a save filename for a signal set: `YYYY-MM-DDTHH:MM_FREQ.dat`.
///
/// Returns `None` when `signals` is empty (no mean is defined).
pub(super) fn signals_filename(
    start_time: DateTime<Utc>,
    center_freq: f64,
    signals: &[data_absolute::Point],
) -> Option<String> {
    if signals.is_empty() {
        return None;
    }
    let n = signals.len() as f64;
    let mean_secs = signals.iter().map(|s| s.0.x as f64).sum::<f64>() / n;
    let mean_freq = center_freq + signals.iter().map(|s| s.0.y as f64).sum::<f64>() / n;
    let mean_time = start_time + sec_to_duration(mean_secs);
    Some(format!(
        "{}_{:.0}k.dat",
        mean_time.format("%Y-%m-%dT%H:%M"),
        mean_freq / 1e3,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn pt(x: f32, y: f32) -> data_absolute::Point {
        data_absolute::Point::new(x, y)
    }

    fn utc(year: i32, month: u32, day: u32, hour: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, 0)
            .unwrap()
    }

    #[test]
    fn insert_track_point_keeps_points_sorted_by_time() {
        let mut marks = Marks::default();
        for x in [3.0, 1.0, 2.0] {
            marks.insert_track_point(pt(x, 0.0));
        }
        let times: Vec<f32> = marks.track_points().iter().map(|p| p.0.x).collect();
        assert_eq!(times, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn insert_track_point_replaces_point_at_same_time() {
        let mut marks = Marks::default();
        marks.insert_track_point(pt(1.0, 10.0));
        marks.insert_track_point(pt(1.0, 20.0));
        assert_eq!(marks.track_points().to_vec(), vec![pt(1.0, 20.0)]);
    }

    #[test]
    fn empty_signals_returns_none() {
        assert_eq!(
            signals_filename(utc(2024, 1, 1, 0, 0), 437_000_000.0, &[]),
            None
        );
    }

    #[test]
    fn single_signal_at_center() {
        // One signal exactly at the center: mean time = start, mean freq = center_freq
        let name = signals_filename(utc(2024, 6, 15, 12, 30), 145_900_000.0, &[pt(0.0, 0.0)]);
        assert_eq!(name.as_deref(), Some("2024-06-15T12:30_145900k.dat"));
    }

    #[test]
    fn single_signal_with_offsets() {
        // 60 s into the observation, +1000 Hz from center → 145901 kHz
        let name = signals_filename(utc(2024, 6, 15, 12, 30), 145_900_000.0, &[pt(60.0, 1000.0)]);
        assert_eq!(name.as_deref(), Some("2024-06-15T12:31_145901k.dat"));
    }

    #[test]
    fn mean_of_multiple_signals() {
        // Two signals 120 s apart → mean at +60 s → :31
        // freqs +0 and +200 → mean +100 Hz → 145900 kHz
        let signals = [pt(0.0, 0.0), pt(120.0, 200.0)];
        let name = signals_filename(utc(2024, 6, 15, 12, 30), 145_900_000.0, &signals);
        assert_eq!(name.as_deref(), Some("2024-06-15T12:31_145900k.dat"));
    }

    #[test]
    fn minute_boundary_rollover() {
        // Start at 12:59; 61 s offset rolls over to 13:00
        let name = signals_filename(utc(2024, 6, 15, 12, 59), 437_525_000.0, &[pt(61.0, 0.0)]);
        assert_eq!(name.as_deref(), Some("2024-06-15T13:00_437525k.dat"));
    }

    #[test]
    fn negative_freq_offset() {
        // Negative offset: center 437.525 MHz, −525 Hz → 437524.475 kHz → 437524k
        let name = signals_filename(utc(2024, 1, 1, 0, 0), 437_525_000.0, &[pt(0.0, -525.0)]);
        assert_eq!(name.as_deref(), Some("2024-01-01T00:00_437524k.dat"));
    }
}
