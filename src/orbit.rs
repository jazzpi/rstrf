use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, NaiveDateTime, Utc};
use ndarray::{Array1, ArrayView1, Zip, arr1};
use ndarray_linalg::Norm;
use serde::{Deserialize, Serialize};
use sgp4::Prediction;
use tokio::io::AsyncBufReadExt;

use super::util::minmax;

/// Loads frequencies from a strf-style frequencies.txt file
pub async fn load_frequencies(path: &std::path::PathBuf) -> anyhow::Result<HashMap<u64, Vec<f64>>> {
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
        freqs
            .entry(norad_id)
            .or_insert_with(Vec::new)
            .push(freq * 1e6);
    }
    Ok(freqs)
}

/// Loads TLEs from the given file
///
/// Parses 2LE, and 3LE with an optional initial 0 in the title line.
pub async fn load_tles(
    path: &std::path::PathBuf,
    tx_freqs: HashMap<u64, Vec<f64>>,
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
                if line.starts_with("1 ") {
                    ParseState::AwaitLine2(Some(title), line)
                } else {
                    log::warn!("Expected line 1 of TLE, got: {}", line);
                    ParseState::AwaitLine1OrTitle
                }
            }
            ParseState::AwaitLine2(title, line1) => {
                if line.starts_with("2 ") {
                    let sat = Satellite::from_tle(title, &line1, &line, &tx_freqs);
                    match sat {
                        Ok(sat) => elements.push(sat),
                        Err(e) => {
                            log::warn!("Failed to parse TLE:\n{}\n{}\nError: {}", line1, line, e)
                        }
                    }
                } else {
                    log::warn!("Expected line 2 of TLE, got: {}", line);
                }
                ParseState::AwaitLine1OrTitle
            }
        };
    }
    Ok(elements)
}

const RADIUS_EARTH: f64 = 6378.137; // km
const SPEED_OF_LIGHT: f64 = 299792.458; // km/s

#[derive(Debug, Clone, Serialize)]
pub struct Satellite {
    pub elements: sgp4::Elements,
    #[serde(skip)]
    pub constants: sgp4::Constants,
    pub transmitters: Vec<f64>,
}

impl<'de> Deserialize<'de> for Satellite {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SatelliteHelper {
            elements: sgp4::Elements,
            transmitters: Vec<f64>,
        }
        let helper = SatelliteHelper::deserialize(deserializer)?;
        let constants =
            sgp4::Constants::from_elements(&helper.elements).map_err(serde::de::Error::custom)?;
        Ok(Satellite {
            elements: helper.elements,
            constants,
            transmitters: helper.transmitters,
        })
    }
}

impl PartialEq for Satellite {
    fn eq(&self, other: &Self) -> bool {
        self.transmitters == other.transmitters && self.elements == other.elements
    }
}

impl Satellite {
    pub fn from_tle(
        title: Option<String>,
        line1: &str,
        line2: &str,
        tx_freqs: &HashMap<u64, Vec<f64>>,
    ) -> anyhow::Result<Self> {
        let elements = sgp4::Elements::from_tle(title, line1.as_bytes(), line2.as_bytes())
            .context("Failed to parse TLE")?;
        let constants =
            sgp4::Constants::from_elements(&elements).context("Failed to derive SGP4 constants")?;
        let transmitters = tx_freqs
            .get(&elements.norad_id)
            .cloned()
            .unwrap_or_default();
        Ok(Satellite {
            elements,
            constants,
            transmitters,
        })
    }
    pub fn predict(&self, time: &NaiveDateTime) -> anyhow::Result<sgp4::Prediction> {
        let minutes = self.elements.datetime_to_minutes_since_epoch(time)?;
        let prediction = self.constants.propagate(minutes)?;
        Ok(prediction)
    }

    pub fn predict_pass(
        &self,
        start: DateTime<Utc>,
        times: ArrayView1<f64>,
        site: &Site,
    ) -> (Vec<Array1<f64>>, Array1<f64>) {
        if self.transmitters.is_empty() {
            log::warn!(
                "Predicting pass for {} which has no transmitters",
                self.norad_id()
            );
        }

        let n = times.len();
        let mut range_rates = Array1::zeros(n);
        let mut angles = Array1::zeros(n);
        let mut warned = false;
        Zip::from(&times)
            .and(&mut range_rates)
            .and(&mut angles)
            .for_each(|&t, rr, angle| {
                let t = (start + chrono::Duration::milliseconds((t * 1000.0).round() as i64))
                    .naive_utc();
                let prediction = match self.predict(&t) {
                    Ok(prediction) => prediction,
                    Err(e) => {
                        if !warned {
                            log::warn!(
                                "Failed to predict position for {} at time {}: {}",
                                self.norad_id(),
                                t,
                                e
                            );
                            warned = true;
                        }
                        *rr = f64::NAN;
                        *angle = f64::NAN;
                        return;
                    }
                };
                let site_prediction = site.at_time(&t);
                let site_pos = arr1(&site_prediction.position);
                let delta_pos = arr1(&prediction.position) - &site_pos;
                let delta_vel = arr1(&prediction.velocity) - arr1(&site_prediction.velocity);
                let range = delta_pos.norm();
                *rr = delta_pos.dot(&delta_vel) / range;
                *angle = (delta_pos.dot(&site_pos) / (range * RADIUS_EARTH)).acos();
            });
        let frequencies = self
            .transmitters
            .iter()
            .map(|&tx_freq| range_rates.mapv(|rr| (1.0 - rr / SPEED_OF_LIGHT) * tx_freq))
            .collect();
        (frequencies, angles)
    }

