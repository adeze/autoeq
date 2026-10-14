//! Deterministic, I/O-free transforms over canonical frequency-response curves.

use crate::Curve;
use crate::phase_utils::unwrap_phase_degrees;
use ndarray::Array1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Low frequency bound for the default response normalization band.
pub const NORMALIZE_LOW_FREQ: f64 = 1000.0;

/// High frequency bound for the default response normalization band.
pub const NORMALIZE_HIGH_FREQ: f64 = 2000.0;

pub(crate) fn interpolate_log_space_values(
    transformed_output_frequencies: &[f64],
    transformed_input_frequencies: &[f64],
    input_values: &Array1<f64>,
) -> Array1<f64> {
    let output_len = transformed_output_frequencies.len();
    let input_len = transformed_input_frequencies.len();
    let mut output_values = Array1::zeros(output_len);
    if input_len == 0 {
        return output_values;
    }

    for (index, &target) in transformed_output_frequencies.iter().enumerate() {
        if target <= transformed_input_frequencies[0] {
            if input_len >= 2 {
                let denominator =
                    transformed_input_frequencies[1] - transformed_input_frequencies[0];
                output_values[index] = if denominator.abs() < 1e-10 {
                    input_values[0]
                } else {
                    let slope = (input_values[1] - input_values[0]) / denominator;
                    input_values[0] + slope * (target - transformed_input_frequencies[0])
                };
            } else {
                output_values[index] = input_values[0];
            }
        } else if target >= transformed_input_frequencies[input_len - 1] {
            if input_len >= 2 {
                let denominator = transformed_input_frequencies[input_len - 1]
                    - transformed_input_frequencies[input_len - 2];
                output_values[index] = if denominator.abs() < 1e-10 {
                    input_values[input_len - 1]
                } else {
                    let slope =
                        (input_values[input_len - 1] - input_values[input_len - 2]) / denominator;
                    input_values[input_len - 1]
                        + slope * (target - transformed_input_frequencies[input_len - 1])
                };
            } else {
                output_values[index] = input_values[input_len - 1];
            }
        } else {
            let upper = transformed_input_frequencies.partition_point(|&value| value < target);
            let lower = upper.saturating_sub(1).min(input_len - 2);
            let denominator =
                transformed_input_frequencies[lower + 1] - transformed_input_frequencies[lower];
            output_values[index] = if denominator.abs() < 1e-10 {
                input_values[lower]
            } else {
                let fraction = (target - transformed_input_frequencies[lower]) / denominator;
                input_values[lower] * (1.0 - fraction) + input_values[lower + 1] * fraction
            };
        }
    }
    output_values
}

/// Mean of a piecewise-linear response over a logarithmic-frequency interval.
pub fn mean_over_log_frequency(
    frequencies: &Array1<f64>,
    values: &Array1<f64>,
    minimum_frequency: f64,
    maximum_frequency: f64,
) -> Option<f64> {
    if frequencies.len() != values.len()
        || frequencies.len() < 2
        || minimum_frequency > maximum_frequency
    {
        return None;
    }

    let lower = minimum_frequency.max(frequencies[0]);
    let upper = maximum_frequency.min(frequencies[frequencies.len() - 1]);
    if !lower.is_finite() || !upper.is_finite() || lower <= 0.0 || upper <= lower {
        return None;
    }

    let log_lower = lower.ln();
    let log_upper = upper.ln();
    let mut integral = 0.0;
    let mut width = 0.0;
    for index in 0..frequencies.len() - 1 {
        let frequency_0 = frequencies[index];
        let frequency_1 = frequencies[index + 1];
        let value_0 = values[index];
        let value_1 = values[index + 1];
        if !frequency_0.is_finite()
            || !frequency_1.is_finite()
            || !value_0.is_finite()
            || !value_1.is_finite()
            || frequency_0 <= 0.0
            || frequency_1 <= frequency_0
        {
            continue;
        }

        let x_0 = frequency_0.ln();
        let x_1 = frequency_1.ln();
        let interval_start = x_0.max(log_lower);
        let interval_end = x_1.min(log_upper);
        if interval_end <= interval_start {
            continue;
        }
        let segment_width = x_1 - x_0;
        let value_at = |x: f64| value_0 + (value_1 - value_0) * ((x - x_0) / segment_width);
        integral += 0.5
            * (value_at(interval_start) + value_at(interval_end))
            * (interval_end - interval_start);
        width += interval_end - interval_start;
    }
    (width > 0.0).then_some(integral / width)
}

/// Reuse exact piecewise-linear log-frequency integrals for smoothing windows.
struct LogFrequencyIntegral<'a> {
    axis: Vec<f64>,
    values: &'a Array1<f64>,
    integral: Vec<f64>,
}

