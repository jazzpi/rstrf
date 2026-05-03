// SPDX-License-Identifier: GPL-3.0-or-later

mod app;
mod config;
mod widgets;
mod windows;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

use crate::app::AppModel;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Open an RFPlot window with the given spectrograms
    Plot(PlotArgs),
}

#[derive(Args, Debug, Clone)]
pub struct PlotArgs {
    /// Spectrogram files to display
    #[arg(value_name = "SPECTROGRAMS", required = true)]
    pub spectrograms: Vec<PathBuf>,
    /// TLE catalog file
    #[arg(short = 'c', long)]
    pub catalog: Option<PathBuf>,
    /// Path to frequencies.txt
    #[arg(short = 'F', long, value_name = "FREQLIST")]
    pub freqs: Option<PathBuf>,
    /// Lower frequency limit for initial zoom (Hz)
    #[arg(long, allow_hyphen_values = true)]
    pub fmin: Option<f64>,
    /// Upper frequency limit for initial zoom (Hz)
    #[arg(long, allow_hyphen_values = true)]
    pub fmax: Option<f64>,
    /// Left time limit for initial zoom (seconds since start of spectrogram)
    #[arg(long, allow_hyphen_values = true)]
    pub tmin: Option<f64>,
    /// Right time limit for initial zoom (seconds since start of spectrogram)
    #[arg(long, allow_hyphen_values = true)]
    pub tmax: Option<f64>,
    /// Minimum power (dB)
    #[arg(long, allow_hyphen_values = true)]
    pub zmin: Option<f32>,
    /// Maximum power (dB)
    #[arg(long, allow_hyphen_values = true)]
    pub zmax: Option<f32>,
    /// Site ID written to out.dat (replaces the trailing 0)
    #[arg(short = 'C', long, value_name = "SITE_ID", default_value_t = 0)]
    pub site_id: i32,
}

fn main() -> iced::Result {
    env_logger::init();

    let args = CliArgs::parse();

    AppModel::create(args).run()
}
