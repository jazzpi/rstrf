// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt::Debug;

use iced::Theme;
use rstrf::orbit::Site;
use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(
    Debug, Display, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, strum::VariantArray,
)]
pub enum BuiltinTheme {
    Light,
    #[default]
    Dark,
    Dracula,
    Nord,
    SolarizedLight,
    SolarizedDark,
    GruvboxLight,
    GruvboxDark,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    CatppuccinMocha,
    TokyoNight,
    TokyoNightStorm,
    TokyoNightLight,
    KanagawaWave,
    KanagawaDragon,
    KanagawaLotus,
    Moonfly,
    Nightfly,
    Oxocarbon,
    Ferra,
}

impl From<BuiltinTheme> for Theme {
    fn from(value: BuiltinTheme) -> Self {
        match value {
            BuiltinTheme::Light => Theme::Light,
            BuiltinTheme::Dark => Theme::Dark,
            BuiltinTheme::Dracula => Theme::Dracula,
            BuiltinTheme::Nord => Theme::Nord,
            BuiltinTheme::SolarizedLight => Theme::SolarizedLight,
            BuiltinTheme::SolarizedDark => Theme::SolarizedDark,
            BuiltinTheme::GruvboxLight => Theme::GruvboxLight,
            BuiltinTheme::GruvboxDark => Theme::GruvboxDark,
            BuiltinTheme::CatppuccinLatte => Theme::CatppuccinLatte,
            BuiltinTheme::CatppuccinFrappe => Theme::CatppuccinFrappe,
            BuiltinTheme::CatppuccinMacchiato => Theme::CatppuccinMacchiato,
            BuiltinTheme::CatppuccinMocha => Theme::CatppuccinMocha,
            BuiltinTheme::TokyoNight => Theme::TokyoNight,
            BuiltinTheme::TokyoNightStorm => Theme::TokyoNightStorm,
            BuiltinTheme::TokyoNightLight => Theme::TokyoNightLight,
            BuiltinTheme::KanagawaWave => Theme::KanagawaWave,
            BuiltinTheme::KanagawaDragon => Theme::KanagawaDragon,
            BuiltinTheme::KanagawaLotus => Theme::KanagawaLotus,
            BuiltinTheme::Moonfly => Theme::Moonfly,
            BuiltinTheme::Nightfly => Theme::Nightfly,
            BuiltinTheme::Oxocarbon => Theme::Oxocarbon,
            BuiltinTheme::Ferra => Theme::Ferra,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: String,
    pub space_track_creds: Option<(String, String)>,
    pub site: Option<Site>,
    pub theme: BuiltinTheme,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            space_track_creds: None,
            site: None,
            theme: BuiltinTheme::default(),
        }
    }
}

impl Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field(
                "space_track_creds",
                &self
                    .space_track_creds
                    .as_ref()
                    .map(|(user, _)| Some((user, "********")))
                    .unwrap_or(None),
            )
            .finish()
    }
}
