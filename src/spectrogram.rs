// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use chrono::{DateTime, Duration, Utc};
use futures_util::{StreamExt, TryStreamExt};
use ndarray::{ArcArray2, ArrayView2};
use rayon::prelude::*;
use regex::Regex;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

use crate::coord::data_absolute;

static HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)HEADER\s+UTC_START\s+(\S+)\s+FREQ\s+([0-9.]+)\s+Hz\s+BW\s+([0-9.]+)\s+Hz\s+LENGTH\s+([0-9.]+)\s+s\s+NCHAN\s+(\d+)\s+(?:NSUB\s+\d+\s+)?END").unwrap()
});

/// Raw spectrum read from a strf `.bin` file, including its per-spectrum timestamp.
pub struct RawStrfSpectrum {
    pub time: DateTime<Utc>,
    pub length_s: f32,
    /// Linear (not dB) power values, one per frequency channel.
    pub power_linear: Vec<f32>,
}

/// Parameters shared by all spectra in a strf `.bin` file.
#[derive(PartialEq, Debug)]
pub struct SpectrogramParams {
    pub freq: f32,
    pub bw: f32,
    pub nchan: usize,
}

/// Loads a single spectrogram file, dispatching on extension.
///
/// This is simply a wrapper around `load_strf_file`, but we might add a different format in the
/// future.
pub async fn load_single(path: PathBuf, freq_range: Option<(u64, u64)>) -> Result<Spectrogram> {
    let spec = load_strf_file(&path, freq_range).await;
    log::debug!("Loaded {}", path.display());
    spec.context(format!("Failed to load file {:?}", path))
}

/// Loads a spectrogram from the given file paths.
pub async fn load(paths: &[PathBuf], freq_range: Option<(u64, u64)>) -> Result<Spectrogram> {
    if paths.is_empty() {
        bail!("No files provided");
    }

    log::debug!("Loading {} spectrogram files", paths.len());

    let mut spectrograms: Vec<_> = futures_util::stream::iter(paths.iter().cloned())
        .map(|path| load_single(path, freq_range))
        .buffer_unordered(8)
        .try_collect()
        .await?;

    log::debug!("Joining {} spectrograms", spectrograms.len());
    spectrograms.sort_by_key(|s| s.start_time());
    Spectrogram::concatenate(spectrograms)
}

async fn load_strf_file(path: &Path, freq_range: Option<(u64, u64)>) -> Result<Spectrogram> {
    let (mut spectra, params) = load_strf_raw(path, freq_range).await?;
    spectra.sort_unstable_by_key(|spec| spec.time);

    let nchan = params.nchan;
    let (data, timestamps, lengths) = tokio::task::spawn_blocking(move || {
        let nslices = spectra.len();
        let timestamps: Vec<_> = spectra.iter().map(|s| s.time).collect();
        let lengths: Vec<_> = spectra.iter().map(|s| s.length_s).collect();

        let mut data = Vec::with_capacity(nslices * nchan);
        for spec in spectra {
            data.extend(spec.power_linear);
        }
        data.par_iter_mut()
            .for_each(|v| *v = 10.0 * (*v + 1e-12f32).log10());

        (data, timestamps, lengths)
    })
    .await?;
    let data = ArcArray2::from_shape_vec((timestamps.len(), params.nchan), data)
        .context("Failed to shape data array")?;
    Ok(Spectrogram {
        id: Uuid::new_v4(),
        nchan: params.nchan,
        nslices: timestamps.len(),
        freq: params.freq,
        bw: params.bw,
        power_bounds: (
            data.iter().cloned().fold(f32::INFINITY, f32::min),
            data.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
        ),
        data,
        timestamps,
        lengths,
    })
}

/// Writes a spectrogram to the given file path in the strf `.bin` format.
pub async fn save_strf(spectrogram: &Spectrogram, path: &Path) -> Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    let mut writer = tokio::io::BufWriter::new(&mut file);

    let header = |start: DateTime<Utc>, length_s: f32| {
        let mut start = start.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        // Remove trailing Z for compatibility with STRF
        start.pop();

        let header = format!(
            r#"HEADER
UTC_START    {}
FREQ         {} Hz
BW           {} Hz
LENGTH       {} s
NCHAN        {}
NSUB         {}
END
"#,
            start,
            spectrogram.freq,
            spectrogram.bw,
            length_s,
            spectrogram.nchan,
            spectrogram.nslices
        );
        format!("{:256}", header)
    };

    for (i, slice) in spectrogram.data().outer_iter().enumerate() {
        writer
            .write_all(header(spectrogram.timestamps[i], spectrogram.lengths[i]).as_bytes())
            .await?;
        for &value in slice.iter() {
            let linear_value = 10f32.powf(value / 10.0);
            writer.write_f32_le(linear_value).await?;
        }
    }

    writer.flush().await?;
    Ok(())
}

