// SPDX-License-Identifier: GPL-3.0-or-later

mod app;
mod config;
mod panes;

use clap::Parser;
use std::path::PathBuf;

use crate::app::AppModel;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Spectrogram files to load
    #[arg(value_name = "SPECTROGRAM_PATH", required = false)]
    spectrogram_path: Vec<PathBuf>,
    /// TLE file to load
    #[arg(short, long, value_name = "TLE_PATH", requires = "frequencies_path")]
    tle_path: Option<PathBuf>,
    /// Frequencies file to load
    #[arg(short, long, value_name = "FREQUENCIES_PATH", requires = "tle_path")]
    frequencies_path: Option<PathBuf>,
}

fn main() -> iced::Result {
    env_logger::init();

    // Parse command line arguments
    let args = Args::parse();

    AppModel::create(args).run()
}
