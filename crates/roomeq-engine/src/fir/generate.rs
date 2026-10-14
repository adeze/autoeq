use super::apply::{gd_delay_padding_samples, realize_gd_fir_delay};
use crate::Curve;
use crate::eq::{EqResources, PreparedEqTarget};
pub use autoeq_fir::FirPhase;
use ndarray::Array1;
use roomeq_model::OptimizerConfig;
use std::error::Error;
use std::sync::Once;

static EXCESS_PHASE_IDENTITY_WARNING: Once = Once::new();
static MISSING_PHASE_FALLBACK_WARNING: Once = Once::new();

/// Fit the realized finite FIR, not just the pre-window inversion spectrum.
/// Windowing a band-limited excess-phase inverse can attenuate its bass even
/// when the requested magnitude was correct. Retain only improving candidates.
pub fn generate_group_residual_fir_prepared(
    measurement: &Curve,
    config: &OptimizerConfig,
    target: &Curve,
    sample_rate: f64,
) -> Result<Vec<f64>, Box<dyn Error>> {
    let [active_min_freq, active_max_freq] = config.active_correction_band();
    let mut best = generate_fir_correction_prepared(measurement, config, target, sample_rate)?;
    let Some(fir) = config
        .fir
        .as_ref()
        .filter(|fir| fir.phase.eq_ignore_ascii_case("kirkeby") && fir.correct_excess_phase)
    else {
        return Ok(best);
    };
    let evaluate = |coeffs: &[f64]| {
        let response =
            crate::response::compute_fir_complex_response(coeffs, &measurement.freq, sample_rate);
        let corrected = crate::response::apply_complex_response(measurement, &response);
        let score =
            crate::group::target_error_score(&corrected, target, active_min_freq, active_max_freq);
        (score, corrected)
    };
    let (mut best_score, mut realized) = evaluate(&best);
    let initial_score = best_score;
    let mut design_target = target.clone();
    for _ in 0..6 {
        if best_score < 0.05 {
            break;
        }
        for i in 0..design_target.freq.len() {
            if (active_min_freq..=active_max_freq).contains(&design_target.freq[i]) {
                let error = target.spl[i] - realized.spl[i];
                design_target.spl[i] = (design_target.spl[i] + 0.8 * error)
                    .clamp(target.spl[i] - 12.0, target.spl[i] + 12.0);
            }
        }
        let candidate =
            generate_fir_correction_prepared(measurement, config, &design_target, sample_rate)?;
        let (score, candidate_curve) = evaluate(&candidate);
        // Keep the existing Kirkeby boost limit (or a stricter explicit cap).
        let boost_limit = fir.max_boost_db.unwrap_or(15.0);
        let boost_ok = candidate_curve
            .spl
            .iter()
            .zip(measurement.spl.iter())
            .all(|(&after, &before)| after.is_finite() && after - before <= boost_limit + 0.1);
        if !score.is_finite() || !boost_ok || score >= best_score {
            break;
        }
        best_score = score;
        best = candidate;
        realized = candidate_curve;
    }
    log::info!(
        "Realized residual FIR target RMS: {initial_score:.3} -> {best_score:.3} dB ({} taps)",
        best.len()
    );
    Ok(best)
}

/// Resolve a workflow-prepared FIR target on the measurement grid.
pub fn prepared_fir_target_curve(
    measurement: &Curve,
    config: &OptimizerConfig,
    resources: &EqResources,
) -> Curve {
    let [active_min_freq, active_max_freq] = config.active_correction_band();
    let mut target = match resources.target.as_ref() {
        Some(PreparedEqTarget::Curve(target)) => {
            autoeq_core::normalize_and_interpolate_response(&measurement.freq, target)
        }
        Some(PreparedEqTarget::Predefined(name)) => {
            autoeq_core::build_target_curve_by_name(name, &measurement.freq, measurement)
        }
        None => {
            let (sum, count) = measurement
                .freq
                .iter()
                .zip(measurement.spl.iter())
                .filter_map(|(&frequency, &level)| {
                    (frequency >= active_min_freq && frequency <= active_max_freq).then_some(level)
                })
                .fold((0.0, 0_usize), |(sum, count), level| {
                    (sum + level, count + 1)
                });
            let mean_level = if count == 0 { 0.0 } else { sum / count as f64 };
            Curve {
                freq: measurement.freq.clone(),
                spl: Array1::from_elem(measurement.freq.len(), mean_level),
                phase: None,
                ..Curve::default()
            }
        }
    };
    let measurement_mean = roomeq_analysis::response_metrics::mean_response_in_range(
        measurement,
        active_min_freq,
        active_max_freq,
    );
    let target_mean = roomeq_analysis::response_metrics::mean_response_in_range(
        &target,
        active_min_freq,
        active_max_freq,
    );
    // IIR and FIR stages must share the same immutable measured reference.
    // In particular, a bass-only correction band is not a level-reference band.
    let reference = crate::spectral_align::limited_correction_target_reference(
        measurement,
        &target,
        active_max_freq,
    )
    .unwrap_or(measurement_mean - target_mean);
    if reference.is_finite() {
        target.spl += reference;
    }
    target
}

