// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use chrono::{DateTime, Duration, Utc};
use futures_util::future::try_join_all;
use ndarray::{ArcArray2, ArrayView2, Axis};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::coord::data_absolute;

const RSTRF_MAGIC: &[u8; 8] = b"RSTRF\x01\n\0";
/// Sentinel dB value written to gap slots in `.rstrf` files.
pub const FILL_DB: f32 = -120.0;

/// Raw spectrum read from a strf `.bin` file, including its per-spectrum timestamp.
pub struct RawStrfSpectrum {
    pub time: DateTime<Utc>,
    /// Linear (not dB) power values, one per frequency channel.
    pub power_linear: Vec<f32>,
}

/// Parameters shared by all spectra in a strf `.bin` file.
pub struct StrfParams {
    pub freq: f32,
    pub bw: f32,
    pub nchan: usize,
}

/// Loads a spectrogram from the given file paths.
///
/// Files ending in `.rstrf` are loaded as the constant-rate RSTRF format.
/// All other files are treated as strf `.bin` files and resampled together
/// onto a uniform time grid (gaps filled with [`FILL_DB`]).
pub async fn load(paths: &[PathBuf]) -> Result<Spectrogram> {
    if paths.is_empty() {
        bail!("No files provided");
    }

    log::debug!("Parsing files {:?}", paths);

    let (bin_paths, rstrf_paths): (Vec<_>, Vec<_>) = paths
        .iter()
        .partition(|p| p.extension().and_then(|e| e.to_str()) != Some("rstrf"));

    let mut spectrograms: Vec<Spectrogram> = Vec::new();

    if !bin_paths.is_empty() {
        spectrograms.push(
            load_and_resample_strf(&bin_paths)
                .await
                .context("Failed to load .bin files")?,
        );
    }

    let rstrf_results = try_join_all(rstrf_paths.iter().map(|path| async {
        load_rstrf_file(path)
            .await
            .context(format!("Failed to load file {}", path.display()))
    }))
    .await?;
    spectrograms.extend(rstrf_results);

    spectrograms.sort_by_key(|s| s.start_time);
    Spectrogram::concatenate(&spectrograms)
}

/// Resamples raw strf spectra onto a uniform time grid.
///
/// The nominal slice length is determined as the median of inter-spectrum gaps,
/// or overridden via `slice_length_s`. Gaps are filled with [`FILL_DB`].
pub fn resample_strf(
    mut spectra: Vec<RawStrfSpectrum>,
    params: &StrfParams,
    slice_length_s: Option<f64>,
) -> Result<Spectrogram> {
    if spectra.is_empty() {
        bail!("No spectra found");
    }

    spectra.sort_unstable_by_key(|s| s.time);

    let nchan = params.nchan;

    let slice_length_s = match slice_length_s {
        Some(s) => {
            ensure!(s > 0.0, "Slice length must be positive");
            s
        }
        None => {
            let mut gaps: Vec<f64> = spectra
                .windows(2)
                .map(|w| {
                    (w[1].time - w[0].time).num_microseconds().unwrap_or(0) as f64 / 1_000_000.0
                })
                .collect();

            ensure!(
                !gaps.is_empty(),
                "Need at least two spectra to determine slice length; use --slice-length"
            );

            gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
            gaps[gaps.len() / 2]
        }
    };

    log::info!(
        "Resampling {} spectra onto {:.6} s grid ({} channels)",
        spectra.len(),
        slice_length_s,
        nchan
    );

    let start_time = spectra[0].time;
    let last_time = spectra.last().unwrap().time;

    let nslices = (((last_time - start_time).num_microseconds().unwrap_or(0) as f64
        / 1_000_000.0
        / slice_length_s)
        .round() as usize)
        + 1;

    let mut data_db = vec![FILL_DB; nslices * nchan];

    for spectrum in &spectra {
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

    let min = data
        .iter()
        .cloned()
        .filter(|&v| v > FILL_DB)
        .fold(f32::INFINITY, f32::min);
    let max = data
        .iter()
        .cloned()
        .filter(|&v| v > FILL_DB)
        .fold(f32::NEG_INFINITY, f32::max);

    Ok(Spectrogram {
        id: Uuid::new_v4(),
        start_time,
        nchan,
        nslices,
        freq: params.freq,
        bw: params.bw,
        slice_length: slice_length_s as f32,
        power_bounds: (min, max),
        data,
    })
}

async fn load_and_resample_strf(paths: &[&PathBuf]) -> Result<Spectrogram> {
    let mut all_spectra: Vec<RawStrfSpectrum> = Vec::new();
    let mut params: Option<StrfParams> = None;

    for path in paths {
        let (spectra, file_params) = load_strf_raw(path)
            .await
            .context(format!("Failed to load {}", path.display()))?;

        if let Some(ref p) = params {
            ensure!(
                p.freq == file_params.freq
                    && p.bw == file_params.bw
                    && p.nchan == file_params.nchan,
                "Inconsistent parameters between .bin files"
            );
        } else {
            params = Some(file_params);
        }

        all_spectra.extend(spectra);
    }

    let params = params.unwrap();
    resample_strf(all_spectra, &params, None)
}

/// Writes a spectrogram to the given file path in the RSTRF format.
pub async fn save(spectrogram: &Spectrogram, path: &Path) -> Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    let mut writer = tokio::io::BufWriter::new(&mut file);

    // Header (64 bytes total)
    writer.write_all(RSTRF_MAGIC).await?;
    writer
        .write_i64_le(spectrogram.start_time.timestamp_millis())
        .await?;
    writer.write_f64_le(spectrogram.freq as f64).await?;
    writer.write_f64_le(spectrogram.bw as f64).await?;
    writer.write_f64_le(spectrogram.slice_length as f64).await?;
    writer.write_u32_le(spectrogram.nchan as u32).await?;
    writer.write_u32_le(spectrogram.nslices as u32).await?;
    writer.write_all(&[0u8; 16]).await?; // reserved

    // Data: nslices * nchan f32 dB values, little-endian, row-major
    for &value in spectrogram.data().iter() {
        writer.write_f32_le(value).await?;
    }

    writer.flush().await?;
    Ok(())
}

