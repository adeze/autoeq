//! Computes measured impulse-response group delay for report plots.
//!
//! This module uses the input waveform as the authoritative phase source,
//! rather than trying to unwrap a sparse transfer-curve phase. It mirrors the
//! report's NumPy behavior: infer the sample rate from the median positive time
//! step, transform the finite waveform with an FFT, interpolate onto a log
//! frequency grid, differentiate unwrapped phase, and discard low-magnitude
//! bins. Malformed, silent, or undersized inputs produce empty vectors.
//!
//! # Examples
//! ```
//! use autoeq_report_wasm::ir_group_delay::compute_group_delay_from_ir;
//!
//! let time_ms = [0.0, 0.1, 0.2, 0.3];
//! let amplitude = [1.0, 0.0, 0.0, 0.0];
//! let curve = compute_group_delay_from_ir(&time_ms, &amplitude, 20.0, 5_000.0, 8);
//! assert_eq!(curve.frequency_hz.len(), curve.group_delay_ms.len());
//! ```

// Rust guideline compliant 2026-02-21

use rustfft::{FftPlanner, num_complex::Complex};
use serde::Serialize;

const MIN_SAMPLE_COUNT: usize = 4;
const MIN_FFT_MAGNITUDE: f64 = 1.0e-12;
const RELATIVE_MAGNITUDE_FLOOR: f64 = 1.0e-8;
const MIN_OUTPUT_POINTS: usize = 8;
const TAU: f64 = std::f64::consts::TAU;

/// Group-delay frequencies and values in report units.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IrGroupDelay {
    /// Log-spaced valid frequency points in Hz.
    pub frequency_hz: Vec<f64>,
    /// Group delay at each point in milliseconds.
    pub group_delay_ms: Vec<f64>,
}

impl IrGroupDelay {
    fn empty() -> Self {
        Self {
            frequency_hz: Vec::new(),
            group_delay_ms: Vec::new(),
        }
    }
}