/// Generate FIR coefficients using only workflow-prepared in-memory resources.
pub fn generate_fir_correction_with_resources(
    measurement: &Curve,
    config: &OptimizerConfig,
    resources: &EqResources,
    sample_rate: f64,
) -> Result<Vec<f64>, Box<dyn Error>> {
    let target = prepared_fir_target_curve(measurement, config, resources);
    generate_fir_correction_prepared(measurement, config, &target, sample_rate)
}

fn correction_rms_db(measurement: &Curve, target: &Curve, min_freq: f64, max_freq: f64) -> f64 {
    let (sum_squares, count) = measurement
        .freq
        .iter()
        .zip(measurement.spl.iter())
        .zip(target.spl.iter())
        .filter(|((frequency, _), _)| **frequency >= min_freq && **frequency <= max_freq)
        .fold((0.0, 0_usize), |(sum, count), ((_, measured), desired)| {
            (sum + (desired - measured).powi(2), count + 1)
        });
    if count == 0 {
        0.0
    } else {
        (sum_squares / count as f64).sqrt()
    }
}

fn is_effectively_identity_fir(coefficients: &[f64]) -> bool {
    let Some((peak_index, peak)) = coefficients
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.abs().total_cmp(&right.abs()))
    else {
        return false;
    };
    let off_peak = coefficients
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != peak_index)
        .map(|(_, coefficient)| coefficient.abs())
        .reduce(f64::max)
        .unwrap_or(0.0);
    (peak.abs() - 1.0).abs() <= 1.0e-5 && off_peak <= 1.0e-8
}

/// Retry an identity-like excess-phase result only for a nontrivial target.
pub(super) fn recover_excess_phase_identity(
    coefficients: Vec<f64>,
    correct_excess_phase: bool,
    correction_rms: f64,
    magnitude_only: impl FnOnce() -> Vec<f64>,
) -> Vec<f64> {
    if correct_excess_phase && correction_rms > 0.1 && is_effectively_identity_fir(&coefficients) {
        EXCESS_PHASE_IDENTITY_WARNING.call_once(|| {
            log::warn!(
                "One or more excess-phase FIR designs collapsed to identity despite a non-trivial magnitude correction; retrying magnitude-only Kirkeby inversion"
            );
        });
        magnitude_only()
    } else {
        coefficients
    }
}

/// Generate an FIR correction filter for a single channel
///
/// This is the main entry point for FIR-based room correction. It handles:
/// - Phase type selection (linear, minimum, or kirkeby)
/// - FIR coefficient generation
///
/// # Arguments
/// * `measurement` - The room measurement curve
/// * `config` - Optimizer configuration (contains FIR settings)
/// * `target_curve` - Prepared target curve on the measurement frequency grid
/// * `sample_rate` - Sample rate in Hz
///
/// # Returns
/// * Vector of FIR coefficients on success
pub fn generate_fir_correction_prepared(
    measurement: &Curve,
    config: &OptimizerConfig,
    target_curve: &Curve,
    sample_rate: f64,
) -> Result<Vec<f64>, Box<dyn Error>> {
    let requested =
        generate_fir_correction_prepared_raw(measurement, config, target_curve, sample_rate)?;
    if !config
        .fir
        .as_ref()
        .is_some_and(|fir| fir.phase.eq_ignore_ascii_case("kirkeby") && fir.correct_excess_phase)
        || measurement.phase.is_none()
    {
        return Ok(requested);
    }

    // A complex inverse has the desired magnitude before truncation/windowing,
    // not necessarily afterwards. In particular, long acoustic delays can put
    // its energy outside the available taps. Do not sacrifice magnitude EQ to
    // an unrealizable excess-phase request.
    let mut magnitude_config = config.clone();
    magnitude_config.fir.as_mut().unwrap().correct_excess_phase = false;
    let magnitude = generate_fir_correction_prepared_raw(
        measurement,
        &magnitude_config,
        target_curve,
        sample_rate,
    )?;
    let [low, high] = config.active_correction_band();
    let score = |coefficients: &[f64]| {
        let transfer = crate::response::compute_fir_complex_response(
            coefficients,
            &measurement.freq,
            sample_rate,
        );
        let realized = crate::response::apply_complex_response(measurement, &transfer);
        crate::group::target_error_score(&realized, target_curve, low, high)
    };
    let requested_error = score(&requested);
    let magnitude_error = score(&magnitude);
    // Keep feasible phase correction, allowing only negligible realization
    // differences. Runtime electrical/acoustic acceptance still applies later.
    if magnitude_error.is_finite()
        && (!requested_error.is_finite() || requested_error > magnitude_error + 0.05)
    {
        log::warn!(
            "Excess-phase FIR magnitude regression after realization ({requested_error:.3} dB vs \
             {magnitude_error:.3} dB magnitude-only over {low:.1}-{high:.1} Hz); \
             retaining magnitude correction without excess-phase inversion"
        );
        Ok(magnitude)
    } else {
        Ok(requested)
    }
}