impl<'a> LogFrequencyIntegral<'a> {
    fn new(frequencies: &Array1<f64>, values: &'a Array1<f64>) -> Option<Self> {
        if frequencies.len() != values.len()
            || frequencies.len() < 2
            || frequencies.iter().any(|f| !f.is_finite() || *f <= 0.0)
            || values.iter().any(|v| !v.is_finite())
        {
            return None;
        }
        let axis: Vec<_> = frequencies.iter().map(|f| f.ln()).collect();
        if axis.windows(2).any(|pair| pair[1] <= pair[0]) {
            return None;
        }
        let mut integral = Vec::with_capacity(axis.len());
        integral.push(0.0);
        let mut sum = 0.0;
        let mut compensation = 0.0;
        // Remove the common level before accumulating, preserving flat curves
        // exactly and reducing cancellation when subtracting nearby prefixes.
        for index in 0..axis.len() - 1 {
            let area = 0.5
                * ((values[index] - values[0]) + (values[index + 1] - values[0]))
                * (axis[index + 1] - axis[index]);
            let corrected = area - compensation;
            let next = sum + corrected;
            compensation = (next - sum) - corrected;
            sum = next;
            integral.push(sum);
        }
        Some(Self {
            axis,
            values,
            integral,
        })
    }

    fn at(&self, x: f64) -> f64 {
        let index = self
            .axis
            .partition_point(|value| *value <= x)
            .saturating_sub(1)
            .min(self.axis.len() - 2);
        let width = x - self.axis[index];
        let start = self.values[index] - self.values[0];
        let slope = (self.values[index + 1] - self.values[index])
            / (self.axis[index + 1] - self.axis[index]);
        self.integral[index] + width * (start + 0.5 * slope * width)
    }

    fn mean(&self, lower: f64, upper: f64) -> Option<f64> {
        let lower = lower.ln().max(self.axis[0]);
        let upper = upper.ln().min(self.axis[self.axis.len() - 1]);
        (upper > lower)
            .then(|| self.values[0] + (self.at(upper) - self.at(lower)) / (upper - lower))
    }
}

/// Interpolate all measured curve fields in logarithmic frequency space.
pub fn interpolate_log_space(output_frequencies: &Array1<f64>, curve: &Curve) -> Curve {
    debug_assert!(
        curve
            .freq
            .as_slice()
            .is_none_or(|frequencies| frequencies.windows(2).all(|pair| pair[0] <= pair[1])),
        "interpolate_log_space() requires sorted frequencies"
    );
    let needs_dc_axis = curve
        .freq
        .first()
        .is_some_and(|frequency| *frequency <= 0.0)
        || output_frequencies.iter().any(|frequency| *frequency <= 0.0);
    let first_positive = curve
        .freq
        .iter()
        .copied()
        .find(|frequency| *frequency > 0.0);
    let transform = |frequency: f64| match (needs_dc_axis, first_positive) {
        (true, Some(pivot)) if frequency <= pivot => frequency / pivot,
        (true, Some(pivot)) => 1.0 + (frequency / pivot).ln(),
        (true, None) => frequency,
        (false, _) => frequency.ln(),
    };
    let input_axis: Vec<_> = curve.freq.iter().copied().map(transform).collect();
    let output_axis: Vec<_> = output_frequencies.iter().copied().map(transform).collect();
    let spl = interpolate_log_space_values(&output_axis, &input_axis, &curve.spl);
    let phase = curve.phase.as_ref().map(|phase| {
        interpolate_log_space_values(&output_axis, &input_axis, &unwrap_phase_degrees(phase))
    });
    let coherence = curve
        .coherence
        .as_ref()
        .map(|values| interpolate_log_space_values(&output_axis, &input_axis, values));
    let noise_floor_db = curve
        .noise_floor_db
        .as_ref()
        .map(|values| interpolate_log_space_values(&output_axis, &input_axis, values));
    Curve {
        freq: output_frequencies.clone(),
        spl,
        phase,
        coherence,
        noise_floor_db,
        resolution: Some(curve.retained_resolution()),
        ..Default::default()
    }
}

/// Create a standard base-10 logarithmic frequency grid.
pub fn create_log_frequency_grid(
    point_count: usize,
    minimum_frequency: f64,
    maximum_frequency: f64,
) -> Array1<f64> {
    Array1::logspace(
        10.0,
        minimum_frequency.log10(),
        maximum_frequency.log10(),
        point_count,
    )
}

