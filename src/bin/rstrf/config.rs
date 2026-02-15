// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt::Debug;

use rstrf::orbit::Site;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: String,
    pub space_track_creds: Option<(String, String)>,
    pub site: Option<Site>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            space_track_creds: None,
            site: None,
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
