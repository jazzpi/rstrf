// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt::Debug;

use iced::Theme;
use rstrf::{colormap::Colormap, orbit::Site};
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
#[serde(default)]
pub struct Config {
    pub version: String,
    pub space_track_creds: Option<(String, String)>,
    pub follow_strf_site: bool,
    pub site: Option<Site>,
    pub theme: BuiltinTheme,
    pub default_colormap: Colormap,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: "0.2.0".to_string(),
            space_track_creds: None,
            site: None,
            theme: BuiltinTheme::default(),
            default_colormap: Colormap::Viridis,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn default_config_round_trips() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let config2: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config, config2);
    }

    #[test]
    fn debug_masks_password_but_shows_username() {
        let config = Config {
            version: "0.1.0".to_string(),
            space_track_creds: Some((
                "user@example.com".to_string(),
                "s3cr3t_password".to_string(),
            )),
            ..Default::default()
        };
        let debug = format!("{:?}", config);
        assert!(
            !debug.contains("s3cr3t_password"),
            "password leaked in debug output"
        );
        assert!(
            debug.contains("user@example.com"),
            "username missing from debug output"
        );
        assert!(debug.contains("********"), "masking indicator missing");
    }

    #[test]
    fn debug_with_no_creds_does_not_panic() {
        let config = Config::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("Config"));
    }
}
