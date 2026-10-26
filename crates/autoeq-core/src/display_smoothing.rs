//! Display-only magnitude smoothing shared with Swift `FrequencyDisplaySmoother`.
//! Never use these arrays for objective scores, filter design, phase or qualification.

use std::{error::Error, fmt, str::FromStr};

pub const ALGORITHM_VERSION: &str = "log-cell-rectangular-cubic-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Raw,
    OctaveThird,
    OctaveSixth,
    OctaveTwelfth,
    OctaveTwentyFourth,
    OctaveFortyEighth,
    Psychoacoustic,
}

impl Mode {
    pub const ALL: [Self; 7] = [
        Self::Raw,
        Self::OctaveThird,
        Self::OctaveSixth,
        Self::OctaveTwelfth,
        Self::OctaveTwentyFourth,
        Self::OctaveFortyEighth,
        Self::Psychoacoustic,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::OctaveThird => "octaveThird",
            Self::OctaveSixth => "octaveSixth",
            Self::OctaveTwelfth => "octaveTwelfth",
            Self::OctaveTwentyFourth => "octaveTwentyFourth",
            Self::OctaveFortyEighth => "octaveFortyEighth",
            Self::Psychoacoustic => "psychoacoustic",
        }
    }

    /// Full window width, symmetric in log2(frequency); inputs are validated by `smooth`.
    pub fn bandwidth_octaves(self, frequency_hz: f64) -> f64 {
        match self {
            Self::Raw => 0.0,
            Self::OctaveThird => 1.0 / 3.0,
            Self::OctaveSixth => 1.0 / 6.0,
            Self::OctaveTwelfth => 1.0 / 12.0,
            Self::OctaveTwentyFourth => 1.0 / 24.0,
            Self::OctaveFortyEighth => 1.0 / 48.0,
            Self::Psychoacoustic => {
                1.0 / 3.0 - (frequency_hz / 100.0).log10().clamp(0.0, 1.0) / 6.0
            }
        }
    }
}

impl FromStr for Mode {
    type Err = SmoothingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .ok_or(SmoothingError::UnknownMode)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmoothingError {
    InvalidSamples,
    NumericalRange,
    UnknownMode,
}

impl fmt::Display for SmoothingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSamples => {
                "display smoothing requires positive increasing frequencies and finite paired dB"
            }
            Self::NumericalRange => "display smoothing exceeded finite numerical range",
            Self::UnknownMode => "unknown display smoothing mode",
        })
    }
}

impl Error for SmoothingError {}

