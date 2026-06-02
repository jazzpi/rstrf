// SPDX-License-Identifier: GPL-3.0-or-later

mod app;
mod config;
mod io_service;
mod pass_png;
mod widgets;
mod windows;

use clap::{ArgGroup, Args, Parser, Subcommand};
use std::path::PathBuf;

use crate::app::AppModel;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Increase rstrf log level (-v: debug, -vv: trace); RUST_LOG overrides
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Open an RFPlot window with the given spectrograms
    Plot(PlotArgs),
    /// Generate images for each pass of a given satellite
    PassPng(PassPngArgs),
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

#[derive(Args, Debug, Clone)]
#[command(group(ArgGroup::new("freq_source").required(true).args(["freq", "freqs"])))]
pub struct PassPngArgs {
    /// Spectrogram files to display
    #[arg(value_name = "SPECTROGRAMS", required = true)]
    pub spectrograms: Vec<PathBuf>,
    /// TLE catalog file
    #[arg(short = 'c', long)]
    pub catalog: PathBuf,
    /// Satellite to generate pass images for
    #[arg(short = 'i', long)]
    pub norad_id: u64,
    /// Transmitter frequency (Hz), may be specified multiple times
    #[arg(short = 'f', long, allow_hyphen_values = true)]
    pub freq: Vec<f64>,
    /// Path to frequencies.txt
    #[arg(short = 'F', long, value_name = "FREQLIST")]
    pub freqs: Option<PathBuf>,
    /// Minimum power (dB)
    #[arg(long, allow_hyphen_values = true)]
    pub zmin: Option<f32>,
    /// Maximum power (dB)
    #[arg(long, allow_hyphen_values = true)]
    pub zmax: Option<f32>,
    /// Output path prefix; files are named <prefix>_000.png, <prefix>_001.png, ...
    #[arg(short = 'o', long)]
    pub output: std::path::PathBuf,
    #[arg(short = 'w', long, default_value_t = 800)]
    pub width: u32,
    #[arg(short = 'h', long, default_value_t = 600)]
    pub height: u32,
}

fn main() -> iced::Result {
    let args = CliArgs::parse();

    let default_filter = match args.verbose {
        0 => "warn,rstrf=info,cosmic_config::dbus=off",
        1 => "warn,rstrf=debug,cosmic_config::dbus=off",
        _ => "warn,rstrf=trace,cosmic_config::dbus=off",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter))
        .init();

    AppModel::create(args).run()
}
