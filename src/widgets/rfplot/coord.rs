use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};

use copy_range::CopyRange;
use cosmic::iced::Rectangle;
use duplicate::duplicate;
use glam::Vec2;
use itertools::izip;

use super::{Controls, Spectrogram};

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

    pub fn data_absolute(
        &self,
        bounds: &Rectangle,
        controls: &Controls,
        spectrogram: &Spectrogram,
    ) -> DataAbsolute {
        self.data_normalized(bounds, controls)
            .data_absolute(spectrogram)
    }
}

impl PlotArea {
    pub fn data_normalized(&self, controls: &Controls) -> DataNormalized {
        controls.center() + DataNormalized((self.0 - Vec2::splat(0.5)) * controls.scale())
    }
}

impl DataNormalized {
    pub fn data_absolute(&self, spectrogram: &Spectrogram) -> DataAbsolute {
        DataAbsolute::new(
            self.0.x * spectrogram.length().as_seconds_f32(),
            (self.0.y - 0.5) * spectrogram.bw,
        )
    }
}

#[derive(Debug, Clone)]
pub struct Bounds {
    pub x: CopyRange<f32>,
    pub y: CopyRange<f32>,
}

impl Bounds {
    pub fn new(x: impl Into<CopyRange<f32>>, y: impl Into<CopyRange<f32>>) -> Self {
        Self {
            x: x.into(),
            y: y.into(),
        }
    }

    pub fn contains(&self, point: &Vec2) -> bool {
        point.x >= self.x.start
            && point.x <= self.x.end
            && point.y >= self.y.start
            && point.y <= self.y.end
    }
}

duplicate! {
    [trait_name fn_name; [Add] [add];[Sub] [sub]; [Mul] [mul]]
    impl trait_name<Vec2> for Bounds {
        type Output = Bounds;

        fn fn_name(self, rhs: Vec2) -> Self::Output {
            Bounds::new(
                (self.x.start.fn_name(rhs.x))..(self.x.end.fn_name(rhs.x)),
                (self.y.start.fn_name(rhs.y))..(self.y.end.fn_name(rhs.y)),
            )
        }
    }
}

pub fn clip_line(bounds: &Bounds, a: Vec2, b: Vec2) -> Option<(Vec2, Vec2)> {
    // https://en.wikipedia.org/wiki/Liang%E2%80%93Barsky_algorithm
    let delta = b - a;

    let pv = [-delta.x, delta.x, -delta.y, delta.y];
    let qv = [
        a.x - bounds.x.start,
        bounds.x.end - a.x,
        a.y - bounds.y.start,
        bounds.y.end - a.y,
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
