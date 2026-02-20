// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    mem::MaybeUninit,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use chrono::{DateTime, Duration, Utc};
use futures_util::future::try_join_all;
use itertools::Itertools;
use ndarray::{ArcArray2, Array1, Array2, ArrayView2, Axis};
use ndarray_stats::QuantileExt;
use rustfft::{FftPlanner, num_complex::Complex};
use scirs2_signal::window::blackman;
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub start_time: DateTime<Utc>,
    pub freq: f32,   // Hz
    pub bw: f32,     // Hz
    pub length: f32, // s
    pub nchan: usize,
}
const HEADER_SIZE: usize = 256;

impl Header {
    pub fn same_params(&self, other: &Self) -> bool {
        self.freq == other.freq && self.bw == other.bw && self.nchan == other.nchan
    }

    pub fn nth_following(&self, nth: i32) -> DateTime<Utc> {
        self.start_time + chrono::Duration::milliseconds((self.length * 1000.0) as i64) * nth
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
                    && spectrogram.slice_length == first.slice_length
                    && spectrogram.nchan == first.nchan,
                "Inconsistent spectrogram parameters during concatenation"
            );
            let prev = &components[i - 1];
            ensure!(
                (spectrogram.start_time - prev.end_time())
                    .num_milliseconds()
                    .abs()
                    < 10,
                "Non-contiguous spectrograms during concatenation"
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

    let header = parse_header(&mut reader)
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

    while data_offset < uninit.len() {
        let new_header = parse_header(&mut reader).await?;
        ensure!(
            header.same_params(&new_header),
            "Inconsistent spectrogram parameters detected"
        );
        let expected_time = header.nth_following((data_offset / header.nchan) as i32);
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, VariantArray, Display)]
pub enum SampleFormat {
    CS8,
    CS16,
    CS32,
    CS64,
    CF32,
    CF64,
}

impl SampleFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "cs8" => Some(SampleFormat::CS8),
            "cs16" => Some(SampleFormat::CS16),
            "cs32" => Some(SampleFormat::CS32),
            "cs64" => Some(SampleFormat::CS64),
            "cf32" => Some(SampleFormat::CF32),
            "cf64" => Some(SampleFormat::CF64),
            _ => None,
        }
    }

    pub fn sample_size(&self) -> usize {
        match self {
            SampleFormat::CS8 => 2,
            SampleFormat::CS16 => 4,
            SampleFormat::CS32 | SampleFormat::CF32 => 8,
            SampleFormat::CS64 | SampleFormat::CF64 => 16,
        }
    }

    pub async fn read_sample<R: tokio::io::AsyncRead + Unpin>(
        &self,
        reader: &mut R,
    ) -> Result<f32> {
        match self {
            SampleFormat::CS8 => Ok(reader.read_i8().await? as f32 / -(i8::MIN as f32)),
            SampleFormat::CS16 => Ok(reader.read_i16_le().await? as f32 / -(i16::MIN as f32)),
            SampleFormat::CS32 => Ok(reader.read_i32_le().await? as f32 / -(i32::MIN as f32)),
            SampleFormat::CS64 => Ok(reader.read_i64_le().await? as f32 / -(i64::MIN as f32)),
            SampleFormat::CF32 => Ok(reader.read_f32_le().await? as f32),
            SampleFormat::CF64 => Ok(reader.read_f64_le().await? as f32),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IqFormat {
    pub samples: SampleFormat,
    pub sample_rate: f32,
}

pub async fn load_iq_file(
    path: &PathBuf,
    format: IqFormat,
    header: &Header,
) -> Result<Spectrogram> {
    let file = tokio::fs::File::open(path).await?;
    let file_size = file.metadata().await?.len() as usize;
    let n_samples = file_size / format.samples.sample_size();
    ensure!(
        n_samples % 2 == 0,
        "IQ file must contain an even number of samples"
    );

    let mut reader = tokio::io::BufReader::new(file);

    let mut samples: Vec<Complex<f32>> = Vec::with_capacity(n_samples);
    let uninit = samples.spare_capacity_mut();
    for value in uninit.iter_mut() {
        let i = format.samples.read_sample(&mut reader).await?;
        let q = format.samples.read_sample(&mut reader).await?;
        value.write(Complex::new(i, q));
    }
    // SAFETY: We have initialized all elements via uninit
    unsafe {
        samples.set_len(n_samples);
    }

    let shape = (samples.len() / header.nchan, header.nchan);
    let mut samples = Array2::from_shape_vec(shape, samples[..(shape.0 * shape.1)].to_vec())?;
    // TODO: Rayon?
    samples.mapv_inplace(|s| s * s);

    let n_samples_per_slice = (header.length * format.sample_rate) as usize;
    let n_windows = n_samples_per_slice / header.nchan;
    let window = Array1::from_iter(
        blackman(header.nchan, false)?
            .iter()
            .map(|&v| Complex::new(v as f32, 0.0)),
    );

    let fft = FftPlanner::new().plan_fft_forward(header.nchan);
    // TODO: Changing between ndarrays and Vecs so much seems inefficient
    // TODO: Rayon?
    let data = samples
        .outer_iter()
        .map(|slice| {
            let mut slice = slice.to_owned();
            // Remove DC offset
            let mean = slice.mean().unwrap_or(Complex::ZERO);
            slice -= mean;
            slice *= &window;
            fft.process(&mut slice.as_slice_mut().unwrap());
            slice.mapv(|s| 10.0 * (s.norm() + 1e-12).log10())
        })
        .chunks(n_windows)
        .into_iter()
        .map(|slice| {
            let avg = slice
                .into_iter()
                .fold(Array1::from_elem(header.nchan, 0.0), |acc, s| acc + s)
                / n_windows as f32;
            avg.to_vec()
        })
        .flatten()
        .collect_vec();

    Spectrogram::new(&header, data)
}