/// Interpolate SPL and unwrapped phase on a linear frequency axis.
pub fn interpolate(output_frequencies: &Array1<f64>, curve: &Curve) -> Curve {
    debug_assert!(
        curve
            .freq
            .as_slice()
            .is_some_and(|frequencies| frequencies.windows(2).all(|pair| pair[0] <= pair[1])),
        "interpolate() requires sorted frequencies"
    );
    let mut spl = Array1::zeros(output_frequencies.len());
    let unwrapped_phase = curve.phase.as_ref().map(unwrap_phase_degrees);
    let mut phase = unwrapped_phase
        .as_ref()
        .map(|_| Array1::zeros(output_frequencies.len()));
    if curve.freq.is_empty() {
        return Curve {
            freq: output_frequencies.clone(),
            spl,
            phase,
            resolution: Some(curve.retained_resolution()),
            ..Default::default()
        };
    }

    for (index, &target) in output_frequencies.iter().enumerate() {
        let (left, right, fraction) = if target <= curve.freq[0] {
            (0, 0, 0.0)
        } else if target >= curve.freq[curve.freq.len() - 1] {
            let last = curve.freq.len() - 1;
            (last, last, 0.0)
        } else {
            let right = curve
                .freq
                .as_slice()
                .expect("Array1 frequency grids are contiguous")
                .partition_point(|&frequency| frequency < target);
            let left = right - 1;
            let fraction = (target - curve.freq[left]) / (curve.freq[right] - curve.freq[left]);
            (left, right, fraction)
        };
        spl[index] = curve.spl[left] + fraction * (curve.spl[right] - curve.spl[left]);
        if let (Some(output), Some(input)) = (phase.as_mut(), unwrapped_phase.as_ref()) {
            output[index] = input[left] + fraction * (input[right] - input[left]);
        }
    }
    Curve {
        freq: output_frequencies.clone(),
        spl,
        phase,
        resolution: Some(curve.retained_resolution()),
        ..Default::default()
    }
}

/// Subtract the log-frequency-weighted mean over the requested band.
pub fn normalize_response(
    curve: &Curve,
    minimum_frequency: f64,
    maximum_frequency: f64,
) -> Array1<f64> {
    mean_over_log_frequency(
        &curve.freq,
        &curve.spl,
        minimum_frequency,
        maximum_frequency,
    )
    .map_or_else(|| curve.spl.clone(), |mean| curve.spl.clone() - mean)
}

/// Interpolate and normalize over the standard 1–2 kHz band.
pub fn normalize_and_interpolate_response(
    output_frequencies: &Array1<f64>,
    curve: &Curve,
) -> Curve {
    normalize_and_interpolate_response_with_range(
        output_frequencies,
        curve,
        NORMALIZE_LOW_FREQ,
        NORMALIZE_HIGH_FREQ,
    )
}

/// Interpolate without changing absolute SPL levels.
pub fn interpolate_response(output_frequencies: &Array1<f64>, curve: &Curve) -> Curve {
    interpolate_log_space(output_frequencies, curve)
}

/// Interpolate in log-frequency space without inventing data outside the
/// measured band: interior points use log-frequency interpolation, while
/// points below/above the measured range hold the nearest measured edge
/// value (magnitude and phase alike).
///
/// Crossover summation of band-limited drivers (e.g. a subwoofer measured to
/// 200 Hz summed with a full-range main) must not slope-extrapolate the
/// band-limited driver: a single noisy edge bin implies an arbitrary
/// dB-per-octave trend that dominates the sum far outside the measurement
/// (hundreds of dB at 20 kHz) and poisons target references derived from it.
pub fn interpolate_log_space_hold_edges(output_frequencies: &Array1<f64>, curve: &Curve) -> Curve {
    let mut interpolated = interpolate_log_space(output_frequencies, curve);
    let Some(&first_freq) = curve.freq.first() else {
        return interpolated;
    };
    let Some(&last_freq) = curve.freq.last() else {
        return interpolated;
    };
    if curve.spl.is_empty() {
        return interpolated;
    }
    let first_spl = curve.spl[0];
    let last_spl = curve.spl[curve.spl.len() - 1];
    let first_phase = curve
        .phase
        .as_ref()
        .and_then(|phase| phase.first().copied());
    let last_phase = curve.phase.as_ref().and_then(|phase| phase.last().copied());
    let first_coherence = curve
        .coherence
        .as_ref()
        .and_then(|values| values.first().copied());
    let last_coherence = curve
        .coherence
        .as_ref()
        .and_then(|values| values.last().copied());
    let first_noise = curve
        .noise_floor_db
        .as_ref()
        .and_then(|values| values.first().copied());
    let last_noise = curve
        .noise_floor_db
        .as_ref()
        .and_then(|values| values.last().copied());
    for (index, &frequency) in output_frequencies.iter().enumerate() {
        if frequency < first_freq {
            interpolated.spl[index] = first_spl;
            if let (Some(phase), Some(edge)) = (interpolated.phase.as_mut(), first_phase) {
                phase[index] = edge;
            }
            if let (Some(values), Some(edge)) = (interpolated.coherence.as_mut(), first_coherence) {
                values[index] = edge;
            }
            if let (Some(values), Some(edge)) = (interpolated.noise_floor_db.as_mut(), first_noise)
            {
                values[index] = edge;
            }
        } else if frequency > last_freq {
            interpolated.spl[index] = last_spl;
            if let (Some(phase), Some(edge)) = (interpolated.phase.as_mut(), last_phase) {
                phase[index] = edge;
            }
            if let (Some(values), Some(edge)) = (interpolated.coherence.as_mut(), last_coherence) {
                values[index] = edge;
            }
            if let (Some(values), Some(edge)) = (interpolated.noise_floor_db.as_mut(), last_noise) {
                values[index] = edge;
            }
        }
    }
    interpolated
}