/// Applies a frequency range filter to the spectrogram parameters.
///
/// Returns
/// - the updated parameters
/// - how many bytes to skip before reading each spectrum
/// - how many bytes to skip after reading each spectrum
fn apply_freq_range(
    header: &Header,
    freq_range: Option<(u64, u64)>,
) -> (SpectrogramParams, usize, usize) {
    let Some((min_freq, max_freq)) = freq_range else {
        return (
            SpectrogramParams {
                freq: header.freq,
                bw: header.bw,
                nchan: header.nchan,
            },
            0,
            0,
        );
    };
    let chan_width = header.bw / header.nchan as f32;
    let start_freq = header.freq - header.bw / 2.0;
    let range_start = (((min_freq as f32 - start_freq).clamp(0.0, header.bw) / chan_width).floor()
        as usize)
        .min(header.nchan);
    let range_end = (((max_freq as f32 - start_freq).clamp(0.0, header.bw) / chan_width).ceil()
        as usize)
        .min(header.nchan);
    let nchan = range_end.saturating_sub(range_start);
    let skip_before = range_start * 4;
    let skip_after = (header.nchan - range_end) * 4;
    (
        SpectrogramParams {
            freq: start_freq + (range_start as f32 + nchan as f32 / 2.0) * chan_width,
            bw: nchan as f32 * chan_width,
            nchan,
        },
        skip_before,
        skip_after,
    )
}

async fn read_spectrum<F>(
    reader: &mut BufReader<F>,
    byte_len: usize,
    skip_before: usize,
    skip_after: usize,
) -> Result<Vec<f32>>
where
    F: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
{
    reader.seek(SeekFrom::Current(skip_before as i64)).await?;
    let mut buf = vec![0u8; byte_len];
    reader.read_exact(&mut buf).await?;
    reader.seek(SeekFrom::Current(skip_after as i64)).await?;
    let power: Vec<f32> = bytemuck::cast_slice(&buf).to_vec();
    Ok(power)
}

/// Reads all spectra from a strf `.bin` file with their per-spectrum timestamps.
pub async fn load_strf_raw(
    path: &Path,
    freq_range: Option<(u64, u64)>,
) -> Result<(Vec<RawStrfSpectrum>, SpectrogramParams)> {
    let file = tokio::fs::File::open(path).await?;
    let file_size = file.metadata().await?.len() as usize;
    let mut reader = tokio::io::BufReader::new(file);

    let first_header = parse_header(&mut reader)
        .await
        .context("Failed to parse header")?;
    let (params, skip_before, skip_after) = apply_freq_range(&first_header, freq_range);
    ensure!(
        params.nchan > 0,
        "Frequency range filter excludes all channels: {:?}",
        params
    );
    log::debug!(
        "Reading {}/{} channels per spectrum",
        params.nchan,
        first_header.nchan
    );

    let data_block_size = first_header.nchan * 4;
    let n_blocks = file_size / (data_block_size + HEADER_SIZE);
    let mut spectra = Vec::with_capacity(n_blocks);

    let byte_len = params.nchan * 4;
    let power = read_spectrum(&mut reader, byte_len, skip_before, skip_after)
        .await
        .context("Failed to read first spectrum")?;
    spectra.push(RawStrfSpectrum {
        time: first_header.start_time,
        length_s: first_header.length,
        power_linear: power,
    });

    while spectra.len() < n_blocks {
        let header = parse_header(&mut reader).await?;
        ensure!(
            first_header.freq == header.freq
                && first_header.bw == header.bw
                && first_header.nchan == header.nchan,
            "Inconsistent spectrogram parameters detected"
        );
        let power = read_spectrum(&mut reader, byte_len, skip_before, skip_after)
            .await
            .context("Failed to read spectrum")?;
        spectra.push(RawStrfSpectrum {
            time: header.start_time,
            length_s: header.length,
            power_linear: power,
        });
    }

    Ok((spectra, params))
}