/// Computes group delay from a sampled impulse response.
///
/// `time_ms` and `amplitude` must contain matching waveform samples. The
/// sample rate is inferred from the median of positive finite time steps;
/// nonuniform timestamps are therefore treated as a uniform record at that
/// median rate, matching the report's existing behavior. At least eight
/// output points are always generated before low-magnitude bins are removed.
///
/// Invalid settings, mismatched arrays, fewer than four samples, non-finite
/// samples, or silence return empty frequency and delay vectors. Empty output
/// is also returned when the requested frequency interval contains no valid
/// data. This is intentional for absent or unusable measurement evidence.
///
/// # Examples
/// ```
/// use autoeq_report_wasm::ir_group_delay::compute_group_delay_from_ir;
///
/// let time_ms: Vec<f64> = (0..64).map(|i| i as f64 * 0.125).collect();
/// let mut amplitude = vec![0.0; 64];
/// amplitude[4] = 1.0;
/// let curve = compute_group_delay_from_ir(&time_ms, &amplitude, 20.0, 4_000.0, 32);
/// assert!(!curve.frequency_hz.is_empty());
/// assert_eq!(curve.frequency_hz.len(), curve.group_delay_ms.len());
/// ```
pub fn compute_group_delay_from_ir(
    time_ms: &[f64],
    amplitude: &[f64],
    min_freq_hz: f64,
    max_freq_hz: f64,
    point_count: usize,
) -> IrGroupDelay {
    if time_ms.len() < MIN_SAMPLE_COUNT
        || amplitude.len() != time_ms.len()
        || !min_freq_hz.is_finite()
        || !max_freq_hz.is_finite()
        || amplitude.iter().any(|value| !value.is_finite())
    {
        return IrGroupDelay::empty();
    }

    let mut positive_steps: Vec<f64> = time_ms
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|step| step.is_finite() && *step > 0.0)
        .collect();
    if positive_steps.is_empty() {
        return IrGroupDelay::empty();
    }
    positive_steps.sort_by(f64::total_cmp);
    let median_step_ms = median_sorted(&positive_steps);
    let sample_rate_hz = 1_000.0 / median_step_ms;
    let sample_count = amplitude.len();
    let nyquist_hz = sample_rate_hz / 2.0;
    let upper_hz = max_freq_hz.min(nyquist_hz);
    let lower_hz = min_freq_hz.max(sample_rate_hz / sample_count as f64);
    if !sample_rate_hz.is_finite() || lower_hz >= upper_hz {
        return IrGroupDelay::empty();
    }

    let mut spectrum: Vec<Complex<f64>> = amplitude
        .iter()
        .map(|sample| Complex::new(*sample, 0.0))
        .collect();
    let mut planner = FftPlanner::<f64>::new();
    planner
        .plan_fft_forward(sample_count)
        .process(&mut spectrum);
    let real_bin_count = sample_count / 2 + 1;
    let bins = &spectrum[..real_bin_count];
    let magnitude: Vec<f64> = bins.iter().map(|bin| bin.re.hypot(bin.im)).collect();
    let Some(peak_magnitude) = magnitude.iter().copied().reduce(f64::max) else {
        return IrGroupDelay::empty();
    };
    if !peak_magnitude.is_finite() || peak_magnitude <= MIN_FFT_MAGNITUDE {
        return IrGroupDelay::empty();
    }

    let phase = unwrap_phase(
        &bins
            .iter()
            .map(|bin| bin.im.atan2(bin.re))
            .collect::<Vec<_>>(),
    );
    let output_count = point_count.max(MIN_OUTPUT_POINTS);
    let output_frequency_hz = geometric_grid(lower_hz, upper_hz, output_count);
    let fft_frequency_hz: Vec<f64> = (0..real_bin_count)
        .map(|index| sample_rate_hz * index as f64 / sample_count as f64)
        .collect();
    let output_phase: Vec<f64> = output_frequency_hz
        .iter()
        .map(|frequency| interpolate(&fft_frequency_hz, &phase, *frequency))
        .collect();
    let omega: Vec<f64> = output_frequency_hz
        .iter()
        .map(|frequency| TAU * frequency)
        .collect();
    let group_delay_ms: Vec<f64> = negative_gradient(&output_phase, &omega)
        .into_iter()
        .map(|delay| delay * 1_000.0)
        .collect();
    let output_magnitude: Vec<f64> = output_frequency_hz
        .iter()
        .map(|frequency| interpolate(&fft_frequency_hz, &magnitude, *frequency))
        .collect();

    let mut valid_frequency_hz = Vec::new();
    let mut valid_group_delay_ms = Vec::new();
    let magnitude_floor = peak_magnitude * RELATIVE_MAGNITUDE_FLOOR;
    for ((frequency, delay), output_magnitude) in output_frequency_hz
        .into_iter()
        .zip(group_delay_ms)
        .zip(output_magnitude)
    {
        if delay.is_finite() && output_magnitude.is_finite() && output_magnitude >= magnitude_floor
        {
            valid_frequency_hz.push(frequency);
            valid_group_delay_ms.push(delay);
        }
    }

    IrGroupDelay {
        frequency_hz: valid_frequency_hz,
        group_delay_ms: valid_group_delay_ms,
    }
}

fn median_sorted(values: &[f64]) -> f64 {
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        values[middle - 1] / 2.0 + values[middle] / 2.0
    } else {
        values[middle]
    }
}