/// Interpolate without changing absolute SPL levels and without inventing
/// data outside the measured band (see [`interpolate_log_space_hold_edges`]).
pub fn interpolate_response_hold_edges(output_frequencies: &Array1<f64>, curve: &Curve) -> Curve {
    interpolate_log_space_hold_edges(output_frequencies, curve)
}

/// Interpolate and normalize over a caller-selected frequency band.
pub fn normalize_and_interpolate_response_with_range(
    output_frequencies: &Array1<f64>,
    curve: &Curve,
    minimum_frequency: f64,
    maximum_frequency: f64,
) -> Curve {
    let mut interpolated = interpolate_log_space(output_frequencies, curve);
    interpolated.spl = normalize_response(&interpolated, minimum_frequency, maximum_frequency);
    interpolated
}

/// Configuration for frequency-dependent psychoacoustic smoothing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PsychoacousticSmoothingConfig {
    pub low_freq_n: usize,
    pub high_freq_n: usize,
    pub low_freq: f64,
    pub high_freq: f64,
}

impl Default for PsychoacousticSmoothingConfig {
    fn default() -> Self {
        Self {
            low_freq_n: 48,
            high_freq_n: 6,
            low_freq: 100.0,
            high_freq: 1000.0,
        }
    }
}

#[doc(hidden)]
pub fn calculate_variable_n(frequency: f64, config: &PsychoacousticSmoothingConfig) -> f64 {
    if frequency <= config.low_freq {
        config.low_freq_n as f64
    } else if frequency >= config.high_freq {
        config.high_freq_n as f64
    } else {
        let fraction = (frequency.ln() - config.low_freq.ln())
            / (config.high_freq.ln() - config.low_freq.ln());
        ((config.low_freq_n as f64).ln()
            + fraction * ((config.high_freq_n as f64).ln() - (config.low_freq_n as f64).ln()))
        .exp()
    }
}

/// Apply frequency-dependent psychoacoustic smoothing.
pub fn smooth_psychoacoustic(curve: &Curve, config: &PsychoacousticSmoothingConfig) -> Curve {
    let mut spl = Array1::zeros(curve.spl.len());
    let integral = LogFrequencyIntegral::new(&curve.freq, &curve.spl);
    for index in 0..curve.freq.len() {
        let frequency = curve.freq[index].max(1e-12);
        let bands_per_octave = calculate_variable_n(frequency, config);
        let half_window = 2.0_f64.powf(1.0 / (2.0 * bands_per_octave));
        let lower = frequency / half_window;
        let upper = frequency * half_window;
        spl[index] = match &integral {
            Some(integral) => integral.mean(lower, upper),
            None => mean_over_log_frequency(&curve.freq, &curve.spl, lower, upper),
        }
        .unwrap_or(curve.spl[index]);
    }
    Curve {
        freq: curve.freq.clone(),
        spl,
        phase: curve.phase.clone(),
        coherence: curve.coherence.clone(),
        noise_floor_db: curve.noise_floor_db.clone(),
        resolution: Some(
            curve
                .retained_resolution()
                .with_smoothing(
                    curve
                        .freq
                        .iter()
                        .copied()
                        .filter(|f| *f > 0.0)
                        .map(|f| (f, 1.0 / calculate_variable_n(f, config)))
                        .collect(),
                )
                .unwrap_or_else(|| {
                    std::sync::Arc::new(crate::resolution::MeasurementResolution::unsupported())
                }),
        ),
        ..Default::default()
    }
}

