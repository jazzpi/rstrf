// SPDX-License-Identifier: GPL-3.0-or-later

//! Converts strf `.bin` files to the constant-rate `.rstrf` format.
//!
//! Handles non-constant recording rates by resampling to a uniform time grid.
//! Spectra are mapped to the nearest grid slot; gaps are filled with -120 dB.

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use ndarray::ArcArray2;
use std::path::PathBuf;
use uuid::Uuid;

use rstrf::spectrogram::{self, FILL_DB, RawStrfSpectrum, Spectrogram, StrfParams};

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

    // Load all input files
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

    let params = params.unwrap(); // guaranteed non-empty by clap

    if all_spectra.is_empty() {
        bail!("No spectra found in input files");
    }

    // Sort by timestamp (files may be provided out of order)
    all_spectra.sort_unstable_by_key(|s| s.time);

    let nchan = params.nchan;

    // Compute nominal slice length as median of inter-spectrum gaps
    let slice_length_s = match args.slice_length {
        Some(s) => {
            ensure!(s > 0.0, "Slice length must be positive");
            s
        }
        None => {
            let mut gaps: Vec<f64> = all_spectra
                .windows(2)
                .map(|w| {
                    (w[1].time - w[0].time).num_microseconds().unwrap_or(0) as f64 / 1_000_000.0
                })
                .collect();

            if gaps.is_empty() {
                bail!("Need at least two spectra to determine slice length; use --slice-length");
            }

            gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
            gaps[gaps.len() / 2]
        }
    };

    log::info!(
        "Using slice length: {:.6} s ({} input spectra, {} channels)",
        slice_length_s,
        all_spectra.len(),
        nchan
    );

    let start_time = all_spectra[0].time;
    let last_time = all_spectra.last().unwrap().time;

    let nslices = (((last_time - start_time).num_microseconds().unwrap_or(0) as f64
        / 1_000_000.0
        / slice_length_s)
        .round() as usize)
        + 1;

    log::info!(
        "Output: {} slices ({} total values)",
        nslices,
        nslices * nchan
    );

    // -120 dB ≈ 10*log10(1e-12), the sentinel for missing data
    let mut data_db = vec![FILL_DB; nslices * nchan];

    for spectrum in &all_spectra {
        let offset_s =
            (spectrum.time - start_time).num_microseconds().unwrap_or(0) as f64 / 1_000_000.0;
        let slot = (offset_s / slice_length_s).round() as usize;
        if slot < nslices {
            let row = &mut data_db[slot * nchan..(slot + 1) * nchan];
            for (dst, &src) in row.iter_mut().zip(spectrum.power_linear.iter()) {
                *dst = 10.0 * (src + 1e-12f32).log10();
            }
        }
    }

    let data = ArcArray2::from_shape_vec((nslices, nchan), data_db)
        .context("Failed to shape data array")?;

    let spectrogram = Spectrogram {
        id: Uuid::new_v4(),
        start_time,
        nchan,
        nslices,
        freq: params.freq,
        bw: params.bw,
        slice_length: slice_length_s as f32,
        power_bounds: (FILL_DB, -FILL_DB), // not saved anyways
        data: data.into(),
    };

    spectrogram::save(&spectrogram, &args.output)
        .await
        .context("Failed to write output file")?;

    println!(
        "Wrote {} slices × {} channels to {}",
        nslices,
        nchan,
        args.output.display()
    );

    Ok(())
}