fn checked_coefficients(
    coefficients: Vec<f64>,
    requested_taps: usize,
) -> Result<Vec<f64>, Box<dyn Error>> {
    if coefficients.len() != requested_taps || coefficients.iter().any(|tap| !tap.is_finite()) {
        return Err("FIR design produced an invalid tap count or nonfinite coefficients".into());
    }
    Ok(coefficients)
}

// Apply the warrant to the regularized design spectrum, retaining original
// acquisition support rather than inferring it from the dense FFT grid.
pub(super) fn generate_resolved_kirkeby(
    measurement: &Curve,
    target: &Curve,
    optimizer: &OptimizerConfig,
    design: &math_audio_iir_fir::FirDesignConfig,
) -> Result<Vec<f64>, Box<dyn Error>> {
    let support = measurement.retained_resolution();
    let [source_low, source_high] = support
        .support_band_hz()
        .ok_or("Kirkeby correction has no common measurement resolution support")?;
    let band = [
        design.min_freq.max(source_low),
        design.max_freq.min(source_high),
    ];
    if band[0] >= band[1] {
        return Err("Kirkeby correction band has no measurement resolution support".into());
    }
    let upper_envelope = fir_boost_envelope(measurement, optimizer);
    let mut context = math_audio_iir_fir::FirDesignContext::new();
    context.generate_kirkeby_correction_with_magnitude_projection(
        &measurement.freq.to_vec(),
        &measurement.spl.to_vec(),
        measurement
            .phase
            .as_ref()
            .map(|phase| phase.to_vec())
            .as_deref(),
        &target.spl.to_vec(),
        design,
        |frequencies, magnitude| -> Result<(), Box<dyn Error>> {
            let spectrum = Curve {
                freq: Array1::from_vec(frequencies.to_vec()),
                spl: Array1::from_vec(magnitude.to_vec()),
                resolution: Some(support.clone()),
                ..Default::default()
            };
            let mut basis = autoeq_core::curve_transforms::ResolutionMagnitudeBasis::new(
                &spectrum,
                &spectrum.spl,
                1.0 / optimizer.smooth_n.max(1) as f64,
                band,
            )?;
            basis.cap_positive_support(&measurement.freq, &upper_envelope)?;
            let constrained = basis.evaluate(&spectrum.freq);
            magnitude.copy_from_slice(
                constrained
                    .as_slice()
                    .ok_or("Noncontiguous FIR magnitude")?,
            );
            Ok(())
        },
    )
}

fn fir_boost_envelope(measurement: &Curve, config: &OptimizerConfig) -> Array1<f64> {
    let cap = config
        .fir
        .as_ref()
        .and_then(|fir| fir.max_boost_db)
        .unwrap_or(f64::MAX / 4.0);
    Array1::from_iter(
        super::fir_null_mask(measurement)
            .into_iter()
            .map(|protected| if protected { 0.0 } else { cap }),
    )
}