/// Apply simple 1/N-octave smoothing.
pub fn smooth_one_over_n_octave(curve: &Curve, bands_per_octave: usize) -> Curve {
    let bands_per_octave = bands_per_octave.max(1);
    let half_window = 2.0_f64.powf(1.0 / (2.0 * bands_per_octave as f64));
    let mut spl = Array1::zeros(curve.spl.len());
    let integral = LogFrequencyIntegral::new(&curve.freq, &curve.spl);
    for index in 0..curve.freq.len() {
        let frequency = curve.freq[index].max(1e-12);
        let lower = frequency / half_window;
        let upper = frequency * half_window;
        spl[index] = match &integral {
            Some(integral) => integral.mean(lower, upper),
            None => mean_over_log_frequency(&curve.freq, &curve.spl, lower, upper),
        }
        .unwrap_or(curve.spl[index]);
    }
    Curve {
        freq: curve.freq.clone(),
        spl,
        phase: curve.phase.clone(),
        coherence: curve.coherence.clone(),
        noise_floor_db: curve.noise_floor_db.clone(),
        resolution: Some(
            curve
                .retained_resolution()
                .with_smoothing(
                    curve
                        .freq
                        .iter()
                        .copied()
                        .filter(|f| *f > 0.0)
                        .map(|f| (f, 1.0 / bands_per_octave as f64))
                        .collect(),
                )
                .unwrap_or_else(|| {
                    std::sync::Arc::new(crate::resolution::MeasurementResolution::unsupported())
                }),
        ),
        ..Default::default()
    }
}

/// Broad log-frequency magnitude features on retained measurement support.
///
/// Raised-cosine functions have FWHM and spacing at least the widest original
/// sampling/smoothing floor in the active band. Their full supports fit inside
/// that band, so boundaries cannot truncate a feature into a narrow transition.
/// This constrains magnitude design, not acquisition accuracy or finite-tap ripple.
#[derive(Debug, Clone)]
pub struct ResolutionMagnitudeBasis {
    width_octaves: f64,
    coefficients: Vec<(f64, f64)>,
}

