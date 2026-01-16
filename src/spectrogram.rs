// SPDX-License-Identifier: MIT

use anyhow::{Context, Result, anyhow, bail, ensure};
use chrono::{DateTime, Duration, Utc};
use futures_util::future::try_join_all;
use ndarray::ArrayView2;
use tokio::io::AsyncReadExt;

/// Loads a spectrogram from the given file paths
pub async fn load(paths: &[std::path::PathBuf]) -> Result<Spectrogram> {
    if paths.is_empty() {
        bail!("No files provided");
    }

    println!("Parsing files {:?}", paths);
    let spectrograms = try_join_all(paths.iter().map(|path| async {
        load_file(path)
            .await
            .context(format!("Failed to load file {}", path.display()))
    }))
    .await?;

    Spectrogram::concatenate(&spectrograms)
}

#[derive(Debug)]
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
        return self.freq == other.freq && self.bw == other.bw && self.nchan == other.nchan;
    }

    pub fn nth_following(&self, nth: i32) -> DateTime<Utc> {
        self.start_time
            + chrono::Duration::milliseconds((self.length * 1000.0) as i64) * (nth as i32)
    }
}

#[derive(Clone)]
pub struct Spectrogram {
    pub start_time: DateTime<Utc>,
    pub freq: f32,         // Hz
    pub bw: f32,           // Hz
    pub slice_length: f32, // s
    pub nchan: usize,
    raw_data: Vec<f32>,
}

impl std::fmt::Debug for Spectrogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spectrogram")
            .field("start_time", &self.start_time)
            .field("freq", &self.freq)
            .field("bw", &self.bw)
            .field("slice_length", &self.slice_length)
            .field("nchan", &self.nchan)
            .field(
                "nslices",
                &(self.raw_data.len() as usize / self.nchan as usize),
            )
            .finish()
    }
}

impl Spectrogram {
    pub(self) fn new(first_header: &Header, raw_data: Vec<f32>) -> Self {
        Spectrogram {
            start_time: first_header.start_time,
            freq: first_header.freq,
            bw: first_header.bw,
            slice_length: first_header.length,
            nchan: first_header.nchan,
            raw_data,
        }
    }

    pub fn concatenate(components: &[Spectrogram]) -> Result<Spectrogram> {
        if components.is_empty() {
            bail!("No spectrograms to concatenate");
        }

        let first = &components[0];
        let total_slices: usize = components.iter().map(|s| s.raw_data.len() / s.nchan).sum();
        let mut raw_data: Vec<f32> = Vec::with_capacity(total_slices * first.nchan);
        raw_data.extend_from_slice(&components[0].raw_data);

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
            raw_data.extend_from_slice(&spectrogram.raw_data);
        }

        Ok(Spectrogram {
            start_time: first.start_time,
            freq: first.freq,
            bw: first.bw,
            slice_length: first.slice_length,
            nchan: first.nchan,
            raw_data,
        })
    }

    pub fn data(&self) -> ArrayView2<'_, f32> {
        let nslices = self.raw_data.len() / self.nchan;
        ArrayView2::from_shape((nslices, self.nchan), &self.raw_data).unwrap()
    }

    pub fn length(&self) -> Duration {
        Duration::milliseconds(
            (self.slice_length * 1000.0) as i64 * (self.raw_data.len() as i64 / self.nchan as i64),
        )
    }

    pub fn end_time(&self) -> DateTime<Utc> {
        self.start_time + self.length()
    }
}

async fn load_file(path: &std::path::Path) -> Result<Spectrogram> {
    let file = tokio::fs::File::open(path).await?;
    let file_size = file.metadata().await?.len() as usize;
    let mut reader = tokio::io::BufReader::new(file);

    let header = parse_header(&mut reader)
        .await
        .context("Failed to parse header")?;
    println!("Parsed header: {:?}", header);
    // File alternates between headers and data blocks of size nchan * 4 bytes (f32)
    let data_block_size = header.nchan * 4;
    let n_blocks = file_size / (data_block_size + HEADER_SIZE);

    let mut raw_data: Vec<f32> = Vec::with_capacity(n_blocks * header.nchan);
    unsafe {
        raw_data.set_len(n_blocks * header.nchan);
    }

    let mut data_offset = 0usize;
    parse_data(
        &mut reader,
        &mut raw_data[data_offset..data_offset + header.nchan],
    )
    .await?;
    data_offset += header.nchan;

    while data_offset < raw_data.len() {
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
            &mut raw_data[data_offset..data_offset + header.nchan],
        )
        .await?;
        data_offset += header.nchan;
    }

    Ok(Spectrogram::new(&header, raw_data))
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
    data: &mut [f32],
) -> Result<()> {
    for value in data.iter_mut() {
        *value = reader.read_f32_le().await?;
    }

    Ok(())
}
