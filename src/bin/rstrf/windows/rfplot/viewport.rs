use glam::Vec2;
use rstrf::coord::{
    DataAbsoluteToDataNormalized, PlotAreaToDataNormalized, data_absolute, data_normalized,
    plot_area,
};
use serde::{Deserialize, Serialize};

const ZOOM_MIN: f32 = 0.0;
const ZOOM_MAX: f32 = 8.0;

const MIN_FREQ_SPAN_HZ: f32 = 10e3;
const MIN_TIME_SPAN_S: f32 = 60.0;

const ZOOM_WHEEL_SCALE: f32 = 0.2;

/// The visible region of the spectrogram, in data-normalized (unit-square) coordinates.
///
/// Every mutator clamps zoom to `[0, zoom_max]` and snaps the resulting view back inside
/// `[0, 1]` on both axes before returning, so `bounds()` never needs to be checked by callers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Viewport {
    /// Per-axis zoom ceiling
    zoom_max: Vec2,
    log_scale: Vec2,
    center: data_normalized::Point,
}

impl Viewport {
    pub fn set_zoom_x(&mut self, zoom_x: f32) {
        self.log_scale.x = zoom_x.clamp(ZOOM_MIN, self.zoom_max.x);
        self.snap_to_bounds();
    }

    pub fn set_zoom_y(&mut self, zoom_y: f32) {
        self.log_scale.y = zoom_y.clamp(ZOOM_MIN, self.zoom_max.y);
        self.snap_to_bounds();
    }

    pub fn pan_by(&mut self, delta: plot_area::Vector) {
        self.center -= delta * self.data_normalized();
        self.snap_to_bounds();
    }

    pub fn zoom_at(&mut self, plot_pos: plot_area::Point, delta: f32) {
        let delta = delta * ZOOM_WHEEL_SCALE;
        let old_data = plot_pos * self.data_normalized();
        let prev_zoom = self.log_scale;
        self.set_scale(prev_zoom + Vec2::splat(delta));
        let new_data = plot_pos * self.data_normalized();
        self.center += old_data - new_data;
        self.snap_to_bounds();
    }

    pub fn zoom_x_at(&mut self, plot_pos: plot_area::Point, delta: f32) {
        let delta = delta * ZOOM_WHEEL_SCALE;
        let old_x = (plot_pos * self.data_normalized()).0.x;
        self.set_scale(self.log_scale.with_x(self.log_scale.x + delta));
        let new_x = (plot_pos * self.data_normalized()).0.x;
        self.center.0.x += old_x - new_x;
        self.snap_to_bounds();
    }

    pub fn zoom_y_at(&mut self, plot_pos: plot_area::Point, delta: f32) {
        let delta = delta * ZOOM_WHEEL_SCALE;
        let old_y = (plot_pos * self.data_normalized()).0.y;
        self.set_scale(self.log_scale.with_y(self.log_scale.y + delta));
        let new_y = (plot_pos * self.data_normalized()).0.y;
        self.center.0.y += old_y - new_y;
        self.snap_to_bounds();
    }

    pub fn reset(&mut self) {
        self.log_scale = Vec2::new(ZOOM_MIN, ZOOM_MIN);
        self.center = data_normalized::Point::new(0.5, 0.5);
    }

    /// Set the view to show `rect`, clamping zoom to the allowed range.
    pub fn set_view_from_rect_da(
        &mut self,
        rect: &data_absolute::Rectangle,
        spec_bounds: &data_absolute::Rectangle,
    ) {
        let to_norm = DataAbsoluteToDataNormalized::new(spec_bounds);
        let norm_rect = *rect * to_norm;
        self.set_view_from_rect_dn(&norm_rect);
    }

    pub fn set_view_from_rect_dn(&mut self, rect: &data_normalized::Rectangle) {
        let width = rect.0.width.max(1e-6);
        let height = rect.0.height.max(1e-6);
        self.set_scale(Vec2::new(1.0_f32 / width, 1.0_f32 / height).log2());
        self.center = data_normalized::Point::new(rect.0.x + width / 2.0, rect.0.y + height / 2.0);
        self.snap_to_bounds();
    }

    pub fn set_data_bounds(&mut self, total_time_s: f32, total_bw_hz: f32) {
        self.zoom_max = Vec2::new(
            (total_time_s / MIN_TIME_SPAN_S).log2().max(ZOOM_MIN),
            (total_bw_hz / MIN_FREQ_SPAN_HZ).log2().max(ZOOM_MIN),
        );
        // Reapply zoom_max
        self.set_scale(self.log_scale);
        self.snap_to_bounds();
    }

    fn set_scale(&mut self, log_scale: Vec2) {
        self.log_scale = log_scale.clamp(Vec2::splat(ZOOM_MIN), self.zoom_max);
    }

    pub fn size(&self) -> data_normalized::Size {
        data_normalized::Size::new(
            1.0 / 2.0_f32.powf(self.log_scale.x),
            1.0 / 2.0_f32.powf(self.log_scale.y),
        )
    }

    pub fn bounds(&self) -> data_normalized::Rectangle {
        let size = self.size();
        data_normalized::Rectangle::new(
            data_normalized::Point::new(
                self.center.0.x - size.0.width / 2.0,
                self.center.0.y - size.0.height / 2.0,
            ),
            size,
        )
    }