impl ResolutionMagnitudeBasis {
    /// Project a magnitude proposal onto full-width supported features.
    ///
    /// Uses log-frequency integral means over complete supports. Bands narrower
    /// than one complete function stay neutral; interpolation cannot enlarge
    /// source support. Original measurement magnitude and phase remain unchanged.
    ///
    /// # Errors
    ///
    /// Rejects invalid arrays, bandwidth, band, or unsupported active samples.
    pub fn new(
        source: &Curve,
        correction_db: &Array1<f64>,
        additional_octaves: f64,
        band_hz: [f64; 2],
    ) -> Result<Self, String> {
        if source.freq.len() != correction_db.len()
            || source.freq.len() < 2
            || !additional_octaves.is_finite()
            || additional_octaves < 0.0
            || !band_hz[0].is_finite()
            || !band_hz[1].is_finite()
            || band_hz[0] <= 0.0
            || band_hz[1] <= band_hz[0]
            || correction_db.iter().any(|value| !value.is_finite())
            || source
                .freq
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
            || source
                .freq
                .windows(2)
                .into_iter()
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(String::from(
                "invalid resolution-constrained correction inputs",
            ));
        }
        let first = source
            .freq
            .iter()
            .position(|frequency| *frequency > 0.0)
            .ok_or("resolution-constrained correction has no positive frequencies")?;
        let frequencies = Array1::from_iter(source.freq.iter().skip(first).copied());
        let values = Array1::from_iter(correction_db.iter().skip(first).copied());
        let integral = LogFrequencyIntegral::new(&frequencies, &values)
            .ok_or("resolution-constrained correction needs two positive samples")?;
        let support = source.retained_resolution();
        let supported = support
            .support_band_hz()
            .ok_or("missing original correction support")?;
        // Fail closed for extrapolated active bins, even when a subset of the
        // requested band intersects valid original support.
        for frequency in source
            .freq
            .iter()
            .copied()
            .filter(|frequency| (band_hz[0]..=band_hz[1]).contains(frequency))
        {
            support
                .width_octaves(frequency)
                .ok_or_else(|| format!("no original resolution support at {frequency:.3} Hz"))?;
        }
        let low = band_hz[0].max(supported[0]).max(frequencies[0]);
        let high = band_hz[1]
            .min(supported[1])
            .min(frequencies[frequencies.len() - 1]);
        if low >= high {
            return Err(String::from(
                "correction band has no original resolution support",
            ));
        }
        let width = support
            .max_width_octaves([low, high])
            .ok_or("correction band has no finite original feature width")?
            .max(additional_octaves);
        let mut basis = Self {
            width_octaves: width,
            coefficients: Vec::new(),
        };
        if high.log2() - low.log2() < 2.0 * width {
            return Ok(basis);
        }
        let mut center = low.log2() + width;
        while center + width <= high.log2() {
            let gain = integral
                .mean(2.0_f64.powf(center - width), 2.0_f64.powf(center + width))
                .ok_or("resolution-constrained correction has an empty full-width support")?;
            if !gain.is_finite() {
                return Err(String::from(
                    "nonfinite resolution-constrained correction coefficient",
                ));
            }
            basis.coefficients.push((center, gain));
            let next = center + width;
            if next <= center {
                return Err(String::from(
                    "resolution feature lattice cannot advance numerically",
                ));
            }
            center = next;
        }
        Ok(basis)
    }

    /// Cap positive coefficients over their complete supports.
    ///
    /// The supplied nonnegative envelope is linear in log frequency. Its
    /// minimum over every overlapping feature bounds that entire coefficient;
    /// cuts stay unchanged. No pointwise clipping can create a narrow feature.
    ///
    /// # Errors
    ///
    /// Rejects invalid envelopes or envelopes not covering the complete basis.
    pub fn cap_positive_support(
        &mut self,
        frequencies: &Array1<f64>,
        upper_db: &Array1<f64>,
    ) -> Result<(), String> {
        if frequencies.len() != upper_db.len()
            || frequencies.len() < 2
            || frequencies.iter().any(|f| !f.is_finite() || *f < 0.0)
            || frequencies
                .windows(2)
                .into_iter()
                .any(|pair| pair[0] >= pair[1])
            || upper_db.iter().any(|cap| !cap.is_finite() || *cap < 0.0)
        {
            return Err("invalid broad magnitude upper envelope".into());
        }
        let first = frequencies
            .iter()
            .position(|f| *f > 0.0)
            .ok_or("broad magnitude envelope has no positive support")?;
        let grid: Vec<_> = frequencies.iter().skip(first).map(|f| f.log2()).collect();
        if grid.len() < 2 {
            return Err("broad magnitude envelope needs two positive samples".into());
        }
        let at = |logarithm: f64| {
            let logarithm = logarithm.clamp(grid[0], grid[grid.len() - 1]);
            let right = grid
                .partition_point(|value| *value < logarithm)
                .clamp(1, grid.len() - 1);
            let fraction = (logarithm - grid[right - 1]) / (grid[right] - grid[right - 1]);
            // Convex interpolation avoids subtracting near-f64::MAX caps.
            (1.0 - fraction) * upper_db[first + right - 1] + fraction * upper_db[first + right]
        };
        for &(center, _) in &self.coefficients {
            if center - self.width_octaves < grid[0] - 1e-12
                || center + self.width_octaves > grid[grid.len() - 1] + 1e-12
            {
                return Err("broad magnitude envelope does not cover feature support".into());
            }
        }
        for (center, gain) in &mut self.coefficients {
            if *gain <= 0.0 {
                continue;
            }
            let low = *center - self.width_octaves;
            let high = *center + self.width_octaves;
            let begin = grid.partition_point(|value| *value < low);
            let end = grid.partition_point(|value| *value <= high);
            let cap = upper_db
                .iter()
                .skip(first + begin)
                .take(end - begin)
                .copied()
                .fold(at(low).min(at(high)), f64::min);
            *gain = gain.min(cap);
        }
        Ok(())
    }

    /// Evaluate the analytic magnitude basis at one frequency.
    ///
    /// Outside complete feature supports, or for invalid frequencies, returns
    /// neutral correction. At most two lattice neighbours overlap. Sampling this
    /// basis on an FFT grid does not introduce new optimization parameters.
    pub fn at(&self, frequency: f64) -> f64 {
        if !frequency.is_finite() || frequency <= 0.0 {
            return 0.0;
        }
        let logarithm = frequency.log2();
        let right = self
            .coefficients
            .partition_point(|(center, _)| *center <= logarithm);
        self.coefficients[right.saturating_sub(1)..(right + 1).min(self.coefficients.len())]
            .iter()
            .map(|&(center, gain)| {
                let distance = (logarithm - center) / self.width_octaves;
                if distance.abs() < 1.0 {
                    gain * 0.5 * (1.0 + (std::f64::consts::PI * distance).cos())
                } else {
                    0.0
                }
            })
            .sum()
    }

    /// Evaluate the analytic basis on a supplied numerical grid.
    pub fn evaluate(&self, frequencies: &Array1<f64>) -> Array1<f64> {
        frequencies.mapv(|frequency| self.at(frequency))
    }
}

/// Project correction magnitudes onto full-width retained-support features.
///
/// Returns neutral samples outside complete basis supports, including band
/// boundaries. Original magnitude and phase remain unchanged. This is a design
/// constraint, not a hearing or acquisition assessment.
///
/// # Errors
///
/// Returns the refusal from [`ResolutionMagnitudeBasis::new`].
pub fn smooth_correction_to_resolution(
    source: &Curve,
    correction_db: &Array1<f64>,
    additional_octaves: f64,
    band_hz: [f64; 2],
) -> Result<Array1<f64>, String> {
    Ok(
        ResolutionMagnitudeBasis::new(source, correction_db, additional_octaves, band_hz)?
            .evaluate(&source.freq),
    )
}