/// Writes a spectrogram to the given file path in the strf `.bin` format.
///
/// Use this when you need the output to be compatible with `rfplot` or other
/// strf tools. For rstrf-internal use, prefer [`save`].
pub async fn save_strf(spectrogram: &Spectrogram, path: &Path) -> Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    let mut writer = tokio::io::BufWriter::new(&mut file);

    let header = |nslice: usize| {
        let mut start = (spectrogram.start_time
            + Duration::milliseconds((spectrogram.slice_length * 1000.0) as i64 * nslice as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
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
            spectrogram.slice_length,
            spectrogram.nchan,
            spectrogram.nslices
        );
        format!("{:256}", header)
    };

    for (i, slice) in spectrogram.data().outer_iter().enumerate() {
        writer.write_all(header(i).as_bytes()).await?;
        for &value in slice.iter() {
            let linear_value = 10f32.powf(value / 10.0);
            writer.write_f32_le(linear_value).await?;
        }
    }

    writer.flush().await?;
    Ok(())
}

/// Reads all spectra from a strf `.bin` file with their per-spectrum timestamps.
///
/// Unlike [`load_strf_file`], this function does not enforce timestamp regularity, making
/// it suitable for use by converters that need to handle gaps or jitter.
pub async fn load_strf_raw(path: &Path) -> Result<(Vec<RawStrfSpectrum>, StrfParams)> {
    let file = tokio::fs::File::open(path).await?;
    let file_size = file.metadata().await?.len() as usize;
    let mut reader = tokio::io::BufReader::new(file);

    let first_header = parse_header(&mut reader)
        .await
        .context("Failed to parse header")?;
    let params = StrfParams {
        freq: first_header.freq,
        bw: first_header.bw,
        nchan: first_header.nchan,
    };

    let data_block_size = first_header.nchan * 4;
    let n_blocks = file_size / (data_block_size + HEADER_SIZE);
    let mut spectra = Vec::with_capacity(n_blocks);

    let mut power = vec![0f32; first_header.nchan];
    for v in power.iter_mut() {
        *v = reader.read_f32_le().await?;
    }
    spectra.push(RawStrfSpectrum {
        time: first_header.start_time,
        power_linear: power,
    });

    while spectra.len() < n_blocks {
        let header = parse_header(&mut reader).await?;
        ensure!(
            params.freq == header.freq && params.bw == header.bw && params.nchan == header.nchan,
            "Inconsistent spectrogram parameters detected"
        );
        let mut power = vec![0f32; header.nchan];
        for v in power.iter_mut() {
            *v = reader.read_f32_le().await?;
        }
        spectra.push(RawStrfSpectrum {
            time: header.start_time,
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
    pub start_time: DateTime<Utc>,
    pub nchan: usize,
    pub nslices: usize,
    pub freq: f32,                // Hz
    pub bw: f32,                  // Hz
    pub slice_length: f32,        // s
    pub power_bounds: (f32, f32), // dB
    pub data: ArcArray2<f32>,     // dB
}

impl std::fmt::Debug for Spectrogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spectrogram")
            .field("start_time", &self.start_time)
            .field("freq", &self.freq)
            .field("bw", &self.bw)
            .field("slice_length", &self.slice_length)
            .field("nchan", &self.nchan)
            .field("nslices", &self.nslices)
            .field("power_bounds", &self.power_bounds)
            .finish()
    }
}

impl Spectrogram {
    pub fn concatenate(components: &[Spectrogram]) -> Result<Spectrogram> {
        if components.is_empty() {
            bail!("No spectrograms to concatenate");
        }

        let first = &components[0];
        for (i, spectrogram) in components.iter().enumerate().skip(1) {
            ensure!(
                spectrogram.freq == first.freq
                    && spectrogram.bw == first.bw
                    && (spectrogram.slice_length / first.slice_length - 1.0).abs() < 0.01
                    && spectrogram.nchan == first.nchan,
                "Inconsistent spectrogram parameters during concatenation: {:?} vs {:?}",
                spectrogram,
                first
            );
            let prev = &components[i - 1];
            ensure!(
                (spectrogram.start_time - prev.end_time())
                    .num_milliseconds()
                    .abs()
                    < 10,
                "Non-contiguous spectrograms during concatenation (expected {}, got {})",
                prev.end_time(),
                spectrogram.start_time
            );
        }

        let data = ndarray::concatenate(
            Axis(0),
            &components.iter().map(|s| s.data.view()).collect::<Vec<_>>(),
        )
        .context("Failed to concatenate spectrograms")?;

        let nslices: usize = components.iter().map(|s| s.nslices).sum();
        let power_bounds =
            components
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |bounds, spectrogram| {
                    (
                        bounds.0.min(spectrogram.power_bounds.0),
                        bounds.1.max(spectrogram.power_bounds.1),
                    )
                });

        Ok(Spectrogram {
            id: Uuid::new_v4(),
            start_time: first.start_time,
            freq: first.freq,
            bw: first.bw,
            slice_length: first.slice_length,
            nchan: first.nchan,
            nslices,
            power_bounds,
            data: data.into(),
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
        Duration::milliseconds((self.slice_length * 1000.0) as i64 * self.nslices as i64)
    }

    pub fn end_time(&self) -> DateTime<Utc> {
        self.start_time + self.length()
    }

    pub fn bounds(&self) -> data_absolute::Rectangle {
        data_absolute::Rectangle::new(
            data_absolute::Point::new(0.0, -self.bw / 2.0),
            data_absolute::Size::new(self.length().as_seconds_f32(), self.bw),
        )
    }
}

async fn load_rstrf_file(path: &Path) -> Result<Spectrogram> {
    let file = tokio::fs::File::open(path).await?;
    let mut reader = tokio::io::BufReader::new(file);

    let mut magic = [0u8; 8];
    reader
        .read_exact(&mut magic)
        .await
        .context("Failed to read magic bytes")?;
    ensure!(&magic == RSTRF_MAGIC, "Not an RSTRF file (bad magic bytes)");

    let start_time_ms = reader.read_i64_le().await?;
    let freq = reader.read_f64_le().await? as f32;
    let bw = reader.read_f64_le().await? as f32;
    let slice_length = reader.read_f64_le().await? as f32;
    let nchan = reader.read_u32_le().await? as usize;
    let nslices = reader.read_u32_le().await? as usize;
    // reserved
    let mut _reserved = [0u8; 16];
    reader.read_exact(&mut _reserved).await?;

    let start_time = DateTime::from_timestamp_millis(start_time_ms)
        .ok_or_else(|| anyhow!("Invalid start timestamp: {}", start_time_ms))?;

    let mut data_db = vec![0f32; nslices * nchan];
    for v in data_db.iter_mut() {
        *v = reader.read_f32_le().await?;
    }

    let data = ArcArray2::from_shape_vec((nslices, nchan), data_db)?;
    let min = data
        .iter()
        .cloned()
        .filter(|&v| v > FILL_DB)
        .fold(f32::INFINITY, f32::min);
    let max = data
        .iter()
        .cloned()
        .filter(|&v| v > FILL_DB)
        .fold(f32::NEG_INFINITY, f32::max);

    Ok(Spectrogram {
        id: Uuid::new_v4(),
        start_time,
        nchan,
        nslices,
        freq,
        bw,
        slice_length,
        power_bounds: (min, max),
        data,
    })
}

async fn parse_header<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Header> {
    let mut buf = [0u8; HEADER_SIZE];
    reader
        .read_exact(&mut buf)
        .await
        .context("Failed to read header")?;

    let text = std::str::from_utf8(&buf)?.trim_end_matches('\0').trim();

    let re = regex::Regex::new(
        r"(?s)HEADER\s+UTC_START\s+(\S+)\s+FREQ\s+([0-9.]+)\s+Hz\s+BW\s+([0-9.]+)\s+Hz\s+LENGTH\s+([0-9.]+)\s+s\s+NCHAN\s+(\d+)\s+(?:NSUB\s+\d+\s+)?END",
    )?;

    let caps = re
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
            start_time: start,
            freq: 437e6,
            bw: 100e3,
            slice_length: 1.0,
            nchan,
            nslices,
            power_bounds: (min, max),
            data: data.into(),
        }
    }

    fn make_raw_spectra(
        start: DateTime<Utc>,
        n: usize,
        nchan: usize,
        interval_ms: i64,
        power_linear: f32,
    ) -> (Vec<RawStrfSpectrum>, StrfParams) {
        let spectra = (0..n)
            .map(|i| RawStrfSpectrum {
                time: start + Duration::milliseconds(interval_ms * i as i64),
                power_linear: vec![power_linear; nchan],
            })
            .collect();
        let params = StrfParams {
            freq: 437e6,
            bw: 100e3,
            nchan,
        };
        (spectra, params)
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
        assert!(Spectrogram::concatenate(&[]).is_err());
    }

    #[test]
    fn concatenate_single_preserves_metadata() {
        let start = test_start();
        let spec = make_spec(start, 10, 1024, 1.0);
        let result = Spectrogram::concatenate(&[spec]).unwrap();
        assert_eq!(result.nslices, 10);
        assert_eq!(result.start_time, start);
        assert_eq!(result.nchan, 1024);
    }

    #[test]
    fn concatenate_two_sums_slices() {
        let start = test_start();
        let s1 = make_spec(start, 10, 1024, 1.0);
        let s2 = make_spec(s1.end_time(), 5, 1024, 2.0);
        let result = Spectrogram::concatenate(&[s1, s2]).unwrap();
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
        assert!(Spectrogram::concatenate(&[s1, s2]).is_err());
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

    #[test]
    fn resample_strf_empty_errors() {
        let params = StrfParams {
            freq: 437e6,
            bw: 100e3,
            nchan: 4,
        };
        assert!(resample_strf(vec![], &params, None).is_err());
    }

    #[test]
    fn resample_strf_single_spectrum_without_override_errors() {
        let (spectra, params) = make_raw_spectra(test_start(), 1, 4, 1000, 1.0);
        assert!(resample_strf(spectra, &params, None).is_err());
    }

    #[test]
    fn resample_strf_single_spectrum_with_override() {
        let start = test_start();
        let (spectra, params) = make_raw_spectra(start, 1, 4, 1000, 1.0);
        let result = resample_strf(spectra, &params, Some(1.0)).unwrap();
        assert_eq!(result.nslices, 1);
        assert_eq!(result.nchan, 4);
        assert_eq!(result.start_time, start);
    }

    #[test]
    fn resample_strf_slice_length_from_median_gap() {
        // 5 spectra at 1 s intervals → median gap = 1.0 s
        let (spectra, params) = make_raw_spectra(test_start(), 5, 4, 1000, 1.0);
        let result = resample_strf(spectra, &params, None).unwrap();
        assert!((result.slice_length - 1.0).abs() < 1e-3);
        assert_eq!(result.nslices, 5);
    }

    #[test]
    fn resample_strf_start_time_is_earliest_spectrum() {
        let start = test_start();
        let params = StrfParams {
            freq: 437e6,
            bw: 100e3,
            nchan: 4,
        };
        // Provide out-of-order spectra; start_time should be the earliest
        let spectra = vec![
            RawStrfSpectrum {
                time: start + Duration::seconds(2),
                power_linear: vec![1.0; 4],
            },
            RawStrfSpectrum {
                time: start,
                power_linear: vec![1.0; 4],
            },
            RawStrfSpectrum {
                time: start + Duration::seconds(1),
                power_linear: vec![1.0; 4],
            },
        ];
        let result = resample_strf(spectra, &params, Some(1.0)).unwrap();
        assert_eq!(result.start_time, start);
    }

    #[test]
    fn resample_strf_out_of_order_input_sorted_correctly() {
        let start = test_start();
        let params = StrfParams {
            freq: 437e6,
            bw: 100e3,
            nchan: 4,
        };
        let spectra = vec![
            RawStrfSpectrum {
                time: start + Duration::seconds(2),
                power_linear: vec![3.0; 4],
            },
            RawStrfSpectrum {
                time: start,
                power_linear: vec![1.0; 4],
            },
            RawStrfSpectrum {
                time: start + Duration::seconds(1),
                power_linear: vec![2.0; 4],
            },
        ];
        let result = resample_strf(spectra, &params, Some(1.0)).unwrap();
        assert_eq!(result.nslices, 3);

        let db = |x: f32| 10.0 * (x + 1e-12f32).log10();
        let data = result.data();
        for &v in data.slice(s![0, ..]).iter() {
            assert!((v - db(1.0)).abs() < 1e-4);
        }
        for &v in data.slice(s![1, ..]).iter() {
            assert!((v - db(2.0)).abs() < 1e-4);
        }
        for &v in data.slice(s![2, ..]).iter() {
            assert!((v - db(3.0)).abs() < 1e-4);
        }
    }

    #[test]
    fn resample_strf_data_converted_to_db() {
        let linear = 100.0f32;
        let expected_db = 10.0 * (linear + 1e-12f32).log10();
        let (spectra, params) = make_raw_spectra(test_start(), 3, 4, 1000, linear);
        let result = resample_strf(spectra, &params, None).unwrap();
        for &v in result.data().iter() {
            assert!(
                (v - expected_db).abs() < 1e-4,
                "expected {expected_db}, got {v}"
            );
        }
    }

    #[test]
    fn resample_strf_gap_filled_with_fill_db() {
        let start = test_start();
        let params = StrfParams {
            freq: 437e6,
            bw: 100e3,
            nchan: 4,
        };
        // Spectra at t+0, t+1, t+3 — slot 2 (t+2) is missing
        let spectra = vec![
            RawStrfSpectrum {
                time: start,
                power_linear: vec![1.0; 4],
            },
            RawStrfSpectrum {
                time: start + Duration::seconds(1),
                power_linear: vec![1.0; 4],
            },
            RawStrfSpectrum {
                time: start + Duration::seconds(3),
                power_linear: vec![1.0; 4],
            },
        ];
        let result = resample_strf(spectra, &params, Some(1.0)).unwrap();
        assert_eq!(result.nslices, 4);
        let data = result.data();
        for &v in data.slice(s![2, ..]).iter() {
            assert_eq!(v, FILL_DB, "gap slot should be FILL_DB");
        }
        // Non-gap slots should have real data
        for row in [0, 1, 3] {
            assert!(data.slice(s![row, ..]).iter().all(|&v| v > FILL_DB));
        }
    }

    #[test]
    fn resample_strf_power_bounds_exclude_fill_db() {
        let start = test_start();
        let params = StrfParams {
            freq: 437e6,
            bw: 100e3,
            nchan: 4,
        };
        // Slot 1 will be a gap (FILL_DB); slots 0 and 2 have real data
        let spectra = vec![
            RawStrfSpectrum {
                time: start,
                power_linear: vec![1.0; 4],
            },
            RawStrfSpectrum {
                time: start + Duration::seconds(2),
                power_linear: vec![100.0; 4],
            },
        ];
        let result = resample_strf(spectra, &params, Some(1.0)).unwrap();
        let (min, max) = result.power_bounds;
        assert!(min > FILL_DB, "min should exclude FILL_DB sentinel");
        assert!(max > min);
    }

    #[test]
    fn resample_strf_override_slice_length_used() {
        // 3 spectra at 1 s intervals; override to 0.5 s → 5 slots spanning 0..2 s
        let (spectra, params) = make_raw_spectra(test_start(), 3, 4, 1000, 1.0);
        let result = resample_strf(spectra, &params, Some(0.5)).unwrap();
        assert!((result.slice_length - 0.5).abs() < 1e-6);
        assert_eq!(result.nslices, 5);
    }

    #[test]
    fn resample_strf_zero_slice_length_errors() {
        let (spectra, params) = make_raw_spectra(test_start(), 3, 4, 1000, 1.0);
        assert!(resample_strf(spectra, &params, Some(0.0)).is_err());
    }

    #[test]
    fn resample_strf_negative_slice_length_errors() {
        let (spectra, params) = make_raw_spectra(test_start(), 3, 4, 1000, 1.0);
        assert!(resample_strf(spectra, &params, Some(-1.0)).is_err());
    }

    #[tokio::test]
    async fn load_single_bin_file_roundtrip() {
        let start = test_start();
        let spec = make_spec(start, 10, 16, 100.0);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        save_strf(&spec, &path).await.unwrap();

        let loaded = load(&[path]).await.unwrap();

        assert_eq!(loaded.nslices, spec.nslices);
        assert_eq!(loaded.nchan, spec.nchan);
        assert_eq!(loaded.start_time, spec.start_time);
        assert!((loaded.freq - spec.freq).abs() < 1.0);
        assert!((loaded.bw - spec.bw).abs() < 1.0);
        assert!((loaded.slice_length - spec.slice_length).abs() < 0.01);

        let orig = spec.data();
        let got = loaded.data();
        for (&a, &b) in orig.iter().zip(got.iter()) {
            // save_strf converts dB→linear; resample_strf converts back, adding a second
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

        let loaded = load(&[path1, path2]).await.unwrap();

        assert_eq!(loaded.nslices, s1.nslices + s2.nslices);
        assert_eq!(loaded.nchan, s1.nchan);
        assert_eq!(loaded.start_time, start);
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
        let loaded = load(&[path2.clone(), path1.clone()]).await.unwrap();
        let loaded_fwd = load(&[path1, path2]).await.unwrap();

        assert_eq!(loaded.nslices, loaded_fwd.nslices);
        assert_eq!(loaded.start_time, loaded_fwd.start_time);
    }

    #[tokio::test]
    async fn load_bin_and_rstrf_concatenated() {
        let start = test_start();
        let s1 = make_spec(start, 5, 16, 100.0);
        let s2 = make_spec(s1.end_time(), 5, 16, 200.0);

        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("part1.bin");
        let rstrf_path = dir.path().join("part2.rstrf");
        save_strf(&s1, &bin_path).await.unwrap();
        save(&s2, &rstrf_path).await.unwrap();

        let loaded = load(&[bin_path, rstrf_path]).await.unwrap();

        assert_eq!(loaded.nslices, s1.nslices + s2.nslices);
        assert_eq!(loaded.start_time, start);
    }

    #[tokio::test]
    async fn save_load_rstrf_roundtrip() {
        let start = test_start();
        let spec = make_spec(start, 10, 64, 1.5);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rstrf");

        save(&spec, &path).await.unwrap();
        let loaded = load(&[path]).await.unwrap();

        assert_eq!(loaded.nslices, spec.nslices);
        assert_eq!(loaded.nchan, spec.nchan);
        assert_eq!(loaded.start_time, spec.start_time);
        assert!((loaded.freq - spec.freq).abs() < 1.0);
        assert!((loaded.bw - spec.bw).abs() < 1.0);
        assert!((loaded.slice_length - spec.slice_length).abs() < 1e-4);

        let orig = spec.data();
        let got = loaded.data();
        for (&a, &b) in orig.iter().zip(got.iter()) {
            assert!((a - b).abs() < 1e-4, "dB mismatch: {} vs {}", a, b);
        }
    }
}
