// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    mem::MaybeUninit,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use chrono::{DateTime, Duration, Utc};
use futures_util::future::try_join_all;
use ndarray::{ArcArray2, ArrayView2, Axis};
use ndarray_stats::QuantileExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::coord::data_absolute;

/// Loads a spectrogram from the given file paths
pub async fn load(paths: &[PathBuf]) -> Result<Spectrogram> {
    if paths.is_empty() {
        bail!("No files provided");
    }

    log::debug!("Parsing files {:?}", paths);
    let spectrograms = try_join_all(paths.iter().map(|path| async {
        load_file(path)
            .await
            .context(format!("Failed to load file {}", path.display()))
    }))
    .await?;

    Spectrogram::concatenate(&spectrograms)
}

/// Writes a spectrogram to the given file path
pub async fn save(spectrogram: &Spectrogram, path: &Path) -> Result<()> {
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

    Ok(())
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

impl Header {
    pub fn same_params(&self, other: &Self) -> bool {
        self.freq == other.freq && self.bw == other.bw && self.nchan == other.nchan
    }
}

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
    pub(self) fn new(first_header: &Header, raw_data: Vec<f32>) -> anyhow::Result<Self> {
        let nslices = raw_data.len() / first_header.nchan;
        let data = ArcArray2::from_shape_vec((nslices, first_header.nchan), raw_data)?
            .mapv(|v| 10.0 * (v + 1e-12).log10());
        let min = *data.min()?;
        let max = *data.max()?;
        Ok(Spectrogram {
            id: Uuid::new_v4(),
            start_time: first_header.start_time,
            freq: first_header.freq,
            bw: first_header.bw,
            slice_length: first_header.length,
            nchan: first_header.nchan,
            nslices,
            power_bounds: (min, max),
            data: data.into(),
        })
    }

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
                    < 500,
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

async fn load_file(path: &Path) -> Result<Spectrogram> {
    let file = tokio::fs::File::open(path).await?;
    let file_size = file.metadata().await?.len() as usize;
    let mut reader = tokio::io::BufReader::new(file);

    let mut header = parse_header(&mut reader)
        .await
        .context("Failed to parse header")?;
    log::debug!("Parsed header: {:?}", header);
    // File alternates between headers and data blocks of size nchan * 4 bytes (f32)
    let data_block_size = header.nchan * 4;
    let n_blocks = file_size / (data_block_size + HEADER_SIZE);

    let mut raw_data: Vec<f32> = Vec::with_capacity(n_blocks * header.nchan);
    let uninit = raw_data.spare_capacity_mut();
    let mut data_offset = 0usize;
    parse_data(
        &mut reader,
        &mut uninit[data_offset..data_offset + header.nchan],
    )
    .await?;
    data_offset += header.nchan;

    let mut prev_header = header.clone();

    let mut slices_length = header.length;

    while data_offset < uninit.len() {
        let new_header = parse_header(&mut reader).await?;
        ensure!(
            header.same_params(&new_header),
            "Inconsistent spectrogram parameters detected"
        );
        let expected_time =
            prev_header.start_time + Duration::milliseconds((prev_header.length * 1000.0) as i64);
        ensure!(
            // STRF sometimes has small differences in timestamps
            (new_header.start_time - expected_time)
                .num_milliseconds()
                .abs()
                < 10,
            "Unexpected spectrogram slice time: expected {}, got {}",
            expected_time,
            new_header.start_time
        );
        parse_data(
            &mut reader,
            &mut uninit[data_offset..data_offset + header.nchan],
        )
        .await?;
        data_offset += header.nchan;
        slices_length += new_header.length;
        prev_header = new_header;
    }

    ensure!(
        data_offset == uninit.len(),
        "Data size mismatch: expected {}, got {}",
        uninit.len(),
        data_offset
    );

    // SAFETY: We have initialized all elements via uninit
    unsafe {
        raw_data.set_len(n_blocks * header.nchan);
    }

    let min_max = raw_data
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
            (min.min(val), max.max(val))
        });
    log::debug!(
        "Loaded spectrogram with {} slices, min: {}, max: {}",
        raw_data.len() / header.nchan,
        min_max.0,
        min_max.1
    );

    header.length = slices_length / (raw_data.len() as f32 / header.nchan as f32);

    Spectrogram::new(&header, raw_data)
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

async fn parse_data<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    data: &mut [MaybeUninit<f32>],
) -> Result<()> {
    for value in data.iter_mut() {
        value.write(reader.read_f32_le().await?);
    }

    Ok(())
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
        let header = Header {
            start_time: start,
            freq: 437e6,
            bw: 100e3,
            length: 1.0,
            nchan,
        };
        let raw_data = vec![data; nslices * nchan];
        Spectrogram::new(&header, raw_data).unwrap()
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
}