/// Apply Gaussian smoothing to a scalar signal.
pub fn smooth_gaussian(signal: &Array1<f64>, sigma: f64) -> Array1<f64> {
    if sigma <= 0.0 {
        return signal.clone();
    }
    let half_size = (3.0 * sigma).ceil() as usize;
    let kernel_size = 2 * half_size + 1;
    let mut kernel: Vec<_> = (0..kernel_size)
        .map(|index| {
            let x = index as f64 - half_size as f64;
            (-0.5 * (x / sigma).powi(2)).exp()
        })
        .collect();
    let kernel_sum: f64 = kernel.iter().sum();
    for weight in &mut kernel {
        *weight /= kernel_sum;
    }
    let mut output = Array1::zeros(signal.len());
    for index in 0..signal.len() {
        let mut weighted_sum = 0.0;
        let mut weight_sum = 0.0;
        for (kernel_index, &weight) in kernel.iter().enumerate() {
            let sample_index = index as isize + kernel_index as isize - half_size as isize;
            if (0..signal.len() as isize).contains(&sample_index) {
                weighted_sum += signal[sample_index as usize] * weight;
                weight_sum += weight;
            }
        }
        output[index] = if weight_sum > 0.0 {
            weighted_sum / weight_sum
        } else {
            signal[index]
        };
    }
    output
}

/// Compute the least-squares SPL slope in dB per octave over a frequency band.
pub fn regression_slope_per_octave_in_range(
    frequencies: &Array1<f64>,
    values: &Array1<f64>,
    minimum_frequency: f64,
    maximum_frequency: f64,
) -> Option<f64> {
    assert_eq!(frequencies.len(), values.len());
    if maximum_frequency <= minimum_frequency {
        return None;
    }
    let mut count = 0_usize;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_x_squared = 0.0;
    for (&frequency, &value) in frequencies.iter().zip(values) {
        if frequency > 0.0 && frequency >= minimum_frequency && frequency <= maximum_frequency {
            let x = frequency.log2();
            count += 1;
            sum_x += x;
            sum_y += value;
            sum_xy += x * value;
            sum_x_squared += x * x;
        }
    }
    if count < 2 {
        return None;
    }
    let count = count as f64;
    let covariance = sum_xy - (sum_x * sum_y) / count;
    let variance = sum_x_squared - (sum_x * sum_x) / count;
    (variance.abs() >= 1e-10).then_some(covariance / variance)
}

/// Compute the least-squares SPL slope of a curve in dB per octave.
pub fn curve_slope_per_octave_in_range(
    curve: &Curve,
    minimum_frequency: f64,
    maximum_frequency: f64,
) -> Option<f64> {
    regression_slope_per_octave_in_range(
        &curve.freq,
        &curve.spl,
        minimum_frequency,
        maximum_frequency,
    )
}

