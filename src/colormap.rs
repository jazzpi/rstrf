use serde::{Deserialize, Serialize};
use strum::{EnumIter, IntoStaticStr};

mod data;
pub use data::{CIVIDIS, INFERNO, MAGMA, MAKO, PLASMA, ROCKET, TURBO, VIRIDIS};

#[derive(
    Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, EnumIter, IntoStaticStr,
)]
pub enum Colormap {
    #[default]
    Magma,
    Inferno,
    Plasma,
    Viridis,
    Cividis,
    Rocket,
    Mako,
    Turbo,
}

impl From<Colormap> for &ColormapBuffer {
    fn from(colormap: Colormap) -> Self {
        match colormap {
            Colormap::Magma => &MAGMA,
            Colormap::Inferno => &INFERNO,
            Colormap::Plasma => &PLASMA,
            Colormap::Viridis => &VIRIDIS,
            Colormap::Cividis => &CIVIDIS,
            Colormap::Rocket => &ROCKET,
            Colormap::Mako => &MAKO,
            Colormap::Turbo => &TURBO,
        }
    }
}

impl Colormap {
    pub fn buffer(&self) -> &ColormapBuffer {
        (*self).into()
    }
}

pub type ColormapBuffer = [[f32; 4]; 256];
