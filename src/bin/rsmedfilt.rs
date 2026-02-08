use anyhow::Context;
use clap::Parser;
use rstrf::spectrogram;
use scirs2_ndimage::{BorderMode, filters::median_filter};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Spectrogram files to load (rffft format)
    #[arg(value_name = "INPUT", required = true)]
    input: Vec<PathBuf>,
    /// Spectrogram file to output (rffft format)
    #[arg(value_name = "OUTPUT", required = true)]
    output: PathBuf,
    /// Window size in Hz
    #[arg(short = 'w', long, value_name = "WINDOW_SIZE", default_value = "20000")]
    window_size: f32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut spectrogram = spectrogram::load(&args.input)
        .await
        .context("Failed to load input spectrogram")?;

    let window_size =
        (spectrogram.nchan as f32 * args.window_size / spectrogram.bw).round() as usize;

    let median = median_filter(
        &spectrogram.data().to_owned(),
        &[1, window_size],
        Some(BorderMode::Nearest),
    )
    .context("Failed to apply median filter")?;

    let result = &spectrogram.data() - &median;

    spectrogram.set_data(result.into())?;

    spectrogram::save(&spectrogram, &args.output)
        .await
        .context("Failed to save filtered spectrogram")?;

    Ok(())
}