/// Build one of the historical AutoEQ predefined targets on a prepared grid.
pub fn build_target_curve_by_name(
    curve_name: &str,
    frequencies: &Array1<f64>,
    input_curve: &Curve,
) -> Curve {
    let spl = match curve_name {
        "flat" => Array1::zeros(frequencies.len()),
        "harman" => Array1::from_shape_fn(frequencies.len(), |index| {
            -0.8 * (frequencies[index].max(1e-12) / 1000.0).log2()
        }),
        "Listening Window" => {
            let log_minimum = 1000.0_f64.log10();
            let log_maximum = 20000.0_f64.log10();
            Array1::from_shape_fn(frequencies.len(), |index| {
                let log_frequency = frequencies[index].max(1e-12).log10();
                if log_frequency < log_minimum {
                    0.0
                } else if log_frequency >= log_maximum {
                    -0.5
                } else {
                    -0.5 * (log_frequency - log_minimum) / (log_maximum - log_minimum)
                }
            })
        }
        "Sound Power" | "Early Reflections" | "Estimated In-Room Response" => {
            let slope =
                curve_slope_per_octave_in_range(input_curve, 100.0, 10000.0).unwrap_or(-1.2) - 0.2;
            let minimum = 100.0_f64;
            let maximum = 20000.0_f64;
            let maximum_value = slope * (maximum / minimum).log2();
            Array1::from_shape_fn(frequencies.len(), |index| {
                let frequency = frequencies[index].max(1e-12);
                if frequency < minimum {
                    0.0
                } else if frequency >= maximum {
                    maximum_value
                } else {
                    slope * (frequency / minimum).log2()
                }
            })
        }
        _ => Array1::zeros(frequencies.len()),
    };
    Curve {
        freq: frequencies.clone(),
        spl,
        phase: None,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(frequencies: Vec<f64>, spl: Vec<f64>) -> Curve {
        Curve {
            freq: Array1::from_vec(frequencies),
            spl: Array1::from_vec(spl),
            ..Default::default()
        }
    }

    #[test]
    fn log_interpolation_handles_dc_and_unwraps_phase() {
        let mut input = curve(vec![0.0, 100.0], vec![0.0, 100.0]);
        input.phase = Some(Array1::from_vec(vec![170.0, -170.0]));
        let output = interpolate_log_space(&Array1::from_vec(vec![0.0, 50.0, 100.0]), &input);
        assert_eq!(output.spl.to_vec(), vec![0.0, 50.0, 100.0]);
        assert_eq!(output.phase.unwrap().to_vec(), vec![170.0, 180.0, 190.0]);
    }

    #[test]
    fn normalization_is_invariant_to_samples_on_log_linear_segments() {
        let sparse = curve(vec![100.0, 200.0, 400.0], vec![0.0, 10.0, 0.0]);
        let dense_frequencies = vec![100.0, 150.0, 200.0, 300.0, 400.0];
        let dense_values = dense_frequencies
            .iter()
            .map(|&frequency| {
                if frequency <= 200.0 {
                    10.0 * (frequency / 100.0_f64).ln() / 2.0_f64.ln()
                } else {
                    10.0 * (1.0 - (frequency / 200.0_f64).ln() / 2.0_f64.ln())
                }
            })
            .collect();
        let dense = curve(dense_frequencies, dense_values);
        let sparse_mean = mean_over_log_frequency(&sparse.freq, &sparse.spl, 100.0, 400.0).unwrap();
        let dense_mean = mean_over_log_frequency(&dense.freq, &dense.spl, 100.0, 400.0).unwrap();
        assert!((sparse_mean - dense_mean).abs() < 1e-12);
    }

    #[test]
    fn magnitude_smoothing_preserves_measured_metadata() {
        let mut input = curve(vec![100.0, 200.0, 400.0], vec![0.0, 10.0, 0.0]);
        input.phase = Some(Array1::from_vec(vec![10.0, 20.0, 30.0]));
        input.coherence = Some(Array1::from_vec(vec![0.8, 0.9, 0.95]));
        input.min_phase = Some(Array1::from_vec(vec![1.0, 2.0, 3.0]));
        for output in [
            smooth_one_over_n_octave(&input, 1),
            smooth_psychoacoustic(&input, &PsychoacousticSmoothingConfig::default()),
        ] {
            assert_eq!(output.phase, input.phase);
            assert_eq!(output.coherence, input.coherence);
            assert!(output.min_phase.is_none());
        }
    }
}

#[cfg(test)]
mod magnitude_envelope_tests {
    use super::*;

    #[test]
    fn positive_support_caps_preserve_cuts_and_broad_feature_widths() {
        let mut basis = ResolutionMagnitudeBasis {
            width_octaves: 0.5,
            coefficients: vec![(8.0, 6.0), (9.0, -4.0), (10.0, 6.0)],
        };
        let frequencies = Array1::from_iter((0..=32).map(|i| 2.0_f64.powf(7.0 + i as f64 / 8.0)));
        let caps = frequencies.mapv(|f| if f == 256.0 { 0.0 } else { 3.5 });
        basis.cap_positive_support(&frequencies, &caps).unwrap();
        assert_eq!(basis.width_octaves, 0.5);
        assert_eq!(
            basis.coefficients,
            vec![(8.0, 0.0), (9.0, -4.0), (10.0, 3.5)]
        );
        assert_eq!(basis.at(256.0), 0.0);
        assert_eq!(basis.at(512.0), -4.0);
        assert_eq!(basis.at(1024.0), 3.5);
        for i in 0..16384 {
            let frequency = 128.0 * 16.0_f64.powf(i as f64 / 16383.0);
            assert!(basis.at(frequency) <= 3.5 + 1e-12);
        }
    }

    #[test]
    fn envelope_support_boundaries_are_checked_between_knots() {
        let mut basis = ResolutionMagnitudeBasis {
            width_octaves: 0.5,
            coefficients: vec![(8.5, 10.0)],
        };
        let frequencies = Array1::from_vec(vec![128.0, 300.0, 600.0, 1024.0]);
        let caps = Array1::from_vec(vec![0.0, 8.0, 8.0, 0.0]);
        basis.cap_positive_support(&frequencies, &caps).unwrap();
        let expected = (8.0 * (256.0_f64 / 128.0).ln() / (300.0_f64 / 128.0).ln())
            .min(8.0 * (1024.0_f64 / 512.0).ln() / (1024.0_f64 / 600.0).ln());
        assert!((basis.coefficients[0].1 - expected).abs() < 1e-12);
        let previous = basis.coefficients.clone();
        assert!(
            basis
                .cap_positive_support(
                    &Array1::from_vec(vec![300.0, 600.0]),
                    &Array1::from_vec(vec![3.0, 3.0])
                )
                .is_err()
        );
        assert_eq!(basis.coefficients, previous);
    }
}