/// Midpoint cells are integrated over rectangular log-frequency windows, truncated
/// and renormalized at source edges. Fractional modes average dB; psychoacoustic
/// averages cubed linear amplitude. This is not REW Gaussian/IIR parity.
pub fn smooth(
    frequencies_hz: &[f64],
    levels_db: &[f64],
    mode: Mode,
) -> Result<Vec<f64>, SmoothingError> {
    if frequencies_hz.is_empty()
        || frequencies_hz.len() != levels_db.len()
        || frequencies_hz.iter().any(|x| !x.is_finite() || *x <= 0.0)
        || levels_db.iter().any(|x| !x.is_finite())
        || frequencies_hz.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(SmoothingError::InvalidSamples);
    }
    if mode == Mode::Raw || frequencies_hz.len() == 1 {
        return Ok(levels_db.to_vec());
    }
    let axis: Vec<_> = frequencies_hz.iter().map(|x| x.log2()).collect();
    if axis.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(SmoothingError::NumericalRange);
    }
    let mut edges = Vec::with_capacity(axis.len() + 1);
    edges.push(axis[0]);
    edges.extend(axis.windows(2).map(|pair| (pair[0] + pair[1]) / 2.0));
    edges.push(axis[axis.len() - 1]);
    let mut result = Vec::with_capacity(axis.len());
    let mut start = 0;
    for (index, frequency) in frequencies_hz.iter().enumerate() {
        let half_width = mode.bandwidth_octaves(*frequency) / 2.0;
        let low = axis[0].max(axis[index] - half_width);
        let high = axis[axis.len() - 1].min(axis[index] + half_width);
        while start + 1 < edges.len() && edges[start + 1] <= low {
            start += 1;
        }
        let mut end = start;
        while end < axis.len() && edges[end] < high {
            end += 1;
        }
        let reference = if mode == Mode::Psychoacoustic {
            levels_db[start..end]
                .iter()
                .copied()
                .reduce(f64::max)
                .unwrap_or(levels_db[index])
        } else {
            0.0
        };
        let scale = if mode == Mode::Psychoacoustic {
            1.0
        } else {
            levels_db[start..end]
                .iter()
                .fold(1.0_f64, |value, next| value.max(next.abs()))
        };
        let mut sum = 0.0;
        let mut weight = 0.0;
        for sample in start..end {
            let overlap = (high.min(edges[sample + 1]) - low.max(edges[sample])).max(0.0);
            let value = if mode == Mode::Psychoacoustic {
                10.0_f64.powf((levels_db[sample] - reference) * (3.0 / 20.0))
            } else {
                levels_db[sample] / scale
            };
            sum += overlap * value;
            weight += overlap;
        }
        result.push(if mode == Mode::Psychoacoustic {
            reference + (20.0 / 3.0) * (sum / weight).log10()
        } else {
            sum / weight * scale
        });
    }
    if result.iter().any(|x| !x.is_finite()) {
        return Err(SmoothingError::NumericalRange);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_swift_reference_vectors() {
        let fixture = include_str!("../tests/fixtures/display_smoothing_v1.tsv");
        let mut lines = fixture.lines().filter(|line| !line.starts_with('#'));
        let modes: Vec<Mode> = lines
            .next()
            .unwrap()
            .split('\t')
            .skip(2)
            .map(|name| name.parse().unwrap())
            .collect();
        let rows: Vec<Vec<f64>> = lines
            .map(|line| {
                line.split('\t')
                    .map(|value| value.parse().unwrap())
                    .collect()
            })
            .collect();
        let frequencies: Vec<_> = rows.iter().map(|row| row[0]).collect();
        let levels: Vec<_> = rows.iter().map(|row| row[1]).collect();
        for (column, mode) in modes.into_iter().enumerate() {
            for (index, value) in smooth(&frequencies, &levels, mode)
                .unwrap()
                .iter()
                .enumerate()
            {
                assert!(
                    (value - rows[index][column + 2]).abs() < 1e-10,
                    "{} sample {index}: {value}",
                    mode.as_str()
                );
            }
        }
    }

    #[test]
    fn validation_edges_flatness_and_cubic_mean() {
        for mode in Mode::ALL {
            assert_eq!(mode.as_str().parse::<Mode>().unwrap(), mode);
            assert_eq!(smooth(&[100.0], &[7.0], mode).unwrap(), [7.0]);
            for axis in [
                vec![],
                vec![0.0, 100.0],
                vec![100.0, 100.0],
                vec![200.0, 100.0],
                vec![100.0, f64::NAN],
            ] {
                assert_eq!(
                    smooth(&axis, &vec![0.0; axis.len()], mode),
                    Err(SmoothingError::InvalidSamples)
                );
            }
            assert!(smooth(&[100.0], &[], mode).is_err());
            assert!(smooth(&[100.0], &[f64::INFINITY], mode).is_err());
            let flat = smooth(&[100.0, 101.0, 102.0], &[-81.0; 3], mode).unwrap();
            assert!(flat.iter().all(|value| (value + 81.0).abs() < 1e-10));
        }
        assert!("bogus".parse::<Mode>().is_err());
        assert_eq!(
            smooth(
                &[1e300, 1e300_f64.next_up()],
                &[0.0, 3.0],
                Mode::Psychoacoustic
            ),
            Err(SmoothingError::NumericalRange)
        );
        let cubic = smooth(
            &[100.0, 101.0],
            &[0.0, 20.0 * 2.0_f64.log10()],
            Mode::Psychoacoustic,
        )
        .unwrap();
        assert!(
            cubic
                .iter()
                .all(|value| (value - (20.0 / 3.0) * 4.5_f64.log10()).abs() < 1e-10)
        );
        assert_eq!(Mode::Psychoacoustic.bandwidth_octaves(20.0), 1.0 / 3.0);
        assert!((Mode::Psychoacoustic.bandwidth_octaves(100000.0_f64.sqrt()) - 0.25).abs() < 1e-12);
        assert_eq!(Mode::Psychoacoustic.bandwidth_octaves(20000.0), 1.0 / 6.0);
    }
}