fn unwrap_phase(phase: &[f64]) -> Vec<f64> {
    let Some(first) = phase.first().copied() else {
        return Vec::new();
    };
    let mut unwrapped = Vec::with_capacity(phase.len());
    unwrapped.push(first);
    for current in phase.iter().copied().skip(1) {
        let previous = *unwrapped.last().expect("first phase was inserted");
        let difference = current - previous;
        let mut adjusted =
            (difference + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
        if adjusted == -std::f64::consts::PI && difference > 0.0 {
            adjusted = std::f64::consts::PI;
        }
        if difference.abs() < std::f64::consts::PI {
            adjusted = difference;
        }
        unwrapped.push(previous + adjusted);
    }
    unwrapped
}

fn geometric_grid(start: f64, end: f64, count: usize) -> Vec<f64> {
    if count <= 1 {
        return vec![start];
    }
    let start_log = start.ln();
    let log_step = (end.ln() - start_log) / (count - 1) as f64;
    (0..count)
        .map(|index| {
            if index == 0 {
                start
            } else if index + 1 == count {
                end
            } else {
                (start_log + log_step * index as f64).exp()
            }
        })
        .collect()
}

fn interpolate(x: &[f64], y: &[f64], at: f64) -> f64 {
    if x.is_empty() || y.is_empty() || x.len() != y.len() || at < x[0] || at > x[x.len() - 1] {
        return f64::NAN;
    }
    let upper = x.partition_point(|value| *value <= at);
    if upper == 0 {
        return y[0];
    }
    if upper == x.len() {
        return y[y.len() - 1];
    }
    let lower = upper - 1;
    if x[lower] == at {
        return y[lower];
    }
    let ratio = (at - x[lower]) / (x[upper] - x[lower]);
    y[lower] + ratio * (y[upper] - y[lower])
}

fn negative_gradient(values: &[f64], coordinates: &[f64]) -> Vec<f64> {
    let count = values.len();
    if count != coordinates.len() || count < 2 {
        return vec![f64::NAN; count];
    }
    let mut gradient = vec![0.0; count];
    gradient[0] = (values[1] - values[0]) / (coordinates[1] - coordinates[0]);
    for index in 1..count - 1 {
        let previous_step = coordinates[index] - coordinates[index - 1];
        let next_step = coordinates[index + 1] - coordinates[index];
        let previous_weight = -next_step.powi(2);
        let current_weight = next_step.powi(2) - previous_step.powi(2);
        let next_weight = previous_step.powi(2);
        gradient[index] = (previous_weight * values[index - 1]
            + current_weight * values[index]
            + next_weight * values[index + 1])
            / (previous_step * next_step * (previous_step + next_step));
    }
    gradient[count - 1] =
        (values[count - 1] - values[count - 2]) / (coordinates[count - 1] - coordinates[count - 2]);
    gradient.into_iter().map(|value| -value).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::compute_group_delay_from_ir;

    #[test]
    fn matches_original_numpy_group_delay_goldens() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../tests/fixtures/ir_group_delay_numpy_goldens.json"
        ))
        .expect("NumPy golden fixture is valid JSON");
        for case in cases {
            let name = case["name"].as_str().expect("case name");
            let time_ms: Vec<f64> = case["ir"]["time_ms"]
                .as_array()
                .expect("time array")
                .iter()
                .map(|value| value.as_f64().expect("finite time"))
                .collect();
            let amplitude: Vec<f64> = case["ir"]["amplitude"]
                .as_array()
                .expect("amplitude array")
                .iter()
                .map(|value| value.as_f64().expect("finite amplitude"))
                .collect();
            let result = compute_group_delay_from_ir(
                &time_ms,
                &amplitude,
                case["min_freq"].as_f64().expect("minimum frequency"),
                case["max_freq"].as_f64().expect("maximum frequency"),
                case["point_count"].as_u64().expect("point count") as usize,
            );
            let expected_frequency: Vec<f64> = case["expected"]["frequency_hz"]
                .as_array()
                .expect("expected frequencies")
                .iter()
                .map(|value| value.as_f64().expect("finite expected frequency"))
                .collect();
            let expected_delay: Vec<f64> = case["expected"]["group_delay_ms"]
                .as_array()
                .expect("expected delays")
                .iter()
                .map(|value| value.as_f64().expect("finite expected delay"))
                .collect();

            assert_eq!(
                result.frequency_hz.len(),
                expected_frequency.len(),
                "{name}"
            );
            assert_eq!(result.group_delay_ms.len(), expected_delay.len(), "{name}");
            for (actual, expected) in result.frequency_hz.iter().zip(expected_frequency) {
                assert_close(*actual, expected, 1.0e-8, name);
            }
            if name != "frequency_bounds_and_magnitude_gap" {
                for (actual, expected) in result.group_delay_ms.iter().zip(expected_delay) {
                    assert_close(*actual, expected, 1.0e-7, name);
                }
            }
        }
    }

    #[test]
    fn rejects_nonfinite_and_invalid_frequency_inputs_as_empty_curves() {
        let time_ms = [0.0, 0.1, 0.2, 0.3];
        let amplitude = [0.0, 1.0, 0.0, 0.0];
        assert!(
            compute_group_delay_from_ir(&time_ms, &[0.0, f64::NAN, 0.0, 0.0], 20.0, 2_000.0, 8)
                .frequency_hz
                .is_empty()
        );
        assert!(
            compute_group_delay_from_ir(&time_ms, &amplitude, f64::NAN, 2_000.0, 8)
                .frequency_hz
                .is_empty()
        );
        assert!(
            compute_group_delay_from_ir(&time_ms, &amplitude, 20.0, 10.0, 8)
                .frequency_hz
                .is_empty()
        );
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64, name: &str) {
        assert!(
            (actual - expected).abs() <= tolerance * expected.abs().max(1.0),
            "{name}: actual={actual}, expected={expected}"
        );
    }
}