    pub fn norad_id(&self) -> u64 {
        self.elements.norad_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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
    satellites: &[Satellite],
    start_time: DateTime<Utc>,
    length_s: f64,
    site: &Site,
    visible_only: bool,
) -> Predictions {
    let times = ndarray::Array1::linspace(0.0, length_s, length_s.round() as usize);
    // TODO: Parallelize predictions?
    let (frequencies, zenith_angles) = satellites
        .iter()
        .filter(|sat| !(visible_only && sat.transmitters.is_empty()))
        .filter_map(|sat| {
            let id = sat.norad_id();
            let (freqs, za) = sat.predict_pass(start_time, times.view(), site);
            if visible_only && za.iter().all(|&angle| angle > std::f64::consts::FRAC_PI_2) {
                None
            } else {
                Some(((id, freqs), (id, za)))
            }
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
    frequencies: HashMap<u64, Vec<Array1<f64>>>,
    zenith_angles: HashMap<u64, Array1<f64>>,
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

pub struct SatPrediction<'a> {
    pub frequencies: Vec<ndarray::ArrayView1<'a, f64>>,
    pub zenith_angle: ndarray::ArrayView1<'a, f64>,
}

impl Predictions {
    pub fn for_id(&self, id: u64) -> Option<SatPrediction<'_>> {
        Some(SatPrediction {
            frequencies: self
                .frequencies
                .get(&id)?
                .iter()
                .map(|a| a.view())
                .collect(),
            zenith_angle: self.zenith_angles.get(&id)?.view(),
        })
    }

    pub fn iter_satellites(
        &self,
    ) -> impl Iterator<Item = (u64, Vec<Array1<f64>>, Array1<f64>)> + '_ {
        self.frequencies.iter().filter_map(|(&id, freqs)| {
            let za = self.zenith_angles.get(&id)?;
            Some((id, freqs.clone(), za.clone()))
        })
    }

    pub fn n_satellites(&self) -> usize {
        self.frequencies.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Utc};
    use std::f64::consts::PI;

    fn j2000() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2000, 1, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    #[test]
    fn gmst_deriv_is_near_sidereal_rate() {
        let rate = gmst_deriv_days(&j2000());
        let expected = 360.98564736629_f64.to_radians();
        assert!(
            (rate - expected).abs() < 1e-6,
            "rate={}, expected={}",
            rate,
            expected
        );
    }

    #[test]
    fn gmst_is_finite() {
        let gmst = GMST::from(&j2000());
        assert!(gmst.0.is_finite(), "GMST = {}", gmst.0);
    }

    #[test]
    fn gmst_j2000_approx_4895_rad() {
        // Published GMST at J2000.0 ≈ 280.46° ≈ 4.895 rad
        let gmst = GMST::from(&j2000());
        assert!(
            (gmst.0 - 4.895).abs() < 0.1,
            "GMST = {} rad, expected ~4.895",
            gmst.0
        );
    }

    #[test]
    fn equatorial_site_z_is_zero() {
        let site = Site {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let pred = site.at_time(&j2000());
        assert!(pred.position[2].abs() < 1e-6, "z={}", pred.position[2]);
        assert!(pred.velocity[2].abs() < 1e-6, "vz={}", pred.velocity[2]);
    }

    #[test]
    fn equatorial_site_radius_near_earth_radius() {
        let site = Site {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let pred = site.at_time(&j2000());
        let r = pred.position.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!((r - RADIUS_EARTH).abs() < 50.0, "r={}", r);
    }

    #[test]
    fn polar_site_xy_near_zero() {
        let site = Site {
            latitude: PI / 2.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let pred = site.at_time(&j2000());
        assert!(pred.position[0].abs() < 1e-6, "x={}", pred.position[0]);
        assert!(pred.position[1].abs() < 1e-6, "y={}", pred.position[1]);
        assert!(pred.velocity[0].abs() < 1e-6, "vx={}", pred.velocity[0]);
        assert!(pred.velocity[1].abs() < 1e-6, "vy={}", pred.velocity[1]);
    }

    #[test]
    fn predict_satellites_empty_input_gives_empty_output() {
        let predictions = predict_satellites(&[], Utc::now(), 10.0, &Site::default(), false);
        assert_eq!(predictions.n_satellites(), 0);
    }

    #[test]
    fn satellite_from_valid_tle() {
        // VANGUARD 1 - classic sgp4 test TLE from Vallado 2006
        let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
        let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
        let sat = Satellite::from_tle(
            Some("VANGUARD 1".to_string()),
            line1,
            line2,
            &HashMap::new(),
        )
        .expect("TLE should parse successfully");
        assert_eq!(sat.norad_id(), 5);
        assert!(sat.transmitters.is_empty());
    }

    #[test]
    fn satellite_from_tle_with_frequency() {
        let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
        let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
        let mut freqs = HashMap::new();
        freqs.insert(5u64, vec![108.03e6, 109.025e6]);
        let sat =
            Satellite::from_tle(Some("VANGUARD 1".to_string()), line1, line2, &freqs).unwrap();
        assert_eq!(sat.transmitters, vec![108.03e6, 109.025e6]);
    }
}
