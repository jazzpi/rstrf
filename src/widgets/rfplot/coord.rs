use std::ops::{Add, Sub};

use cosmic::iced::Rectangle;
use duplicate::duplicate_item;
use glam::Vec2;

use super::Controls;

#[derive(Debug, Clone, Copy)]
/// Screen coordinates in pixels
pub struct Screen(pub Vec2);
#[derive(Debug, Clone, Copy)]
/// Coordinates normalized to the plot area (can be outside [0, 1] e.g. above the axes)
pub struct PlotArea(pub Vec2);
#[derive(Debug, Clone, Copy)]
/// Coordinates in data space, normalized to the data range
pub struct DataNormalized(pub Vec2);
#[derive(Debug, Clone, Copy)]
/// Coordinates in data space
pub struct DataAbsolute(pub Vec2);

#[duplicate_item(name; [Screen]; [PlotArea]; [DataNormalized]; [DataAbsolute])]
impl Sub for name {
    type Output = name;

    fn sub(self, rhs: Self) -> Self::Output {
        name(self.0 - rhs.0)
    }
}
#[duplicate_item(name; [Screen]; [PlotArea]; [DataNormalized]; [DataAbsolute])]
impl Add for name {
    type Output = name;

    fn add(self, rhs: Self) -> Self::Output {
        name(self.0 + rhs.0)
    }
}

impl Screen {
    pub fn plot(&self, bounds: &Rectangle) -> PlotArea {
        let x = self.0.x / bounds.width;
        let y = 1.0 - self.0.y / bounds.height;
        PlotArea(Vec2::new(x, y))
    }

    pub fn data_normalized(&self, bounds: &Rectangle, controls: &Controls) -> DataNormalized {
        self.plot(bounds).data_normalized(controls)
    }
}

impl PlotArea {
    pub fn data_normalized(&self, controls: &Controls) -> DataNormalized {
        DataNormalized(controls.center + (self.0 - Vec2::splat(0.5)) * controls.scale())
    }
}
