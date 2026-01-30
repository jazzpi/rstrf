use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, NaiveDateTime, Utc};
use ndarray::{Array1, ArrayView1, Zip, arr1};
use ndarray_linalg::Norm;
use sgp4::Prediction;
use tokio::io::AsyncBufReadExt;

use super::util::minmax;

/// Loads frequencies from a strf-style frequencies.txt file
pub async fn load_frequencies(path: &std::path::PathBuf) -> anyhow::Result<HashMap<u64, f64>> {
    let file = tokio::fs::File::open(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut freqs = HashMap::new();
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        anyhow::ensure!(
            parts.len() == 2,
            "Expected 2 columns in frequencies file, got: {}",
            line
        );
        let norad_id: u64 = parts[0]
            .parse()
            .with_context(|| format!("Failed to parse NORAD ID from {}", parts[0]))?;
        let freq: f64 = parts[1]
            .parse()
            .with_context(|| format!("Failed to parse frequency from {}", parts[1]))?;
        freqs.insert(norad_id, freq * 1e6);
    }
    Ok(freqs)
}

/// Loads TLEs from the given file
///
/// Parses 2LE, and 3LE with an optional initial 0 in the title line.
pub async fn load_tles(
    path: &std::path::PathBuf,
    tx_freqs: HashMap<u64, f64>,
) -> anyhow::Result<Vec<Satellite>> {
    enum ParseState {
        AwaitLine1OrTitle,
        AwaitLine1(String),
        AwaitLine2(Option<String>, String),
    }

    let file = tokio::fs::File::open(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut state = ParseState::AwaitLine1OrTitle;
    let mut elements = Vec::new();
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        state = match state {
            ParseState::AwaitLine1OrTitle => {
                if line.starts_with("1 ") {
                    ParseState::AwaitLine2(None, line)
                } else if let Some(title) = line.strip_prefix("0 ") {
                    ParseState::AwaitLine1(title.into())
                } else {
                    ParseState::AwaitLine1(line)
                }
            }
            ParseState::AwaitLine1(title) => {
                anyhow::ensure!(
                    line.starts_with("1 "),
                    "Expected line 1 of TLE, got: {}",
                    line
                );
                ParseState::AwaitLine2(Some(title), line)
            }
            ParseState::AwaitLine2(title, line1) => {
                anyhow::ensure!(
                    line.starts_with("2 "),
                    "Expected line 2 of TLE, got: {}",
                    line
                );
                let elem = sgp4::Elements::from_tle(title, line1.as_bytes(), line.as_bytes())
                    .context("Failed to parse TLE")?;
                let constants = sgp4::Constants::from_elements(&elem)
                    .context("Failed to derive SGP4 constants")?;
                let tx_freq = tx_freqs.get(&elem.norad_id).copied().with_context(|| {
                    format!("No transmit frequency found for NORAD ID {}", elem.norad_id)
                })?;
                elements.push(Satellite {
                    elements: elem,
                    constants,
                    tx_freq,
                });
                ParseState::AwaitLine1OrTitle
            }
        };
    }
    Ok(elements)
}

const RADIUS_EARTH: f64 = 6378.137; // km
const SPEED_OF_LIGHT: f64 = 299792.458; // km/s

#[derive(Debug, Clone)]
pub struct Satellite {
    pub elements: sgp4::Elements,
    pub constants: sgp4::Constants,
    pub tx_freq: f64,
}

impl Satellite {
    pub fn predict(&self, time: &NaiveDateTime) -> anyhow::Result<sgp4::Prediction> {
        let minutes = self.elements.datetime_to_minutes_since_epoch(time)?;
        let prediction = self.constants.propagate(minutes)?;
        Ok(prediction)
    }

    pub fn predict_pass(
        &self,
        start: DateTime<Utc>,
        times: ArrayView1<f64>,
        site: Site,
    ) -> (Array1<f64>, Array1<f64>) {
        let mut frequencies = Array1::zeros(times.len());
        let mut angles = Array1::zeros(times.len());
        Zip::from(&times)
            .and(&mut frequencies)
            .and(&mut angles)
            .for_each(|&t, freq, angle| {
                let t = (start + chrono::Duration::milliseconds((t * 1000.0).round() as i64))
                    .naive_utc();
                let prediction = match self.predict(&t) {
                    Ok(prediction) => prediction,
                    Err(e) => {
                        log::warn!(
                            "Failed to predict position for {} at time {}: {}",
                            self.norad_id(),
                            t,
                            e
                        );
                        *freq = f64::NAN;
                        *angle = f64::NAN;
                        return;
                    }
                };
                let site_prediction = site.at_time(&t);
                let site_pos = arr1(&site_prediction.position);
                let delta_pos = arr1(&prediction.position) - &site_pos;
                let delta_vel = arr1(&prediction.velocity) - arr1(&site_prediction.velocity);
                let range = delta_pos.norm();
                let range_rate = delta_pos.dot(&delta_vel) / range;
                *freq = (1.0 - range_rate / SPEED_OF_LIGHT) * self.tx_freq;
                *angle = (delta_pos.dot(&site_pos) / (range * RADIUS_EARTH)).acos();
            });
        (frequencies, angles)
    }

