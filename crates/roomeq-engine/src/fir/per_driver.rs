//! Conservative joint, finite-tap correction of calibrated physical branches.
//!
//! Never invert an individual branch towards the full-range acoustic target.
//! Room-null masks constrain electrical boost, not the acoustic benefit of
//! changing relative branch phase. All branches receive identical causal support.
use crate::Curve;
use ndarray::Array1;
use num_complex::Complex64;
use roomeq_model::{OptimizerConfig, ProcessingMode};
use rustfft::FftPlanner;

#[derive(Debug)]
pub struct PerDriverFir {
    pub coefficients: Vec<Vec<f64>>,
    pub final_curve: Curve,
    pub latency_samples: usize,
    pub protected_bins: usize,
    pub before_rms_db: f64,
    pub after_rms_db: f64,
    /// Selected scale of the assessed phase proposal; zero means none was selected.
    ///
    /// This is a search result, not a claim that every branch or bin changed.
    pub selected_phase_strength: f64,
}

/// Conservative local notch / unreliable-data detector. The half-octave
/// neighbourhood is deliberately wider than the correction smoothing. A single
/// capture cannot prove SBIR; treating ambiguous dips as unboostable is safer.
pub fn fir_null_mask(curve: &Curve) -> Vec<bool> {
    curve
        .freq
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            let envelope = curve
                .freq
                .iter()
                .zip(&curve.spl)
                .filter(|(other, _)| (**other / f).log2().abs() <= 0.5)
                .map(|(_, &spl)| spl)
                .fold(curve.spl[i], f64::max);
            curve.spl[i] < envelope - 6.0
                || curve
                    .coherence
                    .as_ref()
                    .is_some_and(|c| !c[i].is_finite() || c[i] < 0.8)
                || curve
                    .noise_floor_db
                    .as_ref()
                    .is_some_and(|n| curve.spl[i] - n[i] < 10.0)
        })
        .collect()
}

fn complex(curve: &Curve, i: usize) -> Complex64 {
    Complex64::from_polar(
        10.0_f64.powf(curve.spl[i] / 20.0),
        curve.phase.as_ref().map_or(0.0, |p| p[i].to_radians()),
    )
}

fn sum(branches: &[Curve], taps: &[Vec<f64>], fs: f64) -> Curve {
    let freq = &branches[0].freq;
    let mut response = vec![Complex64::new(0.0, 0.0); freq.len()];
    for (branch, coefficients) in branches.iter().zip(taps) {
        let h = crate::response::compute_fir_complex_response(coefficients, freq, fs);
        for (i, v) in h.iter().enumerate() {
            response[i] += complex(branch, i) * v;
        }
    }
    Curve {
        freq: freq.clone(),
        spl: Array1::from_iter(response.iter().map(|v| 20.0 * v.norm().max(1e-15).log10())),
        phase: Some(Array1::from_iter(
            response.iter().map(|v| v.arg().to_degrees()),
        )),
        resolution: autoeq_core::resolution::MeasurementResolution::combine(branches.iter()),
        ..Curve::default()
    }
}

/// Distinguishes invalid phase inputs from insufficient evidence for correction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerDriverPhaseError {
    /// Invalid captures, frequency bounds, sample rate, or assessment policy.
    InvalidInput(String),
    /// Valid inputs whose evidence does not support excess-phase correction.
    Unsupported(String),
}