fn generate_fir_correction_prepared_raw(
    measurement: &Curve,
    config: &OptimizerConfig,
    target_curve: &Curve,
    sample_rate: f64,
) -> Result<Vec<f64>, Box<dyn Error>> {
    let [active_min_freq, active_max_freq] = config.active_correction_band();
    let fir_config = config.fir.as_ref().ok_or("FIR configuration missing")?;
    let n_taps = fir_config.taps;

    // Library entry points also accept programmatically constructed configs;
    // validate before allocations or unchecked numerical design kernels.
    if !sample_rate.is_finite()
        || sample_rate <= 0.0
        || sample_rate > autoeq_fir::MAX_CHECKED_SAMPLE_RATE
    {
        return Err(autoeq_fir::FirDesignError::InvalidSampleRate { value: sample_rate }.into());
    }
    if !(autoeq_fir::MIN_CHECKED_TAPS..=autoeq_fir::MAX_CHECKED_TAPS).contains(&n_taps) {
        return Err(autoeq_fir::FirDesignError::InvalidTapCount { n_taps }.into());
    }
    if measurement.freq.len() < 2 {
        return Err("Prepared FIR measurement needs at least two frequency bins".into());
    }
    if !active_min_freq.is_finite()
        || !active_max_freq.is_finite()
        || active_min_freq <= 0.0
        || active_max_freq <= active_min_freq
        || active_max_freq > sample_rate / 2.0
    {
        return Err(autoeq_fir::FirDesignError::UnsupportedFrequencySpan {
            min_freq: active_min_freq,
            max_freq: active_max_freq,
            nyquist: sample_rate / 2.0,
        }
        .into());
    }
    if fir_config
        .max_boost_db
        .is_some_and(|boost| !boost.is_finite() || boost < 0.0)
    {
        return Err("FIR maximum boost must be finite and nonnegative".into());
    }
    if !fir_config.phase_smoothing.is_finite() || fir_config.phase_smoothing < 0.0 {
        return Err("FIR phase smoothing must be finite and nonnegative".into());
    }
    if fir_config.pre_ringing.as_ref().is_some_and(|policy| {
        !policy.threshold_db.is_finite()
            || !policy.max_time_s.is_finite()
            || policy.max_time_s < 0.0
    }) {
        return Err("FIR pre-ringing policy must have finite values and nonnegative time".into());
    }

    // Prepared targets must already share the measurement grid. In particular,
    // boost capping below is pointwise, not an interpolation operation.
    if target_curve.freq != measurement.freq
        || target_curve.spl.len() != measurement.freq.len()
        || measurement.spl.len() != measurement.freq.len()
    {
        return Err(
            "Prepared FIR target and levels must match the measurement frequency grid".into(),
        );
    }

    measurement.validate("prepared FIR measurement")?;
    target_curve.validate("prepared FIR target")?;

    // Constrain the magnitude proposal on the original numerical support.
    // Preserve the measurement itself: changing it would also change the
    // minimum/excess-phase decomposition used by Kirkeby inversion.
    let correction = &target_curve.spl - &measurement.spl;
    if !fir_config.phase.eq_ignore_ascii_case("kirkeby")
        && fir_config.max_boost_db.is_none()
        && correction
            .iter()
            .any(|gain| !10.0_f64.powf(*gain / 20.0).is_finite())
    {
        // Conditioning must not turn an unrepresentable inversion into an
        // apparently valid neutral result merely because support is sparse.
        return Err("FIR proposal would produce nonfinite coefficients".into());
    }
    let correction = if let Some(max_boost) = fir_config.max_boost_db {
        correction.mapv(|gain| gain.min(max_boost))
    } else {
        correction
    };
    // Cap the input proposal before projection. Clipping individual projected
    // samples afterwards could reintroduce unsupported narrow features.
    let [source_min, source_max] = measurement
        .retained_resolution()
        .support_band_hz()
        .ok_or("prepared FIR has no original measurement resolution support")?;
    let supported_band = [
        active_min_freq.max(source_min),
        active_max_freq.min(source_max),
    ];
    // The basis stays neutral outside full supported features. An extrapolated
    // display grid must neither acquire correction support nor abort the fit.
    let mut magnitude_basis = autoeq_core::curve_transforms::ResolutionMagnitudeBasis::new(
        measurement,
        &correction,
        1.0 / config.smooth_n.max(1) as f64,
        supported_band,
    )?;
    magnitude_basis
        .cap_positive_support(&measurement.freq, &fir_boost_envelope(measurement, config))?;
    let mut resolved_target = target_curve.clone();
    resolved_target.spl = &measurement.spl + &magnitude_basis.evaluate(&measurement.freq);
    let target_curve = &resolved_target;

    if fir_config.phase.to_lowercase() == "kirkeby" {
        // Use Kirkeby regularized inversion with optional excess phase correction
        let pre_ringing =
            fir_config
                .pre_ringing
                .as_ref()
                .map(|pr| math_audio_iir_fir::PreRingingConfig {
                    threshold_db: pr.threshold_db,
                    max_time_s: pr.max_time_s,
                });
        let design = math_audio_iir_fir::FirDesignConfig {
            n_taps,
            sample_rate,
            phase: FirPhase::Kirkeby,
            min_freq: active_min_freq,
            max_freq: active_max_freq,
            correct_excess_phase: fir_config.correct_excess_phase && measurement.phase.is_some(),
            phase_smoothing_octaves: fir_config.phase_smoothing,
            pre_ringing,
            ..Default::default()
        };
        if fir_config.correct_excess_phase && measurement.phase.is_none() {
            MISSING_PHASE_FALLBACK_WARNING.call_once(|| {
                log::warn!(
                    "Excess-phase FIR correction requested but measurement has no phase data; falling back to magnitude-only Kirkeby inversion"
                );
            });
        }
        let coeffs = generate_resolved_kirkeby(measurement, target_curve, config, &design)?;
        let correction_rms =
            correction_rms_db(measurement, target_curve, active_min_freq, active_max_freq);
        let coeffs = if design.correct_excess_phase
            && correction_rms > 0.1
            && is_effectively_identity_fir(&coeffs)
        {
            let magnitude_design = math_audio_iir_fir::FirDesignConfig {
                correct_excess_phase: false,
                ..design
            };
            let fallback =
                generate_resolved_kirkeby(measurement, target_curve, config, &magnitude_design)?;
            recover_excess_phase_identity(coeffs, true, correction_rms, || fallback)
        } else {
            coeffs
        };
        checked_coefficients(coeffs, n_taps)
    } else {
        // Evaluate the supported analytic basis on the designer's exact FFT
        // bins; interpolation of a coarse sampled proposal must not reshape
        // the boundary transitions. A sub-bin neutral anchor fixes DC.
        let fft_size = (n_taps * 8).next_power_of_two().max(4096);
        let step = sample_rate / fft_size as f64;
        let anchor = (step * 0.5).min(active_min_freq * 0.5);
        let freqs: Vec<f64> = std::iter::once(anchor)
            .chain((1..=fft_size / 2).map(|index| index as f64 * step))
            .collect();
        let magnitude_db: Vec<f64> = freqs
            .iter()
            .map(|&frequency| magnitude_basis.at(frequency))
            .collect();

        let phase_type = match fir_config.phase.to_lowercase().as_str() {
            "linear" => FirPhase::Linear,
            "minimum" => FirPhase::Minimum,
            _ => return Err(format!("Unknown FIR phase type: {}", fir_config.phase).into()),
        };

        // Convert pre-ringing config if present
        let pre_ringing =
            fir_config
                .pre_ringing
                .as_ref()
                .map(|pr| math_audio_iir_fir::PreRingingConfig {
                    threshold_db: pr.threshold_db,
                    max_time_s: pr.max_time_s,
                });

        // A causal minimum-phase impulse begins at tap zero. Symmetric windows
        // (the default Blackman) are zero at tap zero and would erase the
        // leading energy, destroying the minimum-phase result. Truncation is
        // sufficient for that phase type.
        let fir_design_config = math_audio_iir_fir::FirDesignConfig {
            n_taps,
            sample_rate,
            phase: phase_type,
            pre_ringing,
            window: if phase_type == FirPhase::Minimum {
                math_audio_iir_fir::WindowType::Rectangular
            } else {
                math_audio_iir_fir::FirDesignConfig::default().window
            },
            ..Default::default()
        };

        let coeffs = math_audio_iir_fir::generate_fir_from_response(
            &freqs,
            &magnitude_db,
            &fir_design_config,
        );
        checked_coefficients(coeffs, n_taps)
    }
}

