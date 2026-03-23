use std::{any::TypeId, hash::Hash, path::PathBuf};

use chrono::Duration;
use futures_util::SinkExt;
use iced::{
    Subscription,
    advanced::{
        graphics::futures::BoxStream,
        subscription::{self, Recipe},
    },
    futures::channel::mpsc,
    stream::channel,
};
use num_traits::FromPrimitive;
use plotters_iced2::sample;
use rustfft::{FftNum, FftPlanner};
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};
use tokio::io::AsyncReadExt;

#[derive(Debug, Clone)]
pub enum Message {
    LoadFile(PathBuf),
}

pub trait SampleType: Copy + FftNum + FromPrimitive + Send + 'static {}

pub struct ImportIq<T: SampleType> {
    // TODO
    planner: FftPlanner<T>,
    sample_format: SampleFormat,
}

impl<T: SampleType> ImportIq<T> {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::LoadFile(path) => {
                todo!()
            }
        }
    }

    async fn read_sample<R: tokio::io::AsyncRead + Unpin>(
        &self,
        reader: &mut R,
    ) -> anyhow::Result<T> {
        let opt = match self.sample_format {
            SampleFormat::CS8 => {
                T::from_i8(reader.read_i8().await?).map(|v| v / -(T::from_i8(i8::MIN).unwrap()))
            }
            SampleFormat::CS16 => T::from_i16(reader.read_i16_le().await?)
                .map(|v| v / -(T::from_i16(i16::MIN).unwrap())),
            SampleFormat::CS32 => T::from_i32(reader.read_i32_le().await?)
                .map(|v| v / -(T::from_i32(i32::MIN).unwrap())),
            SampleFormat::CS64 => T::from_i64(reader.read_i64_le().await?)
                .map(|v| v / -(T::from_i64(i64::MIN).unwrap())),
            SampleFormat::CF32 => T::from_f32(reader.read_f32_le().await?).map(|v| v),
            SampleFormat::CF64 => T::from_f64(reader.read_f64_le().await?).map(|v| v),
        };
        opt.ok_or(anyhow::anyhow!("Failed to convert sample to target type"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Serialize, Deserialize, VariantArray, Display)]
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
}

// impl<T: SampleType> From<SampleFormat> for T {
//     fn from(format: SampleFormat) -> Self {
//         match format {
//             SampleFormat::CS8 => T::from_i8(0).unwrap(),
//             SampleFormat::CS16 => T::from_i16(0).unwrap(),
//             SampleFormat::CS32 => T::from_i32(0).unwrap(),
//             SampleFormat::CS64 => T::from_i64(0).unwrap(),
//             SampleFormat::CF32 => T::from_f32(0.0).unwrap(),
//             SampleFormat::CF64 => T::from_f64(0.0).unwrap(),
//         }
//     }
// }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IqFormat {
    pub samples: SampleFormat,
    pub sample_rate: f32,
}

trait DataSource: Send {
    type SampleType: SampleType;

    fn read_frame(
        &mut self,
        sample_format: SampleFormat,
        fft_size: usize,
    ) -> impl Future<Output = anyhow::Result<Vec<Self::SampleType>>> + Send;

    // fn into_slices(
    //     self,
    //     sample_format: SampleFormat,
    //     fft_size: usize,
    //     slice_length: Duration,
    // ) -> Subscription<Vec<Self::SampleType>>
    // where
    //     Self: Sized + Hash,
    // {
    //     let source = self;
    //     Subscription::run_with(
    //         (source, sample_format, fft_size, slice_length),
    //         |&(mut source, sample_format, fft_size, slice_length)| {
    //             channel(10, async move |mut output| {
    //                 loop {
    //                     let Ok(frame) = source.read_frame(sample_format, fft_size).await else {
    //                         break;
    //                     };
    //                     output.send(frame).await;
    //                 }
    //             })
    //         },
    //     )
    // }
}

struct DataSourceRecipe<S: DataSource> {
    source: S,
    sample_format: SampleFormat,
    fft_size: usize,
}

