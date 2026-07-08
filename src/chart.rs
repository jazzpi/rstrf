use copy_range::CopyRange;
use plotters::prelude::*;

use crate::util::sec_to_duration;

pub enum ReferenceMode {
    Start,
    Center,
    End,
}
pub struct ReferencedTicks {
    pub ticks: Vec<f32>,
    pub reference: f32,
    pub mode: ReferenceMode,
}

impl ReferencedTicks {
    pub fn snap(&mut self, viewport_range: CopyRange<f32>, offset: f32) {
        let [first, second, ..] = self.ticks.as_slice() else {
            return;
        };
        // Keep tick spacing from input
        let delta = second - first;

        // Don't add full offset to all computations. For frequencies in MHz--GHz range,
        // there isn't enough precision in f32 to represent the full absolute frequency. And
        // the only part that matters is the remainder relative to the delta.
        let offset = offset % delta;

        let abs_lo = offset + viewport_range.start;
        let abs_hi = offset + viewport_range.end;
        // Ticks at every multiple of `delta` (in absolute frequency) within the view,
        // stored back in the chart's offset coordinate.
        self.ticks =
            std::iter::successors(Some((abs_lo / delta).ceil() * delta), |t| Some(t + delta))
                .take_while(|t| *t <= abs_hi)
                .map(|t| t - offset)
                .collect();
        // Snap the reference to the same absolute grid.
        self.reference = match self.mode {
            ReferenceMode::Start => (abs_lo / delta).ceil() * delta - offset,
            ReferenceMode::Center => ((abs_lo + abs_hi) / 2.0 / delta).round() * delta - offset,
            ReferenceMode::End => (abs_hi / delta).floor() * delta - offset,
        };
    }
}

pub fn datetime_referenced_ticks(
    range: CopyRange<f32>,
    offset: chrono::DateTime<chrono::Utc>,
    num_ticks: usize,
) -> Vec<f32> {
    let range = (offset + sec_to_duration(range.start))..(offset + sec_to_duration(range.end));
    RangedDateTime::from(range)
        .key_points(num_ticks)
        .into_iter()
        .map(|dt| (dt - offset).as_seconds_f32())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_with_fewer_than_two_ticks_passes_through_unchanged() {
        let ticks = vec![1.0];
        let reference = 10_000.0;
        let mut referenced_ticks = ReferencedTicks {
            ticks: ticks.clone(),
            reference,
            mode: ReferenceMode::Center,
        };
        let range = CopyRange::from_std(-5000.0..25000.0);
        referenced_ticks.snap(range, 105_000.0);
        assert_eq!(referenced_ticks.ticks, ticks);
        assert_eq!(referenced_ticks.reference, reference);
    }

    #[test]
    fn axes_snaps_ticks_and_center_to_the_grid() {
        let ticks = vec![0.0, 10_000.0, 20_000.0];
        let reference = 10_000.0;
        let mut referenced_ticks = ReferencedTicks {
            ticks,
            reference,
            mode: ReferenceMode::Center,
        };
        let range = CopyRange::from_std(-5000.0..25000.0);
        referenced_ticks.snap(range, 105_000.0);
        assert_eq!(
            referenced_ticks.ticks,
            vec![-5000.0, 5000.0, 15000.0, 25000.0]
        );
        assert_eq!(referenced_ticks.reference, 15000.0);
    }

    #[test]
    fn start_reference_mode_snaps_to_the_first_grid_tick_in_view() {
        let ticks = vec![0.0, 10_000.0, 20_000.0];
        let mut referenced_ticks = ReferencedTicks {
            ticks,
            reference: 10_000.0,
            mode: ReferenceMode::Start,
        };
        let range = CopyRange::from_std(-5000.0..25000.0);
        referenced_ticks.snap(range, 105_000.0);
        assert_eq!(
            referenced_ticks.ticks,
            vec![-5000.0, 5000.0, 15000.0, 25000.0]
        );
        assert_eq!(referenced_ticks.reference, -5000.0);
    }

    #[test]
    fn end_reference_mode_snaps_to_the_last_grid_tick_in_view() {
        let ticks = vec![0.0, 10_000.0, 20_000.0];
        let mut referenced_ticks = ReferencedTicks {
            ticks,
            reference: 10_000.0,
            mode: ReferenceMode::End,
        };
        let range = CopyRange::from_std(-5000.0..25000.0);
        referenced_ticks.snap(range, 105_000.0);
        assert_eq!(
            referenced_ticks.ticks,
            vec![-5000.0, 5000.0, 15000.0, 25000.0]
        );
        assert_eq!(referenced_ticks.reference, 25000.0);
    }
}