impl std::fmt::Display for PerDriverPhaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(reason) | Self::Unsupported(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for PerDriverPhaseError {}

/// Assess one physical capture before designing its excess-phase correction.
///
/// Returns the assessment-derived correction in degrees on the capture grid.
/// This numerical check does not establish shared capture timing or direct-sound
/// support; callers must enforce those independently before deploying filters.
///
/// # Errors
///
/// Returns [`PerDriverPhaseError::InvalidInput`] for malformed captures, sample
/// rates, bands, or policy, and [`PerDriverPhaseError::Unsupported`] when valid
/// input lacks sufficient phase, SNR, or window-consistency evidence.
pub fn assess_per_driver_phase_target(
    curve: &Curve,
    policy: &roomeq_model::PhaseAssessmentConfig,
    band: [f64; 2],
    fs: f64,
) -> Result<Vec<f64>, PerDriverPhaseError> {
    use roomeq_analysis::excess_phase::{
        Assessment, ExcessPhaseConfig, ExcessPhaseInput, assess_excess_phase,
    };
    curve
        .validate("per-driver phase capture")
        .map_err(|error| PerDriverPhaseError::InvalidInput(error.to_string()))?;
    if !fs.is_finite()
        || fs <= 0.0
        || !band.iter().all(|value| value.is_finite())
        || band[0] <= 0.0
        || band[1] <= band[0]
        || band[1] >= fs / 2.0
        || curve.freq.iter().any(|frequency| *frequency > fs / 2.0)
    {
        return Err(PerDriverPhaseError::InvalidInput(
            "invalid per-driver phase sample rate or band".into(),
        ));
    }
    if ![
        policy.taper_oct,
        policy.snr_floor_db,
        policy.min_valid_fraction,
        policy.smooth_narrow_oct,
        policy.smooth_wide_oct,
        policy.consistency_tol_ms,
        policy.dip_depth_db,
    ]
    .iter()
    .all(|value| value.is_finite())
        || policy.taper_oct <= 0.0
        || !(0.0..=1.0).contains(&policy.min_valid_fraction)
        || policy.min_valid_fraction == 0.0
        || policy.smooth_narrow_oct <= 0.0
        || policy.smooth_wide_oct < policy.smooth_narrow_oct
        || policy.consistency_tol_ms < 0.0
        || policy.dip_depth_db <= 0.0
    {
        return Err(PerDriverPhaseError::InvalidInput(
            "invalid assessment policy".into(),
        ));
    }
    let input = ExcessPhaseInput {
        freqs_hz: curve.freq.to_vec(),
        magnitude_db: curve.spl.to_vec(),
        phase_deg: curve.phase.as_ref().map(|phase| phase.to_vec()),
        snr_db: curve
            .spl
            .iter()
            .enumerate()
            .map(|(i, level)| {
                // Match the existing FIR coherence guard. Missing coherence is
                // not synthesized; measured poor coherence excludes the bin.
                if curve
                    .coherence
                    .as_ref()
                    .is_some_and(|values| values[i] < 0.8)
                {
                    return f64::NEG_INFINITY;
                }
                curve
                    .noise_floor_db
                    .as_ref()
                    .map_or(f64::NEG_INFINITY, |floor| {
                        let snr = level - floor[i];
                        if snr.is_finite() {
                            snr
                        } else {
                            f64::NEG_INFINITY
                        }
                    })
            })
            .collect(),
        sample_rate_hz: fs,
    };
    let config = ExcessPhaseConfig {
        taper_oct: policy.taper_oct,
        snr_floor_db: policy.snr_floor_db,
        min_valid_fraction: policy.min_valid_fraction,
        smooth_narrow_oct: policy.smooth_narrow_oct,
        smooth_wide_oct: policy.smooth_wide_oct,
        consistency_tol_ms: policy.consistency_tol_ms,
        strict_dips: policy.strict_dips,
        dip_depth_db: policy.dip_depth_db,
        analysis_band_hz: (band[0], band[1]),
    };
    match assess_excess_phase(&input, &config) {
        Assessment::Supported(report) => Ok(report
            .correction_phase_rad
            .iter()
            .map(|phase| phase.to_degrees())
            .collect()),
        Assessment::Unknown { reason, .. } | Assessment::Unsupported { reason } => {
            Err(PerDriverPhaseError::Unsupported(reason))
        }
    }
}

fn band_weight(f: f64, lo: f64, hi: f64) -> f64 {
    if f <= lo || f >= hi {
        return 0.0;
    }
    // Transitions are INSIDE the configured correction band.
    ((f / lo).log2() / 0.25)
        .min((hi / f).log2() / 0.25)
        .clamp(0.0, 1.0)
}

// Raised-cosine functions spaced by their FWHM form a nonnegative partition
// with total weight at most one. Bound each coefficient over its entire support
// rather than clipping individual bins after conditioning.
#[derive(Debug)]
struct MagnitudeBasis {
    width_octaves: f64,
    coefficients: Vec<(f64, f64)>,
}

impl MagnitudeBasis {
    fn at(&self, frequency: f64) -> f64 {
        if frequency <= 0.0 {
            return 0.0;
        }
        let logarithm = frequency.log2();
        let right = self
            .coefficients
            .partition_point(|(center, _)| *center <= logarithm);
        // At most two lattice neighbours overlap; omitted stopband kernels
        // contribute zero and must not stretch the neighbouring supports.
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
}

fn magnitude_basis(
    branch: &Curve,
    measurement: &Curve,
    baseline: &Curve,
    target: &Curve,
    protected: &[bool],
    config: &OptimizerConfig,
    boost: f64,
) -> Result<MagnitudeBasis, String> {
    let support =
        autoeq_core::resolution::MeasurementResolution::combine([branch, measurement, baseline])
            .ok_or("per-driver FIR has no measurement resolution support")?;
    let [source_low, source_high] = support
        .support_band_hz()
        .ok_or("per-driver FIR has no common measurement resolution support")?;
    let [requested_low, requested_high] = config.active_correction_band();
    let low = requested_low.max(source_low);
    let high = requested_high.min(source_high);
    if low >= high {
        return Err("per-driver FIR band has no common measurement resolution support".into());
    }
    // A common width avoids narrowing transitions between variable-width
    // kernels. Query original knots, including those absent from a derived grid.
    let width = support
        .max_width_octaves([low, high])
        .ok_or("per-driver FIR band has no finite measurement resolution warrant")?
        .max(1.0 / config.smooth_n.max(1) as f64);
    let mut basis = MagnitudeBasis {
        width_octaves: width,
        coefficients: Vec::new(),
    };
    let mut center = low.log2() + width;
    // Full support is twice the FWHM. Never shrink a kernel at band boundaries.
    while center + width <= high.log2() {
        let mut weighted_sum = 0.0;
        let mut weight_sum = 0.0;
        let mut cap = boost;
        let mut stopband = false;
        let frequencies = target
            .freq
            .as_slice()
            .ok_or("per-driver FIR needs a contiguous frequency grid")?;
        let first =
            frequencies.partition_point(|frequency| *frequency <= 2.0_f64.powf(center - width));
        let last =
            frequencies.partition_point(|frequency| *frequency < 2.0_f64.powf(center + width));
        for (i, &frequency) in frequencies.iter().enumerate().take(last).skip(first) {
            let distance = (frequency.log2() - center) / width;
            if distance.abs() >= 1.0 {
                continue;
            }
            let weight = 0.5 * (1.0 + (std::f64::consts::PI * distance).cos());
            // Log-frequency quadrature prevents denser display regions from
            // receiving more authority. Endpoint cells use one adjacent span.
            let left = if i > 0 && target.freq[i - 1] > 0.0 {
                (frequency / target.freq[i - 1]).ln()
            } else {
                0.0
            };
            let right = if i + 1 < target.freq.len() {
                (target.freq[i + 1] / frequency).ln()
            } else {
                0.0
            };
            let cell = if left > 0.0 && right > 0.0 {
                0.5 * (left + right)
            } else {
                left.max(right)
            };
            weighted_sum += weight * cell * (target.spl[i] - baseline.spl[i]);
            weight_sum += weight * cell;
            let retained_boost = (branch.spl[i] - measurement.spl[i]).max(0.0);
            cap = cap.min(if protected[i] {
                0.0
            } else {
                (boost - retained_boost).max(0.0)
            });
            stopband |= branch.spl[i] < baseline.spl[i] - 30.0;
        }
        if weight_sum > 0.0 && !stopband {
            // The existing 12 dB cut bound applies to the whole partition.
            let gain = (weighted_sum / weight_sum).clamp(-12.0, cap);
            basis.coefficients.push((center, gain));
        }
        center += width;
    }
    Ok(basis)
}

/// Real, centered Hann-windowed FIR, with inverse FFT normalization 1/N.
/// Phase is interpolated on log Hz; analytic magnitude is sampled directly.
/// DC/Nyquist remain real.
fn realize(
    freq: &Array1<f64>,
    phase: &[f64],
    magnitude: Option<&MagnitudeBasis>,
    strength: f64,
    taps: usize,
    fs: f64,
) -> Vec<f64> {
    let n = (taps * 4).max(65536).next_power_of_two();
    let mut spectrum = vec![Complex64::new(1.0, 0.0); n];
    for k in 1..n / 2 {
        let f = k as f64 * fs / n as f64;
        let (mut left, mut right) = (0, freq.len());
        while left < right {
            let middle = (left + right) / 2;
            if freq[middle] < f {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        let j = left;
        let angle = if j == 0 || j == freq.len() {
            0.0
        } else {
            let t = (f / freq[j - 1]).ln() / (freq[j] / freq[j - 1]).ln();
            phase[j - 1] + t * (phase[j] - phase[j - 1])
        };
        let gain = magnitude.map_or(0.0, |basis| strength * basis.at(f));
        spectrum[k] = Complex64::from_polar(10.0_f64.powf(gain / 20.0), angle.to_radians());
        spectrum[n - k] = spectrum[k].conj();
    }
    FftPlanner::<f64>::new()
        .plan_fft_inverse(n)
        .process(&mut spectrum);
    let center = taps / 2;
    (0..taps)
        .map(|i| {
            let window = 0.5
                + 0.5 * (std::f64::consts::PI * (i as f64 - center as f64) / center as f64).cos();
            spectrum[(n + i - center) % n].re / n as f64 * window
        })
        .collect()
}

/// Design bounded FIRs jointly for phase-referenced physical branches.
///
/// `branches` include retained gain/delay/crossover/IIR and common DSP, on an
/// identical grid; `measurements` are their unfiltered calibrated captures.
/// Mixed phase retains phase-only semantics. A delayed identity is an explicit
/// safe fallback when no finite candidate improves the protected objective.
/// Explicit excess-phase requests require a supported numerical assessment
/// for every raw capture. Callers must separately authorize source timing,
/// target policy, and measurement provenance before deploying the result.
///
/// # Errors
/// Returns an error for invalid inputs or an unsupported excess-phase assessment.
pub fn generate_per_driver_firs(
    branches: &[Curve],
    measurements: &[Curve],
    target: &Curve,
    config: &OptimizerConfig,
    fs: f64,
) -> Result<PerDriverFir, String> {
    let [active_min_freq, active_max_freq] = config.active_correction_band();
    let fir = config.fir.clone().unwrap_or_default();
    if branches.is_empty() || branches.len() != measurements.len() || !fs.is_finite() || fs <= 0.0 {
        return Err("invalid per-driver FIR inputs".into());
    }
    target
        .validate("per-driver FIR target")
        .map_err(|e| e.to_string())?;
    for c in branches.iter().chain(measurements) {
        c.validate("per-driver FIR branch")
            .map_err(|e| e.to_string())?;
        if c.freq != target.freq {
            return Err("per-driver FIR grids must match".into());
        }
        if c.phase.is_none() {
            return Err("per-driver FIR coherent design requires measured branch phase".into());
        }
    }
    if !active_min_freq.is_finite()
        || !active_max_freq.is_finite()
        || active_min_freq <= 0.0
        || active_max_freq <= active_min_freq
        || active_max_freq >= fs / 2.0
    {
        return Err("invalid per-driver FIR correction band".into());
    }
    let phase_only = config.processing_mode == ProcessingMode::MixedPhase;
    let mp = config.mixed_phase.as_ref();
    let taps = if phase_only {
        ((mp.map_or(10.0, |c| c.max_fir_length_ms) * fs / 1000.0).round() as usize).max(31) | 1
    } else {
        fir.taps
    };
    if !(31..=131072).contains(&taps) {
        return Err("per-driver FIR taps must be 31..=131072".into());
    }
    let minimum_phase = !phase_only && fir.phase.eq_ignore_ascii_case("minimum");
    let latency = if minimum_phase { 0 } else { taps / 2 };
    let mut identity = vec![0.0; taps];
    identity[latency] = 1.0;
    let mut best = vec![identity; branches.len()];
    let baseline = sum(branches, &best, fs);
    let masks: Vec<_> = measurements.iter().map(fir_null_mask).collect();
    let combined_mask = fir_null_mask(&baseline);
    let protected_bins = masks.iter().flatten().filter(|&&v| v).count();
    let score = |c: &Curve| {
        let mut error = 0.0;
        let mut weight = 0.0;
        for (i, &masked) in combined_mask[..c.freq.len()].iter().enumerate().skip(1) {
            if !masked && (active_min_freq..=active_max_freq).contains(&c.freq[i]) {
                let w = (c.freq[i] / c.freq[i - 1]).ln();
                error += w * (c.spl[i] - target.spl[i]).powi(2);
                weight += w;
            }
        }
        (error / weight.max(1e-12)).sqrt()
    };
    let before = score(&baseline);
    let mut best_score = before;
    let mut best_curve = baseline.clone();
    let mut selected_phase_strength = 0.0;
    let boost = fir
        .max_boost_db
        .unwrap_or(config.max_db)
        .min(config.max_db)
        .max(0.0);
    let phase_enabled =
        phase_only || (fir.phase.eq_ignore_ascii_case("kirkeby") && fir.correct_excess_phase);
    let assessment = mp
        .map(|config| config.assessment.clone())
        .unwrap_or_default();
    let phases: Vec<_> = measurements
        .iter()
        .enumerate()
        .map(|(driver, c)| {
            if phase_enabled {
                assess_per_driver_phase_target(
                    c,
                    &assessment,
                    [active_min_freq, active_max_freq],
                    fs,
                )
                .map_err(|reason| format!("per-driver {driver} excess-phase assessment: {reason}"))
            } else {
                Ok(vec![0.0; c.freq.len()])
            }
        })
        .collect::<Result<_, String>>()?;
    let magnitudes = if phase_only {
        Vec::new()
    } else {
        branches
            .iter()
            .enumerate()
            .map(|(driver, branch)| {
                let protected: Vec<_> = masks[driver]
                    .iter()
                    .zip(&combined_mask)
                    .map(|(individual, combined)| *individual || *combined)
                    .collect();
                magnitude_basis(
                    branch,
                    &measurements[driver],
                    &baseline,
                    target,
                    &protected,
                    config,
                    boost,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    // Joint search: every candidate is a complete set, never a collection of
    // independently accepted full-target inverses.
    for phase_strength in [0.0, 0.25, 0.5, 1.0] {
        if !phase_enabled && phase_strength != 0.0 {
            continue;
        }
        for strength in [0.0, 0.25, 0.5, 1.0] {
            if (phase_only && strength != 1.0) || (phase_strength == 0.0 && strength == 0.0) {
                continue;
            }
            let candidate: Vec<_> = branches
                .iter()
                .enumerate()
                .map(|(driver, branch)| {
                    let mut phase = vec![0.0; target.freq.len()];
                    for i in 0..phase.len() {
                        let band = band_weight(target.freq[i], active_min_freq, active_max_freq);
                        if !masks[driver][i] {
                            // The assessment already negates and tapers the excess
                            // phase. Search only bounded strengths of that proposal.
                            phase[i] = phase_strength * band * phases[driver][i];
                        }
                        if branch.spl[i] < baseline.spl[i] - 30.0 {
                            phase[i] = 0.0;
                        }
                    }
                    let magnitude = (!phase_only).then(|| &magnitudes[driver]);
                    if minimum_phase {
                        // Evaluate the analytic basis on the designer's own FFT
                        // bins, rather than optimizing an interpolated dense grid.
                        let n = (taps * 8).next_power_of_two().max(4096);
                        let frequencies: Vec<_> = (1..=n / 2)
                            .map(|index| index as f64 * fs / n as f64)
                            .collect();
                        let db: Vec<_> = frequencies
                            .iter()
                            .map(|&frequency| {
                                magnitude.map_or(0.0, |basis| strength * basis.at(frequency))
                            })
                            .collect();
                        math_audio_iir_fir::generate_fir_from_response(
                            &frequencies,
                            &db,
                            &math_audio_iir_fir::FirDesignConfig {
                                n_taps: taps,
                                sample_rate: fs,
                                phase: autoeq_fir::FirPhase::Minimum,
                                // A symmetric window erases the leading energy
                                // of a causal minimum-phase impulse.
                                window: math_audio_iir_fir::WindowType::Rectangular,
                                ..Default::default()
                            },
                        )
                    } else {
                        realize(&target.freq, &phase, magnitude, strength, taps, fs)
                    }
                })
                .collect();
            let safe = candidate.iter().enumerate().all(|(driver, coefficients)| {
                if coefficients.iter().any(|v| !v.is_finite()) {
                    return false;
                }
                if phase_only || fir.pre_ringing.is_some() {
                    let threshold = if phase_only {
                        mp.map_or(-30.0, |c| c.pre_ringing_threshold_db)
                    } else {
                        fir.pre_ringing.as_ref().unwrap().threshold_db
                    };
                    let peak = coefficients.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
                    let pre = coefficients[..latency]
                        .iter()
                        .map(|v| v.abs())
                        .fold(0.0_f64, f64::max);
                    if pre > peak * 10.0_f64.powf(threshold / 20.0) {
                        return false;
                    }
                }
                let response =
                    crate::response::compute_fir_complex_response(coefficients, &target.freq, fs);
                response.iter().enumerate().all(|(i, h)| {
                    let db = 20.0 * h.norm().max(1e-15).log10();
                    let protected = masks[driver][i] || combined_mask[i];
                    let outside =
                        target.freq[i] < active_min_freq || target.freq[i] > active_max_freq;
                    let retained_boost =
                        (branches[driver].spl[i] - measurements[driver].spl[i]).max(0.0);
                    db <= (if protected {
                        0.1
                    } else {
                        (boost - retained_boost).max(0.0) + 0.1
                    }) && (!phase_only || db.abs() <= 0.5)
                        && (!outside || db.abs() <= 0.5)
                })
            });
            if !safe {
                continue;
            }
            let curve = sum(branches, &candidate, fs);
            let loss = score(&curve);
            if loss.is_finite() && loss + 1e-6 < best_score {
                best = candidate;
                best_score = loss;
                best_curve = curve;
                selected_phase_strength = phase_strength;
            }
        }
    }
    log::info!(
        "Per-driver FIR: {} outputs, {} taps, {} protected bins, target RMS {:.3} -> {:.3} dB{}",
        branches.len(),
        taps,
        protected_bins,
        before,
        best_score,
        if best_score == before {
            " (delayed identity: no safe improving candidate)"
        } else {
            ""
        }
    );
    Ok(PerDriverFir {
        coefficients: best,
        final_curve: best_curve,
        latency_samples: latency,
        protected_bins,
        before_rms_db: before,
        after_rms_db: best_score,
        selected_phase_strength,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn flat(level: f64) -> Curve {
        let freq = Array1::logspace(10.0, 20.0_f64.log10(), 20000.0_f64.log10(), 160);
        Curve {
            spl: Array1::from_elem(freq.len(), level),
            phase: Some(Array1::zeros(freq.len())),
            freq,
            ..Curve::default()
        }
    }
    fn config() -> OptimizerConfig {
        OptimizerConfig {
            min_freq: 20.0,
            max_freq: 200.0,
            max_db: 4.0,
            processing_mode: ProcessingMode::PhaseLinear,
            fir: Some(roomeq_model::FirConfig {
                taps: 4096,
                phase: "linear".into(),
                placement: roomeq_model::FirPlacement::PerDriver,
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    #[test]
    fn per_driver_fir_joint_residual_reduces_peak_and_preserves_upper_band() {
        let target = flat(80.0);
        let mut branch = flat(80.0 - 20.0 * 2.0_f64.log10());
        for (i, &f) in branch.freq.iter().enumerate() {
            branch.spl[i] += 6.0 * (-((f / 90.0).log2() / 0.6).powi(2)).exp();
        }
        let branches = vec![branch.clone(), branch];
        let result =
            generate_per_driver_firs(&branches, &branches, &target, &config(), 48000.0).unwrap();
        assert!(
            result.after_rms_db < result.before_rms_db - 0.5,
            "{} -> {}",
            result.before_rms_db,
            result.after_rms_db
        );
        assert_eq!(result.coefficients.len(), 2);
        assert_eq!(result.latency_samples, 2048);
        for taps in &result.coefficients {
            assert_eq!(taps.len(), 4096);
            let h = crate::response::compute_fir_complex_response(
                taps,
                &Array1::from_vec(vec![500.0, 1000.0, 10000.0]),
                48000.0,
            );
            assert!(h.iter().all(|v| (20.0 * v.norm().log10()).abs() < 0.1));
        }
    }
    #[test]
    fn per_driver_fir_room_nulls_are_not_boosted() {
        let target = flat(80.0);
        let mut branch = flat(80.0 - 20.0 * 2.0_f64.log10());
        for (i, &f) in branch.freq.iter().enumerate() {
            branch.spl[i] -= 25.0 * (-((f / 80.0).log2() / 0.08).powi(2)).exp();
        }
        let mask = fir_null_mask(&branch);
        assert!(mask.iter().any(|&v| v));
        let branches = vec![branch.clone(), branch];
        let result =
            generate_per_driver_firs(&branches, &branches, &target, &config(), 48000.0).unwrap();
        assert!(result.protected_bins > 0);
        for taps in &result.coefficients {
            let h = crate::response::compute_fir_complex_response(taps, &target.freq, 48000.0);
            for (i, &protected) in mask.iter().enumerate() {
                if protected {
                    assert!(20.0 * h[i].norm().log10() <= 0.100001);
                }
            }
        }
    }
    #[test]
    fn per_driver_basis_retains_original_resolution_on_dense_display_grid() {
        let mut branch = flat(80.0);
        branch.freq = Array1::logspace(10.0, 20.0_f64.log10(), 20_000.0_f64.log10(), 2048);
        branch.spl = Array1::from_elem(2048, 80.0);
        branch.phase = Some(Array1::zeros(2048));
        let raw = [
            20.0, 40.0, 80.0, 160.0, 320.0, 640.0, 1280.0, 2560.0, 5120.0, 10240.0, 20480.0,
        ];
        branch.resolution = Some(std::sync::Arc::new(
            autoeq_core::resolution::MeasurementResolution::from_grid(&raw).unwrap(),
        ));
        let mut target = branch.clone();
        let spike = target
            .freq
            .iter()
            .position(|frequency| *frequency >= 90.0)
            .unwrap();
        target.spl[spike] += 10.0;
        let mut config = config();
        config.max_freq = 2_000.0;
        let basis = magnitude_basis(
            &branch,
            &branch,
            &branch,
            &target,
            &vec![false; 2048],
            &config,
            4.0,
        )
        .unwrap();
        assert!(basis.width_octaves >= 2.0);
        let peak = target
            .freq
            .iter()
            .map(|frequency| basis.at(*frequency))
            .fold(0.0, f64::max);
        assert!(
            peak < 0.1,
            "a densified one-bin spike must not acquire correction authority: {peak}"
        );
        assert!(basis.coefficients.iter().all(|(center, _)| {
            2.0_f64.powf(center - basis.width_octaves) >= config.min_freq
                && 2.0_f64.powf(center + basis.width_octaves) <= config.max_freq
        }));
        config.max_freq = 100.0;
        let narrow = magnitude_basis(
            &branch,
            &branch,
            &branch,
            &target,
            &vec![false; 2048],
            &config,
            4.0,
        )
        .unwrap();
        assert!(
            narrow.coefficients.is_empty(),
            "never shrink a kernel to fit an unsupported band"
        );
        let combined = sum(&[branch.clone(), branch], &[vec![1.0], vec![1.0]], 48_000.0);
        assert!(combined.retained_resolution().width_octaves(90.0).unwrap() >= 2.0);
    }

    #[test]
    fn per_driver_basis_caps_whole_support_without_binwise_null_clipping() {
        let branch = flat(80.0);
        let mut measurement = branch.clone();
        measurement.spl -= 2.0;
        let mut target = branch.clone();
        target.spl += 6.0;
        let mut config = config();
        config.max_freq = 2_000.0;
        let protected_index = branch
            .freq
            .iter()
            .position(|frequency| *frequency >= 100.0)
            .unwrap();
        let mut protected = vec![false; branch.freq.len()];
        protected[protected_index] = true;
        let basis = magnitude_basis(
            &branch,
            &measurement,
            &branch,
            &target,
            &protected,
            &config,
            4.0,
        )
        .unwrap();
        assert_eq!(basis.at(branch.freq[protected_index]), 0.0);
        assert!(
            basis.at(500.0) > 1.0,
            "a protected null must not disable all warranted boosts"
        );
        for frequency in Array1::logspace(10.0, 20.0_f64.log10(), 2_000.0_f64.log10(), 4096) {
            assert!(
                basis.at(frequency) <= 2.0 + 1e-12,
                "retained boost consumes the same whole-support cap"
            );
        }
        for (center, gain) in &basis.coefficients {
            if (branch.freq[protected_index].log2() - center).abs() < basis.width_octaves {
                assert!(
                    *gain <= 0.0,
                    "null protection bounds the full kernel coefficient"
                );
            }
        }
        let mut stopband_branch = branch.clone();
        stopband_branch.spl[protected_index] -= 31.0;
        let stopband = magnitude_basis(
            &stopband_branch,
            &measurement,
            &branch,
            &target,
            &vec![false; branch.freq.len()],
            &config,
            4.0,
        )
        .unwrap();
        assert_eq!(stopband.at(branch.freq[protected_index]), 0.0);
        assert!(
            stopband.coefficients.iter().all(|(center, _)| {
                (branch.freq[protected_index].log2() - center).abs() >= stopband.width_octaves
            }),
            "omit whole kernels overlapping physical crossover stopbands"
        );
    }

    #[test]
    fn per_driver_minimum_phase_basis_reduces_peak_with_causal_taps() {
        let target = flat(80.0);
        let mut branch = target.clone();
        for (i, &frequency) in branch.freq.iter().enumerate() {
            branch.spl[i] += 6.0 * (-((frequency / 90.0).log2() / 0.6).powi(2)).exp();
        }
        let mut config = config();
        config.fir.as_mut().unwrap().phase = "minimum".into();
        let result = generate_per_driver_firs(
            std::slice::from_ref(&branch),
            std::slice::from_ref(&branch),
            &target,
            &config,
            48_000.0,
        )
        .unwrap();
        assert_eq!(result.latency_samples, 0);
        assert!(result.after_rms_db < result.before_rms_db - 0.5);
        assert!(
            result.coefficients[0][0].abs() > 0.1,
            "do not erase the causal impulse's leading energy"
        );
    }

    #[test]
    fn per_driver_fir_rejects_mismatched_grid_and_invalid_taps() {
        let c = flat(80.0);
        let mut other = c.clone();
        other.freq[0] = 19.0;
        assert!(
            generate_per_driver_firs(&[other], std::slice::from_ref(&c), &c, &config(), 48000.0)
                .is_err()
        );
        let mut config = config();
        config.fir.as_mut().unwrap().taps = 0;
        assert!(
            generate_per_driver_firs(
                std::slice::from_ref(&c),
                std::slice::from_ref(&c),
                &c,
                &config,
                48000.0
            )
            .is_err()
        );
    }
    #[test]
    fn per_driver_fir_mixed_phase_is_not_magnitude_equalization() {
        let mut c = flat(80.0);
        // Explicit synthetic 40 dB SNR for this numerical phase fixture.
        c.noise_floor_db = Some(&c.spl - 40.0);
        let target = flat(86.0);
        let mut config = config();
        config.processing_mode = ProcessingMode::MixedPhase;
        let result =
            generate_per_driver_firs(&[c.clone()], &[c], &target, &config, 48000.0).unwrap();
        assert_eq!(result.coefficients[0].len(), 481);
        let h = crate::response::compute_fir_complex_response(
            &result.coefficients[0],
            &target.freq,
            48000.0,
        );
        assert!(h.iter().all(|v| (20.0 * v.norm().log10()).abs() < 0.00001));
    }
    #[test]
    fn per_driver_fir_low_coherence_and_noise_are_protected() {
        let mut c = flat(80.0);
        c.coherence = Some(Array1::from_elem(c.freq.len(), 0.2));
        assert!(fir_null_mask(&c).iter().all(|&v| v));
        c.coherence = None;
        c.noise_floor_db = Some(Array1::from_elem(c.freq.len(), 75.0));
        assert!(fir_null_mask(&c).iter().all(|&v| v));
    }

    #[test]
    fn per_driver_fir_honors_assessment_policy_and_rejects_invalid_policy() {
        let mut c = flat(80.0);
        c.noise_floor_db = Some(&c.spl - 40.0);
        let mut config = config();
        config.processing_mode = ProcessingMode::MixedPhase;
        config.mixed_phase = Some(
            serde_json::from_value(serde_json::json!({
                "assessment": { "snr_floor_db": 41.0 }
            }))
            .unwrap(),
        );
        let result = generate_per_driver_firs(&[c.clone()], &[c.clone()], &c, &config, 48000.0);
        assert!(result.err().unwrap().contains("snr coverage"));
        for invalid in [f64::NAN, -1.0, 1.1] {
            config
                .mixed_phase
                .as_mut()
                .unwrap()
                .assessment
                .min_valid_fraction = invalid;
            let result = generate_per_driver_firs(&[c.clone()], &[c.clone()], &c, &config, 48000.0);
            assert!(result.err().unwrap().contains("invalid assessment policy"));
        }
    }

    #[test]
    fn per_driver_phase_assessment_preserves_pure_propagation_delay() {
        let mut c = flat(80.0);
        c.noise_floor_db = Some(&c.spl - 40.0);
        c.phase = Some(c.freq.mapv(|f| -360.0 * f * 0.003));
        let phase = assess_per_driver_phase_target(&c, &Default::default(), [20.0, 200.0], 48000.0)
            .unwrap();
        assert!(phase.iter().all(|phase| phase.abs() < 1e-8));
    }

    #[test]
    fn per_driver_phase_admission_distinguishes_missing_evidence_from_invalid_inputs() {
        let mut capture = flat(80.0);
        let policy = roomeq_model::PhaseAssessmentConfig::default();
        assert!(matches!(
            assess_per_driver_phase_target(&capture, &policy, [20.0, 200.0], 48000.0),
            Err(PerDriverPhaseError::Unsupported(_))
        ));
        capture.noise_floor_db = Some(&capture.spl - 40.0);
        assert!(assess_per_driver_phase_target(&capture, &policy, [20.0, 200.0], 48000.0).is_ok());
        for (band, rate) in [
            ([20.0, 200.0], f64::NAN),
            ([20.0, 200.0], 0.0),
            ([200.0, 20.0], 48000.0),
            ([20.0, 24000.0], 48000.0),
        ] {
            assert!(matches!(
                assess_per_driver_phase_target(&capture, &policy, band, rate),
                Err(PerDriverPhaseError::InvalidInput(_))
            ));
        }
        let mut invalid_policy = policy.clone();
        invalid_policy.min_valid_fraction = f64::NAN;
        assert!(matches!(
            assess_per_driver_phase_target(&capture, &invalid_policy, [20.0, 200.0], 48000.0),
            Err(PerDriverPhaseError::InvalidInput(_))
        ));
        let mut malformed = capture.clone();
        malformed.noise_floor_db = Some(Array1::zeros(1));
        assert!(matches!(
            assess_per_driver_phase_target(&malformed, &policy, [20.0, 200.0], 48000.0),
            Err(PerDriverPhaseError::InvalidInput(_))
        ));
        capture.phase = None;
        assert!(matches!(
            assess_per_driver_phase_target(&capture, &policy, [20.0, 200.0], 48000.0),
            Err(PerDriverPhaseError::Unsupported(_))
        ));
    }

    #[test]
    fn per_driver_fir_repairs_relative_phase_without_electrical_boost() {
        let target = flat(80.0);
        let mut first = flat(80.0 - 20.0 * 2.0_f64.log10());
        // Explicit synthetic 40 dB SNR, not inferred production evidence.
        first.noise_floor_db = Some(&first.spl - 40.0);
        let mut second = first.clone();
        for (i, &f) in second.freq.iter().enumerate() {
            second.phase.as_mut().unwrap()[i] = 80.0 * (-((f / 85.0).log2() / 0.8).powi(2)).exp();
        }
        let branches = vec![first, second];
        let mut config = config();
        config.max_db = 0.0;
        let fir = config.fir.as_mut().unwrap();
        fir.phase = "kirkeby".into();
        fir.correct_excess_phase = true;
        fir.max_boost_db = Some(0.0);
        let result =
            generate_per_driver_firs(&branches, &branches, &target, &config, 48000.0).unwrap();
        assert!(
            result.after_rms_db < result.before_rms_db - 0.01,
            "{} -> {}",
            result.before_rms_db,
            result.after_rms_db
        );
        assert_ne!(result.coefficients[0], result.coefficients[1]);
        for taps in &result.coefficients {
            let h = crate::response::compute_fir_complex_response(taps, &target.freq, 48000.0);
            assert!(h.iter().all(|v| 20.0 * v.norm().log10() <= 0.100001));
        }
    }
}