/// Generate a correction FIR and apply an optional group-delay alignment
/// target to the selected channel.
/// All channels receive the common causal padding returned by
/// `gd_delay_padding_samples(&target.per_channel_delay_ms, sample_rate)`.
/// Consumers must include that padding in their absolute latency report.
pub fn generate_fir_correction_with_gd_target_prepared(
    measurement: &Curve,
    config: &OptimizerConfig,
    target_curve: &Curve,
    sample_rate: f64,
    gd_target: Option<&crate::gd_opt::GdAlignmentTarget>,
    channel_index: usize,
) -> Result<Vec<f64>, Box<dyn Error>> {
    let mut coeffs =
        generate_fir_correction_prepared(measurement, config, target_curve, sample_rate)?;
    if let Some(target) = gd_target {
        let delay_ms = target
            .per_channel_delay_ms
            .get(channel_index)
            .copied()
            .ok_or("missing channel in FIR group-delay target")?;
        let padding = gd_delay_padding_samples(&target.per_channel_delay_ms, sample_rate);
        coeffs = realize_gd_fir_delay(&coeffs, delay_ms, sample_rate, padding)?.coefficients;
    }
    if gd_target
        .and_then(|target| target.per_channel_polarity_inverted.get(channel_index))
        .copied()
        .unwrap_or(false)
    {
        for coefficient in &mut coeffs {
            *coefficient = -*coefficient;
        }
    }
    Ok(coeffs)
}
