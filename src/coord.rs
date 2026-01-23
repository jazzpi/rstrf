//! This module contains newtype wrappers for the iced Point/Vector types. The wrappers allow
//! type-stated transformations between different coordinate systems used in RFPlot.
use glam::{Mat4, Quat, Vec3, Vec4};
use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};

use duplicate::{duplicate, duplicate_item};

#[duplicate_item(name; [screen]; [plot_area]; [data_normalized]; [data_absolute])]
pub mod name {
    use duplicate::duplicate_item;
    #[derive(Debug, Clone, Copy)]
    pub struct Point(pub iced::Point);
    impl Point {
        pub fn new(x: f32, y: f32) -> Self {
            Self(iced::Point::new(x, y))
        }
    }

    #[duplicate_item(type_name; [Point]; [&Point])]
    impl From<type_name> for (f32, f32) {
        fn from(point: type_name) -> Self {
            (point.0.x, point.0.y)
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Vector(pub iced::Vector);
    impl Vector {
        pub fn new(x: f32, y: f32) -> Self {
            Self(iced::Vector::new(x, y))
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Size(pub iced::Size);
    impl Size {
        pub fn new(width: f32, height: f32) -> Self {
            Self(iced::Size::new(width, height))
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Rectangle(pub iced::Rectangle);
    impl Rectangle {
        pub fn new(pos: Point, size: Size) -> Self {
            Self(iced::Rectangle::new(pos.0, size.0))
        }

        pub fn contains(&self, point: Point) -> bool {
            self.0.contains(point.0)
        }
    }
}

duplicate! {
    [name; [screen]; [plot_area]; [data_normalized]; [data_absolute]]

    // Point
    impl Sub for name::Point {
        type Output = name::Vector;
        fn sub(self, rhs: name::Point) -> Self::Output {
            name::Vector(self.0 - rhs.0)
        }
    }
    #[duplicate_item(
        trait_name fn_name;
        [Add] [add];
        [Sub] [sub]
    )]
    impl trait_name<name::Vector> for name::Point {
        type Output = name::Point;
        fn fn_name(self, rhs: name::Vector) -> Self::Output {
            name::Point(self.0.fn_name(rhs.0))
        }
    }

    // Vector
    #[duplicate_item(
        trait_name fn_name;
        [Add] [add];
        [Sub] [sub]
    )]
    impl trait_name for name::Vector {
        type Output = name::Vector;
        fn fn_name(self, rhs: name::Vector) -> Self::Output {
            name::Vector(self.0.fn_name(rhs.0))
        }
    }
    impl Mul<f32> for name::Vector {
        type Output = name::Vector;
        fn mul(self, rhs: f32) -> Self::Output {
            name::Vector(self.0.mul(rhs))
        }
    }

    #[duplicate_item(
        trait_name fn_name;
        [AddAssign] [add_assign];
        [SubAssign] [sub_assign]
    )]
    impl trait_name<name::Vector> for name::Point {
        fn fn_name(&mut self, rhs: name::Vector) {
            self.0.fn_name(rhs.0);
        }
    }

    // TODO: Now that we don't use libcosmic's iced anymore, we should be able to add a few more
    // trait impls
}

#[duplicate_item(
    name;
    [ScreenToPlotArea]; [ScreenToDataNormalized]; [ScreenToDataAbsolute];
    [PlotAreaToScreen]; [PlotAreaToDataNormalized]; [PlotAreaToDataAbsolute];
    [DataNormalizedToScreen]; [DataNormalizedToPlotArea]; [DataNormalizedToDataAbsolute];
    [DataAbsoluteToScreen]; [DataAbsoluteToPlotArea]; [DataAbsoluteToDataNormalized]
)]
#[derive(Debug, Clone, Copy)]
pub struct name(Mat4);

duplicate! {
    [from to transform;
    [screen] [plot_area] [ScreenToPlotArea];
    [screen] [data_normalized] [ScreenToDataNormalized];
    [screen] [data_absolute] [ScreenToDataAbsolute];
    [plot_area] [screen] [PlotAreaToScreen];
    [plot_area] [data_normalized] [PlotAreaToDataNormalized];
    [plot_area] [data_absolute] [PlotAreaToDataAbsolute];
    [data_normalized] [screen] [DataNormalizedToScreen];
    [data_normalized] [plot_area] [DataNormalizedToPlotArea];
    [data_normalized] [data_absolute] [DataNormalizedToDataAbsolute];
    [data_absolute] [screen] [DataAbsoluteToScreen];
    [data_absolute] [plot_area] [DataAbsoluteToPlotArea];
    [data_absolute] [data_normalized] [DataAbsoluteToDataNormalized]]
    // Would be real nice if we could just use iced's impl Mul<Transformation> here... But we can't
    // construct arbitrary Transformations, so we need to use Mat4s instead and re-implement the
    // Muls here.
    impl Mul<transform> for from::Point {
        type Output = to::Point;
        fn mul(self, tf: transform) -> Self::Output {
            let result = tf.0.mul_vec4(Vec4::new(self.0.x, self.0.y, 1.0, 1.0));
            to::Point(iced::Point::new(result.x, result.y))
        }
    }

    impl Mul<transform> for from::Vector {
        type Output = to::Vector;
        fn mul(self, tf: transform) -> Self::Output {
            let result = tf.0.mul_vec4(Vec4::new(self.0.x, self.0.y, 1.0, 0.0));
            to::Vector(iced::Vector::new(result.x, result.y))
        }
    }

    impl Mul<transform> for from::Size {
        type Output = to::Size;
        fn mul(self, tf: transform) -> Self::Output {
            let result = tf.0.mul_vec4(Vec4::new(
                self.0.width,
                self.0.height,
                1.0,
                0.0,
            ));
            to::Size(iced::Size::new(result.x, result.y))
        }
    }

    impl Mul<transform> for from::Rectangle {
        type Output = to::Rectangle;
        fn mul(self, tf: transform) -> Self::Output {
            let position = from::Point(self.0.position());
            let size = from::Size(self.0.size());

            to::Rectangle::new(position * tf, size * tf)
        }
    }
}