#[derive(Debug, Clone, PartialEq)]
struct Header {
    start_time: DateTime<Utc>,
    freq: f32,   // Hz
    bw: f32,     // Hz
    length: f32, // s
    nchan: usize,
}
const HEADER_SIZE: usize = 256;

#[derive(Clone, PartialEq)]
pub struct Spectrogram {
    pub id: Uuid,
    pub nchan: usize,
    pub nslices: usize,
    pub freq: f32,                // Hz
    pub bw: f32,                  // Hz
    pub power_bounds: (f32, f32), // dB
    pub data: ArcArray2<f32>,     // dB
    // TODO: Replace with ArcArray1?
    pub timestamps: Vec<DateTime<Utc>>,
    pub lengths: Vec<f32>,
}

impl std::fmt::Debug for Spectrogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spectrogram")
            .field("start_time", &self.start_time())
            .field("freq", &self.freq)
            .field("bw", &self.bw)
            .field("slice_length", &self.lengths[0])
            .field("nchan", &self.nchan)
            .field("nslices", &self.nslices)
            .field("power_bounds", &self.power_bounds)
            .finish()
    }
}

impl Spectrogram {
    pub fn concatenate(components: Vec<Spectrogram>) -> Result<Spectrogram> {
        if components.is_empty() {
            bail!("No spectrograms to concatenate");
        }

        let first = &components[0];
        for spectrogram in components.iter().skip(1) {
            ensure!(
                spectrogram.params() == first.params(),
                "Inconsistent spectrogram parameters during concatenation: {:?} vs {:?}",
                spectrogram,
                first
            );
        }

        let nslices: usize = components.iter().map(|s| s.nslices).sum();
        let nchan = first.nchan;
        let freq = first.freq;
        let bw = first.bw;

        let mut data_flat = Vec::with_capacity(nslices * nchan);
        let mut timestamps = Vec::with_capacity(nslices);
        let mut lengths = Vec::with_capacity(nslices);
        let mut power_bounds = (f32::INFINITY, f32::NEG_INFINITY);

        for spec in components {
            data_flat.extend_from_slice(spec.data.as_slice().unwrap());
            timestamps.extend(spec.timestamps);
            lengths.extend(spec.lengths);
            power_bounds.0 = power_bounds.0.min(spec.power_bounds.0);
            power_bounds.1 = power_bounds.1.max(spec.power_bounds.1);
        }

        let data = ArcArray2::from_shape_vec((nslices, nchan), data_flat)
            .context("Failed to concatenate spectrograms")?;

        Ok(Spectrogram {
            id: Uuid::new_v4(),
            freq,
            bw,
            nchan,
            nslices,
            power_bounds,
            data,
            timestamps,
            lengths,
        })
    }