    pub fn norad_id(&self) -> u64 {
        self.elements.norad_id
    }
}

pub struct Site {
    /// Latitude in radians
    pub latitude: f64,
    /// Longitude in radians
    pub longitude: f64,
    /// Altitude in km
    pub altitude: f64,
}

impl Site {
    pub fn at_time(&self, time: &NaiveDateTime) -> sgp4::Prediction {
        // Adapted from strf's obspos_xyz()
        const FLAT: f64 = 1.0 / 298.257;

        let theta = GMST::from(time).0 + self.longitude;
        let dtheta = gmst_deriv_days(time) / 86400.0;

        let s = self.latitude.sin();
        let ff = (1.0 - FLAT * (2.0 - FLAT) * s * s).sqrt();
        let gc = 1.0 / ff + self.altitude / RADIUS_EARTH;
        let gs = (1.0 - FLAT) * (1.0 - FLAT) / ff + self.altitude / RADIUS_EARTH;

        Prediction {
            position: [
                gc * self.latitude.cos() * theta.cos() * RADIUS_EARTH,
                gs * self.latitude.cos() * theta.sin() * RADIUS_EARTH,
                gs * s * RADIUS_EARTH,
            ],
            velocity: [
                -gc * self.latitude.cos() * theta.sin() * RADIUS_EARTH * dtheta,
                gc * self.latitude.cos() * theta.cos() * RADIUS_EARTH * dtheta,
                0.0,
            ],
        }
    }
}

/// Greenwich Mean Sidereal Time in radians
pub struct GMST(f64);

impl From<&NaiveDateTime> for GMST {
    fn from(time: &NaiveDateTime) -> Self {
        let epoch = sgp4::julian_years_since_j2000(time);
        GMST(sgp4::iau_epoch_to_sidereal_time(epoch))
    }
}

/// dtheta/dt where theta is GMST in radians and t is time in Julian days
pub fn gmst_deriv_days(time: &NaiveDateTime) -> f64 {
    // NOT adapted from strf's dgmst() because I'm pretty sure the factors there are incorrect
    // https://www2.mps.mpg.de/homes/fraenz/systems/systems3art/node10.html
    let t_0 = sgp4::julian_years_since_j2000(time) / 100.0;
    (360.98564736629_f64).to_radians() + 2.0 * (0.003875_f64).to_radians() * t_0
        - 3.0 * (2.6e-8_f64).to_radians() * t_0 * t_0
}

pub fn predict_satellites(
    satellites: Vec<Satellite>,
    start_time: DateTime<Utc>,
    length_s: f64,
) -> Predictions {
    let times = ndarray::Array1::linspace(
        0.0, length_s, 1000, // TODO: number of points
    );
    // TODO: Make this configurable
    const SITE: Site = Site {
        latitude: 78.2244_f64.to_radians(),
        longitude: 15.3952_f64.to_radians(),
        altitude: 0.474,
    };
    // TODO: Parallelize predictions?
    let (frequencies, zenith_angles) = satellites
        .iter()
        .map(|sat| {
            let id = sat.norad_id();
            let (freq, za) = sat.predict_pass(start_time, times.view(), SITE);
            ((id, freq), (id, za))
        })
        .unzip();
    Predictions {
        times,
        frequencies,
        zenith_angles,
    }
}

#[derive(Clone)]
pub struct Predictions {
    pub times: Array1<f64>,
    pub frequencies: HashMap<u64, Array1<f64>>,
    pub zenith_angles: HashMap<u64, Array1<f64>>,
}

impl std::fmt::Debug for Predictions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Predictions")
            .field("times", &minmax(&self.times))
            .field("frequencies", &self.frequencies.len())
            .field("zenith_angles", &self.zenith_angles.len())
            .finish()
    }
}
