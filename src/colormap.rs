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

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn all_colormaps_have_256_entries() {
        for cm in Colormap::iter() {
            assert_eq!(cm.buffer().len(), 256, "{:?} has wrong number of entries", cm);
        }
    }

    #[test]
    fn all_colormap_values_are_in_unit_range() {
        for cm in Colormap::iter() {
            for (i, entry) in cm.buffer().iter().enumerate() {
                for (c, &channel) in entry.iter().enumerate() {
                    assert!(
                        channel >= 0.0 && channel <= 1.0,
                        "{:?}[{}][{}] = {} out of [0, 1]",
                        cm,
                        i,
                        c,
                        channel
                    );
                }
            }
        }
    }

    #[test]
    fn colormaps_are_not_all_zeros() {
        for cm in Colormap::iter() {
            let has_nonzero = cm.buffer().iter().any(|e| e.iter().any(|&v| v > 0.0));
            assert!(has_nonzero, "{:?} is all zeros", cm);
        }
    }
}