    pub fn data_normalized(&self) -> PlotAreaToDataNormalized {
        PlotAreaToDataNormalized::new(&self.bounds())
    }

    pub fn log_scale(&self) -> Vec2 {
        self.log_scale
    }

    pub fn zoom_max(&self) -> Vec2 {
        self.zoom_max
    }

    /// Ensure that the current view bounds are within [0, 1] in both axes.
    fn snap_to_bounds(&mut self) {
        let bounds = self.bounds().0;
        let dx = if bounds.x < 0.0 {
            -bounds.x
        } else if bounds.x + bounds.width > 1.0 {
            1.0 - (bounds.x + bounds.width)
        } else {
            0.0
        };
        let dy = if bounds.y < 0.0 {
            -bounds.y
        } else if bounds.y + bounds.height > 1.0 {
            1.0 - (bounds.y + bounds.height)
        } else {
            0.0
        };
        self.center.0.x += dx;
        self.center.0.y += dy;
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            log_scale: Vec2::new(ZOOM_MIN, ZOOM_MIN),
            center: data_normalized::Point::new(0.5, 0.5),
            zoom_max: Vec2::splat(ZOOM_MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_size_is_full_view() {
        let v = Viewport::default();
        assert!((v.size().0.width - 1.0).abs() < 1e-6);
        assert!((v.size().0.height - 1.0).abs() < 1e-6);
    }

    #[test]
    fn default_bounds_covers_unit_square() {
        let v = Viewport::default();
        let b = v.bounds();
        assert!((b.0.x - 0.0).abs() < 1e-6);
        assert!((b.0.y - 0.0).abs() < 1e-6);
        assert!((b.0.width - 1.0).abs() < 1e-6);
        assert!((b.0.height - 1.0).abs() < 1e-6);
    }

    #[test]
    fn update_zoom_x_changes_width() {
        let mut v = Viewport::default();
        v.set_zoom_x(2.0);
        // 1 / 2^2 = 0.25
        assert!((v.size().0.width - 0.25).abs() < 1e-6);
        assert!((v.size().0.height - 1.0).abs() < 1e-6);
    }

    #[test]
    fn reset_view_restores_full_view() {
        let mut v = Viewport::default();
        v.set_zoom_x(5.0);
        v.set_zoom_y(3.0);
        v.reset();
        assert!((v.size().0.width - 1.0).abs() < 1e-6);
        assert!((v.size().0.height - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pan_large_delta_snaps_back_in_bounds() {
        let mut v = Viewport::default();
        v.pan_by(plot_area::Vector::new(10.0, 0.0));
        let b = v.bounds();
        assert!(b.0.x >= -1e-5, "x={}", b.0.x);
        assert!(
            b.0.x + b.0.width <= 1.0 + 1e-5,
            "right edge={}",
            b.0.x + b.0.width
        );
    }

    fn assert_bounds_in_unit_square(b: data_normalized::Rectangle) {
        assert!(b.0.x >= -1e-5, "x={}", b.0.x);
        assert!(
            b.0.x + b.0.width <= 1.0 + 1e-5,
            "right edge={}",
            b.0.x + b.0.width
        );
        assert!(b.0.y >= -1e-5, "y={}", b.0.y);
        assert!(
            b.0.y + b.0.height <= 1.0 + 1e-5,
            "top edge={}",
            b.0.y + b.0.height
        );
    }

    #[test]
    fn zoom_delta_snaps_back_in_bounds() {
        let mut v = Viewport::default();
        v.set_zoom_x(6.0);
        v.set_zoom_y(6.0);
        v.pan_by(plot_area::Vector::new(-10.0, -10.0));
        // Zoom out anchored at the corner opposite the one the view is now pinned
        // against, pushing the pinned corner further past the [0, 1] bound.
        v.zoom_at(plot_area::Point::new(0.0, 0.0), -1000.0);
        assert_bounds_in_unit_square(v.bounds());
    }

    #[test]
    fn zoom_delta_x_snaps_back_in_bounds() {
        let mut v = Viewport::default();
        v.set_zoom_x(6.0);
        v.pan_by(plot_area::Vector::new(-10.0, 0.0));
        v.zoom_x_at(plot_area::Point::new(0.0, 0.0), -1000.0);
        assert_bounds_in_unit_square(v.bounds());
    }

    #[test]
    fn zoom_delta_y_snaps_back_in_bounds() {
        let mut v = Viewport::default();
        v.set_zoom_y(6.0);
        v.pan_by(plot_area::Vector::new(0.0, -10.0));
        v.zoom_y_at(plot_area::Point::new(0.0, 0.0), -1000.0);
        assert_bounds_in_unit_square(v.bounds());
    }

    #[test]
    fn set_data_bounds_snaps_view_when_span_shrinks() {
        let mut v = Viewport::default();
        v.set_data_bounds(10_000.0, 1_000_000.0);
        v.set_zoom_x(8.0);
        v.set_zoom_y(8.0);
        v.pan_by(plot_area::Vector::new(-10.0, -10.0));
        // A much smaller span shrinks zoom_max below the current log_scale, so the
        // clamp in set_scale grows the view back out around the still-pinned center.
        v.set_data_bounds(60.0, 10_000.0);
        assert_bounds_in_unit_square(v.bounds());
    }
}
