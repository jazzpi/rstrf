use std::ops::{Add, AddAssign, Sub, SubAssign};

use cosmic::iced::Rectangle;
use duplicate::duplicate;
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

pub trait Coord {
    fn new(x: f32, y: f32) -> Self;
}

duplicate! {
    [name; [Screen]; [PlotArea]; [DataNormalized]; [DataAbsolute]]

    impl Coord for name {
        fn new(x: f32, y: f32) -> Self {
            Self(Vec2::new(x, y))
        }
    }

    duplicate! {
        [trait_name fn_name; [Add] [add]; [Sub] [sub]]
        impl trait_name for name {
            type Output = name;

            fn fn_name(self, rhs: Self) -> Self::Output {
                Self(self.0.fn_name(rhs.0))
            }
        }
    }

    duplicate! {
        [trait_name fn_name; [AddAssign] [add_assign]; [SubAssign] [sub_assign]]
        impl trait_name for name {
            fn fn_name(&mut self, rhs: Self) {
                self.0.fn_name(rhs.0);
            }
        }
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
