// SPDX-License-Identifier: GPL-3.0-or-later

mod app;
mod config;
mod i18n;
mod widgets;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Spectrogram files to load
    #[arg(value_name = "SPECTROGRAM_PATH", required = true)]
    spectrogram_path: Vec<PathBuf>,
    /// TLE file to load
    #[arg(short, long, value_name = "TLE_PATH")]
    tle_path: Option<PathBuf>,
}

fn main() -> cosmic::iced::Result {
    env_logger::init();

    // Parse command line arguments
    let args = Args::parse();

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Settings for configuring the application window and iced runtime.
    let settings = cosmic::app::Settings::default().size_limits(
        cosmic::iced::Limits::NONE
            .min_width(360.0)
            .min_height(180.0),
    );

    // Starts the application's event loop with `()` as the application's flags.
    cosmic::app::run::<app::AppModel>(settings, args)
}