impl<S> subscription::Recipe for DataSourceRecipe<S>
where
    S: DataSource + Send + 'static,
{
    type Output = Vec<S::SampleType>;

    fn hash(&self, state: &mut subscription::Hasher) {
        TypeId::of::<S>().hash(state);
        self.sample_format.hash(state);
        self.fft_size.hash(state);
    }

    fn stream(self: Box<Self>, input: subscription::EventStream) -> BoxStream<Self::Output> {
        let DataSourceRecipe {
            source,
            sample_format,
            fft_size,
        } = *self;
        Box::pin(channel(10, async move |mut output| {
            let mut source = source;
            loop {
                let Ok(frame) = source.read_frame(sample_format, fft_size).await else {
                    break;
                };
                output.send(frame).await;
            }
        }))
    }
}

struct FileReader<T: SampleType> {
    path: PathBuf,
    reader: tokio::io::BufReader<tokio::fs::File>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: SampleType> FileReader<T> {
    pub fn new(path: PathBuf) -> anyhow::Result<Self> {
        let file = std::fs::File::open(&path)?;
        let reader = tokio::io::BufReader::new(tokio::fs::File::from_std(file));
        Ok(Self {
            path,
            reader,
            _marker: std::marker::PhantomData,
        })
    }

    async fn read_sample<R: tokio::io::AsyncRead + Unpin>(
        &mut self,
        sample_format: SampleFormat,
    ) -> anyhow::Result<T> {
        let opt = match sample_format {
            SampleFormat::CS8 => T::from_i8(self.reader.read_i8().await?)
                .map(|v| v / -(T::from_i8(i8::MIN).unwrap())),
            SampleFormat::CS16 => T::from_i16(self.reader.read_i16_le().await?)
                .map(|v| v / -(T::from_i16(i16::MIN).unwrap())),
            SampleFormat::CS32 => T::from_i32(self.reader.read_i32_le().await?)
                .map(|v| v / -(T::from_i32(i32::MIN).unwrap())),
            SampleFormat::CS64 => T::from_i64(self.reader.read_i64_le().await?)
                .map(|v| v / -(T::from_i64(i64::MIN).unwrap())),
            SampleFormat::CF32 => T::from_f32(self.reader.read_f32_le().await?).map(|v| v),
            SampleFormat::CF64 => T::from_f64(self.reader.read_f64_le().await?).map(|v| v),
        };
        opt.ok_or(anyhow::anyhow!("Failed to convert sample to target type"))
    }
}

impl<T: SampleType> DataSource for FileReader<T> {
    type SampleType = T;

    async fn read_frame(
        &mut self,
        sample_format: SampleFormat,
        fft_size: usize,
    ) -> anyhow::Result<Vec<Self::SampleType>> {
        read_frame(&mut self.reader, sample_format, fft_size).await
    }
}

async fn read_sample<T: SampleType, R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    sample_format: SampleFormat,
) -> anyhow::Result<T> {
    let opt =
        match sample_format {
            SampleFormat::CS8 => {
                T::from_i8(reader.read_i8().await?).map(|v| v / -(T::from_i8(i8::MIN).unwrap()))
            }
            SampleFormat::CS16 => T::from_i16(reader.read_i16_le().await?)
                .map(|v| v / -(T::from_i16(i16::MIN).unwrap())),
            SampleFormat::CS32 => T::from_i32(reader.read_i32_le().await?)
                .map(|v| v / -(T::from_i32(i32::MIN).unwrap())),
            SampleFormat::CS64 => T::from_i64(reader.read_i64_le().await?)
                .map(|v| v / -(T::from_i64(i64::MIN).unwrap())),
            SampleFormat::CF32 => T::from_f32(reader.read_f32_le().await?).map(|v| v),
            SampleFormat::CF64 => T::from_f64(reader.read_f64_le().await?).map(|v| v),
        };
    opt.ok_or(anyhow::anyhow!("Failed to convert sample to target type"))
}

async fn read_frame<T: SampleType, R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    sample_format: SampleFormat,
    fft_size: usize,
) -> anyhow::Result<Vec<T>> {
    let mut samples = Vec::with_capacity(fft_size);
    for _ in 0..fft_size {
        samples.push(read_sample(reader, sample_format).await?);
    }
    Ok(samples)
}