    pub fn data(&self) -> ArrayView2<'_, f32> {
        self.data.view()
    }

    pub fn set_data(&mut self, data: ArcArray2<f32>) -> anyhow::Result<()> {
        ensure!(
            data.dim() == (self.nslices, self.nchan),
            "Data shape mismatch: expected ({}, {}), got ({}, {})",
            self.nslices,
            self.nchan,
            data.dim().0,
            data.dim().1
        );

        self.data = data;
        Ok(())
    }

    pub fn length(&self) -> Duration {
        self.end_time() - self.start_time()
    }

    pub fn start_time(&self) -> DateTime<Utc> {
        self.timestamps[0]
    }

    pub fn end_time(&self) -> DateTime<Utc> {
        let last = self.timestamps.len() - 1;
        self.timestamps[last] + Duration::milliseconds((self.lengths[last] * 1000.0) as i64)
    }

    pub fn bounds(&self) -> data_absolute::Rectangle {
        data_absolute::Rectangle::new(
            data_absolute::Point::new(0.0, -self.bw / 2.0),
            data_absolute::Size::new(self.length().as_seconds_f32(), self.bw),
        )
    }

    // TODO: `bounds()`, which returns `data_absolute`, isn't absolute...
    pub fn absolute_bounds(&self) -> SpectrogramBounds {
        SpectrogramBounds {
            time_range: self.start_time()..self.end_time(),
            freq_range: (self.freq - self.bw / 2.0)..(self.freq + self.bw / 2.0),
        }
    }

    pub fn params(&self) -> SpectrogramParams {
        SpectrogramParams {
            freq: self.freq,
            bw: self.bw,
            nchan: self.nchan,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpectrogramBounds {
    pub time_range: std::ops::Range<DateTime<Utc>>,
    pub freq_range: std::ops::Range<f32>,
}

async fn parse_header<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Header> {
    let mut buf = [0u8; HEADER_SIZE];
    reader
        .read_exact(&mut buf)
        .await
        .context("Failed to read header")?;

    let text = std::str::from_utf8(&buf)?.trim_end_matches('\0').trim();

    let caps = HEADER_RE
        .captures(text)
        .ok_or_else(|| anyhow!("Incorrect header format"))?;

    Ok(Header {
        start_time: DateTime::parse_from_rfc3339(format!("{}Z", &caps[1]).as_str())
            .context(format!("Invalid start_time: {}", &caps[1]))?
            .with_timezone(&Utc),
        freq: caps[2]
            .parse()
            .context(format!("Invalid freq: {}", &caps[2]))?,
        bw: caps[3]
            .parse()
            .context(format!("Invalid bw: {}", &caps[3]))?,
        length: caps[4]
            .parse()
            .context(format!("Invalid length: {}", &caps[4]))?,
        nchan: caps[5]
            .parse()
            .context(format!("Invalid nchan: {}", &caps[5]))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use ndarray::s;

    fn test_start() -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
    }

    fn make_spec(start: DateTime<Utc>, nslices: usize, nchan: usize, data: f32) -> Spectrogram {
        let raw_data = vec![data; nslices * nchan];
        let data = ArcArray2::from_shape_vec((nslices, nchan), raw_data)
            .unwrap()
            .mapv(|v| 10.0 * (v + 1e-12).log10());
        let min = data.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = data.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        Spectrogram {
            id: Uuid::new_v4(),
            freq: 437e6,
            bw: 100e3,
            nchan,
            nslices,
            power_bounds: (min, max),
            data: data.into(),
            timestamps: (0..nslices)
                .map(|i| start + Duration::milliseconds((1.0 * 1000.0) as i64 * i as i64))
                .collect(),
            lengths: vec![1.0; nslices],
        }
    }

    #[test]
    fn length_equals_nslices_times_slice_length() {
        let spec = make_spec(test_start(), 10, 1024, 1.0);
        assert_eq!(spec.length().num_seconds(), 10);
    }

    #[test]
    fn end_time_is_start_plus_length() {
        let start = test_start();
        let spec = make_spec(start, 5, 1024, 1.0);
        assert_eq!((spec.end_time() - start).num_seconds(), 5);
    }

    #[test]
    fn bounds_origin_and_size_match_params() {
        let spec = make_spec(test_start(), 10, 1024, 1.0);
        let b = spec.bounds();
        assert!((b.0.x - 0.0).abs() < 1e-3);
        assert!((b.0.y - (-50e3)).abs() < 1e-3);
        assert!((b.0.width - 10.0).abs() < 1e-3);
        assert!((b.0.height - 100e3).abs() < 1e-3);
    }

    #[test]
    fn concatenate_empty_errors() {
        assert!(Spectrogram::concatenate(vec![]).is_err());
    }

    #[test]
    fn concatenate_single_preserves_metadata() {
        let start = test_start();
        let spec = make_spec(start, 10, 1024, 1.0);
        let spec_params = spec.params();
        let result = Spectrogram::concatenate(vec![spec]).unwrap();
        assert_eq!(result.nslices, 10);
        assert_eq!(result.params(), spec_params);
    }

    #[test]
    fn concatenate_two_sums_slices() {
        let start = test_start();
        let s1 = make_spec(start, 10, 1024, 1.0);
        let s2 = make_spec(s1.end_time(), 5, 1024, 2.0);
        let result = Spectrogram::concatenate(vec![s1, s2]).unwrap();
        assert_eq!(result.nslices, 15);

        let data = result.data();
        let db1 = 10.0 * (1.0f32 + 1e-12).log10();
        let db2 = 10.0 * (2.0f32 + 1e-12).log10();
        assert!(
            data.slice(s![..10, ..])
                .iter()
                .all(|&v| (v - db1).abs() < 1e-3)
        );
        assert!(
            data.slice(s![10.., ..])
                .iter()
                .all(|&v| (v - db2).abs() < 1e-3)
        );
    }

    #[test]
    fn concatenate_mismatched_nchan_errors() {
        let start = test_start();
        let s1 = make_spec(start, 5, 1024, 1.0);
        let s2 = make_spec(s1.end_time(), 5, 512, 1.0);
        assert!(Spectrogram::concatenate(vec![s1, s2]).is_err());
    }

    #[test]
    fn set_data_rejects_wrong_shape() {
        let mut spec = make_spec(test_start(), 5, 1024, 1.0);
        assert!(spec.set_data(ArcArray2::zeros((3, 1024))).is_err());
    }

    #[test]
    fn set_data_accepts_correct_shape() {
        let mut spec = make_spec(test_start(), 5, 1024, 1.0);
        assert!(spec.set_data(ArcArray2::zeros((5, 1024))).is_ok());
    }

    #[tokio::test]
    async fn load_single_bin_file_roundtrip() {
        let start = test_start();
        let spec = make_spec(start, 10, 16, 100.0);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        save_strf(&spec, &path).await.unwrap();

        let loaded = load(&[path], None).await.unwrap();

        assert_eq!(loaded.nslices, spec.nslices);
        assert_eq!(loaded.nchan, spec.nchan);
        assert_eq!(loaded.start_time(), spec.start_time());
        assert!((loaded.freq - spec.freq).abs() < 1.0);
        assert!((loaded.bw - spec.bw).abs() < 1.0);

        let orig = spec.data();
        let got = loaded.data();
        for (&a, &b) in orig.iter().zip(got.iter()) {
            // save_strf converts dB→linear; load_strf converts back, adding a second
            // epsilon; the round-trip error is negligible for non-tiny power values.
            assert!((a - b).abs() < 0.01, "dB mismatch: {a} vs {b}");
        }
    }

    #[tokio::test]
    async fn load_multiple_bin_files_merged_into_one_spectrogram() {
        let start = test_start();
        let s1 = make_spec(start, 5, 16, 100.0);
        let s2 = make_spec(s1.end_time(), 5, 16, 200.0);

        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("part1.bin");
        let path2 = dir.path().join("part2.bin");
        save_strf(&s1, &path1).await.unwrap();
        save_strf(&s2, &path2).await.unwrap();

        let loaded = load(&[path1, path2], None).await.unwrap();

        assert_eq!(loaded.nslices, s1.nslices + s2.nslices);
        assert_eq!(loaded.nchan, s1.nchan);
        assert_eq!(loaded.start_time(), start);
    }

    #[tokio::test]
    async fn load_bin_files_out_of_cli_order_sorted_by_time() {
        let start = test_start();
        let s1 = make_spec(start, 5, 16, 100.0);
        let s2 = make_spec(s1.end_time(), 5, 16, 200.0);

        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("part1.bin");
        let path2 = dir.path().join("part2.bin");
        save_strf(&s1, &path1).await.unwrap();
        save_strf(&s2, &path2).await.unwrap();

        // Pass in reverse order — should produce the same result
        let loaded = load(&[path2.clone(), path1.clone()], None).await.unwrap();
        let loaded_fwd = load(&[path1, path2], None).await.unwrap();

        assert_eq!(loaded.nslices, loaded_fwd.nslices);
        assert_eq!(loaded.start_time(), loaded_fwd.start_time());
    }

    // Header for apply_freq_range unit tests: freq=500_000 Hz, bw=1_000 Hz, nchan=10
    // chan_width=100 Hz, channels span [499_500, 500_500) Hz
    fn test_header() -> Header {
        Header {
            start_time: test_start(),
            freq: 500_000.0,
            bw: 1_000.0,
            length: 1.0,
            nchan: 10,
        }
    }

    #[test]
    fn apply_freq_range_none_returns_full_params_and_no_skips() {
        let h = test_header();
        let (params, skip_before, skip_after) = apply_freq_range(&h, None);
        assert_eq!(params.nchan, 10);
        assert!((params.freq - 500_000.0).abs() < 0.1);
        assert!((params.bw - 1_000.0).abs() < 0.1);
        assert_eq!(skip_before, 0);
        assert_eq!(skip_after, 0);
    }

    #[test]
    fn apply_freq_range_full_span_returns_all_channels() {
        let h = test_header();
        let (params, skip_before, skip_after) = apply_freq_range(&h, Some((499_500, 500_500)));
        assert_eq!(params.nchan, 10);
        assert_eq!(skip_before, 0);
        assert_eq!(skip_after, 0);
    }

    #[test]
    fn apply_freq_range_lower_half_skips_after() {
        // [499_500, 500_000) → channels 0..5; 5 channels after skipped
        let h = test_header();
        let (params, skip_before, skip_after) = apply_freq_range(&h, Some((499_500, 500_000)));
        assert_eq!(params.nchan, 5);
        assert_eq!(skip_before, 0);
        assert_eq!(skip_after, 5 * 4);
        assert!((params.freq - 499_750.0).abs() < 1.0);
        assert!((params.bw - 500.0).abs() < 1.0);
    }

    #[test]
    fn apply_freq_range_upper_half_skips_before() {
        // [500_000, 500_500) → channels 5..10; 5 channels before skipped
        let h = test_header();
        let (params, skip_before, skip_after) = apply_freq_range(&h, Some((500_000, 500_500)));
        assert_eq!(params.nchan, 5);
        assert_eq!(skip_before, 5 * 4);
        assert_eq!(skip_after, 0);
        assert!((params.freq - 500_250.0).abs() < 1.0);
        assert!((params.bw - 500.0).abs() < 1.0);
    }

    #[test]
    fn apply_freq_range_middle_channels_skips_both_sides() {
        // [499_700, 500_200) → channels 2..7
        let h = test_header();
        let (params, skip_before, skip_after) = apply_freq_range(&h, Some((499_700, 500_200)));
        assert_eq!(params.nchan, 5);
        assert_eq!(skip_before, 2 * 4);
        assert_eq!(skip_after, 3 * 4);
        assert!((params.freq - 499_950.0).abs() < 1.0);
        assert!((params.bw - 500.0).abs() < 1.0);
    }

    #[test]
    fn apply_freq_range_wider_than_spectrum_clamps_to_all() {
        let h = test_header();
        let (params, skip_before, skip_after) = apply_freq_range(&h, Some((0, 1_000_000)));
        assert_eq!(params.nchan, 10);
        assert_eq!(skip_before, 0);
        assert_eq!(skip_after, 0);
    }

    #[test]
    fn apply_freq_range_entirely_below_spectrum_returns_zero_nchan() {
        let h = test_header();
        let (params, _, _) = apply_freq_range(&h, Some((400_000, 450_000)));
        assert_eq!(params.nchan, 0);
    }

    #[test]
    fn apply_freq_range_entirely_above_spectrum_returns_zero_nchan() {
        let h = test_header();
        let (params, _, _) = apply_freq_range(&h, Some((600_000, 700_000)));
        assert_eq!(params.nchan, 0);
    }

    #[tokio::test]
    async fn load_with_freq_range_reduces_nchan_and_updates_freq_bw() {
        // make_spec uses freq=437e6, bw=100e3, nchan=16
        // chan_width = 6250 Hz, start_freq = 436_950_000 Hz
        // Lower 8 channels: [436_950_000, 437_000_000)
        let start = test_start();
        let spec = make_spec(start, 5, 16, 100.0);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        save_strf(&spec, &path).await.unwrap();

        let loaded = load(&[path], Some((436_950_000, 437_000_000))).await.unwrap();

        assert_eq!(loaded.nchan, 8);
        assert!((loaded.bw - 50_000.0).abs() < 1.0);
        assert!((loaded.freq - 436_975_000.0).abs() < 1.0);
        assert_eq!(loaded.nslices, spec.nslices);
    }

    #[tokio::test]
    async fn load_with_freq_range_data_values_match_unfiltered_slice() {
        // Save a file and load it twice — once full, once with lower half only.
        // The loaded values for the lower channels should match.
        let start = test_start();
        let spec = make_spec(start, 3, 16, 100.0);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        save_strf(&spec, &path).await.unwrap();

        let full = load(&[path.clone()], None).await.unwrap();
        let filtered = load(&[path], Some((436_950_000, 437_000_000))).await.unwrap();

        let full_data = full.data();
        let filt_data = filtered.data();
        for row in 0..3 {
            for ch in 0..8 {
                let expected = full_data[[row, ch]];
                let got = filt_data[[row, ch]];
                assert!((expected - got).abs() < 0.01, "row={row} ch={ch}: {expected} vs {got}");
            }
        }
    }

    #[tokio::test]
    async fn load_with_freq_range_excluding_all_channels_errors() {
        let start = test_start();
        let spec = make_spec(start, 3, 16, 100.0);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        save_strf(&spec, &path).await.unwrap();

        // Range entirely outside the spectrum [436_950_000, 437_050_000)
        let result = load(&[path], Some((100_000, 200_000))).await;
        assert!(result.is_err());
    }
}
