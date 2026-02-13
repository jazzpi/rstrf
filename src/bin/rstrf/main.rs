// SPDX-License-Identifier: GPL-3.0-or-later

mod app;
mod config;
mod panes;
mod widgets;
mod workspace;

use clap::Parser;
use std::path::PathBuf;

use crate::app::AppModel;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(value_name = "WORKSPACE", required = false)]
    workspace: Option<PathBuf>,
}

fn main() -> iced::Result {
    env_logger::init();

    // Parse command line arguments
    let args = Args::parse();

    AppModel::create(args).run()
}
