// SPDX-License-Identifier: GPL-3.0-or-later

//! Converts strf `.bin` files to the constant-rate `.rstrf` format.
//!
//! This is a thin wrapper around the library's resampling logic.
//! The `.rstrf` format is pre-converted for faster subsequent loads;
//! `rstrf plot` can load `.bin` files directly without pre-conversion.

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use std::path::PathBuf;

use rstrf::spectrogram::{self, RawStrfSpectrum, StrfParams};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Convert strf .bin files to constant-rate .rstrf format"
)]
struct Args {
    /// Input strf .bin file(s)
    #[arg(value_name = "INPUT", required = true, num_args = 1..)]
    input: Vec<PathBuf>,

    /// Output .rstrf file
    #[arg(value_name = "OUTPUT")]
    output: PathBuf,

    /// Override nominal slice length in seconds (default: median of inter-spectrum gaps)
    #[arg(short = 's', long, value_name = "SECS")]
    slice_length: Option<f64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();

    let mut all_spectra: Vec<RawStrfSpectrum> = Vec::new();
    let mut params: Option<StrfParams> = None;

    for path in &args.input {
        let (spectra, file_params) = spectrogram::load_strf_raw(path)
            .await
            .context(format!("Failed to load {}", path.display()))?;

        if let Some(ref p) = params {
            ensure!(
                p.freq == file_params.freq
                    && p.bw == file_params.bw
                    && p.nchan == file_params.nchan,
                "Inconsistent parameters between input files"
            );
        } else {
            params = Some(file_params);
        }

        all_spectra.extend(spectra);
    }

    if all_spectra.is_empty() {
        bail!("No spectra found in input files");
    }

    let params = params.unwrap();
    let spectrogram = spectrogram::resample_strf(all_spectra, &params, args.slice_length)
        .context("Failed to resample spectra")?;

    spectrogram::save(&spectrogram, &args.output)
        .await
        .context("Failed to write output file")?;

    println!(
        "Wrote {} slices × {} channels to {}",
        spectrogram.nslices,
        spectrogram.nchan,
        args.output.display()
    );

    Ok(())
}