impl ScreenToPlotArea {
    pub fn new(size: &screen::Size) -> Self {
        let scale = Vec3::new(1.0 / size.0.width, -1.0 / size.0.height, 1.0);
        let translation = Vec3::new(0.0, 1.0, 0.0);
        Self(Mat4::from_scale_rotation_translation(
            scale,
            Quat::IDENTITY,
            translation,
        ))
    }
}
impl PlotAreaToDataNormalized {
    pub fn new(bounds: &data_normalized::Rectangle) -> Self {
        let scale = Vec3::new(bounds.0.width, bounds.0.height, 1.0);
        let translation = Vec3::new(bounds.0.x, bounds.0.y, 0.0);
        Self(Mat4::from_scale_rotation_translation(
            scale,
            Quat::IDENTITY,
            translation,
        ))
    }
}
impl DataNormalizedToDataAbsolute {
    pub fn new(bounds: &data_absolute::Rectangle) -> Self {
        let scale = Vec3::new(bounds.0.width, bounds.0.height, 1.0);
        let translation = Vec3::new(bounds.0.x, bounds.0.y, 0.0);
        Self(Mat4::from_scale_rotation_translation(
            scale,
            Quat::IDENTITY,
            translation,
        ))
    }
}

impl ScreenToDataNormalized {
    pub fn new(size: &screen::Size, bounds: &data_normalized::Rectangle) -> Self {
        Self(ScreenToPlotArea::new(size).0 * PlotAreaToDataNormalized::new(bounds).0)
    }
}
impl ScreenToDataAbsolute {
    pub fn new(
        size: &screen::Size,
        norm_bounds: &data_normalized::Rectangle,
        abs_bounds: &data_absolute::Rectangle,
    ) -> Self {
        Self(
            DataNormalizedToDataAbsolute::new(abs_bounds).0
                * PlotAreaToDataNormalized::new(norm_bounds).0
                * ScreenToPlotArea::new(size).0,
        )
    }
}
impl PlotAreaToScreen {
    pub fn new(size: &screen::Size) -> Self {
        Self(ScreenToPlotArea::new(size).0.inverse())
    }
}
impl PlotAreaToDataAbsolute {
    pub fn new(
        norm_bounds: &data_normalized::Rectangle,
        abs_bounds: &data_absolute::Rectangle,
    ) -> Self {
        Self(
            DataNormalizedToDataAbsolute::new(abs_bounds).0
                * PlotAreaToDataNormalized::new(norm_bounds).0,
        )
    }
}
impl DataNormalizedToScreen {
    pub fn new(size: &screen::Size, bounds: &data_normalized::Rectangle) -> Self {
        Self(ScreenToDataNormalized::new(size, bounds).0.inverse())
    }
}
impl DataNormalizedToPlotArea {
    pub fn new(bounds: &data_normalized::Rectangle) -> Self {
        Self(PlotAreaToDataNormalized::new(bounds).0.inverse())
    }
}
impl DataAbsoluteToScreen {
    pub fn new(
        size: &screen::Size,
        norm_bounds: &data_normalized::Rectangle,
        abs_bounds: &data_absolute::Rectangle,
    ) -> Self {
        Self(
            ScreenToDataAbsolute::new(size, norm_bounds, abs_bounds)
                .0
                .inverse(),
        )
    }
}
impl DataAbsoluteToPlotArea {
    pub fn new(
        norm_bounds: &data_normalized::Rectangle,
        abs_bounds: &data_absolute::Rectangle,
    ) -> Self {
        Self(
            PlotAreaToDataAbsolute::new(norm_bounds, abs_bounds)
                .0
                .inverse(),
        )
    }
}
impl DataAbsoluteToDataNormalized {
    pub fn new(bounds: &data_absolute::Rectangle) -> Self {
        Self(DataNormalizedToDataAbsolute::new(bounds).0.inverse())
    }
}
