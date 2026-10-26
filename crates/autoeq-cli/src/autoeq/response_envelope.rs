//! Pointwise population response spread, not confidence intervals or FIR tap bounds.

use super::display_smoothing::{Mode, smooth};

pub const MAX_POPULATION: usize = 256;
pub const MAX_FREQUENCIES: usize = 4096;

#[derive(Debug, Clone)]
pub struct Envelope {
    pub min: Vec<f64>,
    pub max: Vec<f64>,
    pub p10: Vec<f64>,
    pub p90: Vec<f64>,
    pub count: usize,
}

impl Envelope {
    /// Smooth each candidate before computing the pointwise, equally weighted spread.
    pub fn from_candidates(
        frequency_hz: &[f64],
        responses: &[Vec<f64>],
        mode: Mode,
    ) -> Result<Self, String> {
        if responses.is_empty() || responses.len() > MAX_POPULATION {
            return Err(format!(
                "response envelope requires 1..={MAX_POPULATION} candidates"
            ));
        }
        if frequency_hz.is_empty() || frequency_hz.len() > MAX_FREQUENCIES {
            return Err(format!(
                "response envelope requires 1..={MAX_FREQUENCIES} frequencies"
            ));
        }
        let smoothed = responses
            .iter()
            .map(|response| smooth(frequency_hz, response, mode).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut result = Self {
            min: Vec::with_capacity(frequency_hz.len()),
            max: Vec::with_capacity(frequency_hz.len()),
            p10: Vec::with_capacity(frequency_hz.len()),
            p90: Vec::with_capacity(frequency_hz.len()),
            count: responses.len(),
        };
        let mut column = Vec::with_capacity(responses.len());
        for index in 0..frequency_hz.len() {
            column.clear();
            column.extend(smoothed.iter().map(|response| response[index]));
            column.sort_unstable_by(f64::total_cmp);
            result.min.push(column[0]);
            result.max.push(column[column.len() - 1]);
            result.p10.push(quantile(&column, 0.1));
            result.p90.push(quantile(&column, 0.9));
        }
        Ok(result)
    }
}

fn quantile(sorted: &[f64], probability: f64) -> f64 {
    let position = probability * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let fraction = position - lower as f64;
    sorted[lower] * (1.0 - fraction) + sorted[upper] * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_and_smoothing_precede_population_statistics() {
        let frequencies = [100.0, 101.0, 102.0];
        let responses = vec![vec![0.0, 30.0, 0.0], vec![20.0, 0.0, 20.0]];
        let raw = Envelope::from_candidates(&frequencies, &responses, Mode::Raw).unwrap();
        assert_eq!(raw.count, 2);
        assert_eq!(raw.min, vec![0.0, 0.0, 0.0]);
        assert_eq!(raw.max, vec![20.0, 30.0, 20.0]);
        assert_eq!(raw.p10, vec![2.0, 3.0, 2.0]);
        assert_eq!(raw.p90, vec![18.0, 27.0, 18.0]);
        let smoothed =
            Envelope::from_candidates(&frequencies, &responses, Mode::Psychoacoustic).unwrap();
        let candidate_values = responses
            .iter()
            .map(|response| smooth(&frequencies, response, Mode::Psychoacoustic).unwrap()[0])
            .collect::<Vec<_>>();
        assert_eq!(
            smoothed.min[0],
            candidate_values[0].min(candidate_values[1])
        );
        assert_eq!(
            smoothed.max[0],
            candidate_values[0].max(candidate_values[1])
        );
        let one = Envelope::from_candidates(&[100.0], &[vec![3.0]], Mode::Raw).unwrap();
        assert_eq!(one.min, one.max);
        assert_eq!(one.p10, one.p90);
    }

    #[test]
    fn invalid_and_oversized_populations_are_rejected() {
        assert!(Envelope::from_candidates(&[100.0], &[], Mode::Raw).is_err());
        for (frequencies, responses) in [
            (vec![100.0], vec![vec![]]),
            (vec![100.0, 100.0], vec![vec![0.0, 0.0]]),
            (vec![0.0], vec![vec![0.0]]),
            (vec![100.0], vec![vec![f64::NAN]]),
            (vec![100.0], vec![vec![0.0]; MAX_POPULATION + 1]),
            (
                (1..=MAX_FREQUENCIES + 1).map(|n| n as f64).collect(),
                vec![vec![0.0; MAX_FREQUENCIES + 1]],
            ),
        ] {
            assert!(Envelope::from_candidates(&frequencies, &responses, Mode::Raw).is_err());
        }
    }
}
