use super::build::build_bounds;
use super::channel_measurement_input::gd_loss;
use super::compute::compute_bootstrap_improvements;
use super::compute::compute_mean_coherence;
use super::compute::compute_sum_gd_rms;
use super::decode::decode_per_channel;
use super::gd_opt_config::GdOptConfig;
use super::gd_opt_config::param_count;
use super::misc::BOOTSTRAP_SIGMA_THRESHOLD;
use super::misc::MAX_AP_BUDGET;
use super::misc::same_frequency_grid;
use super::types::ChannelMeasurementInput;
use super::types::GroupDelayOptResult;
use super::types::encode_result_as_params;
use super::types::normalize_per_channel_controls;
use autoeq_optim::optim::scalar::{ScalarOptimConfig, optimize_bounded_scalar};
use roomeq_model::{MixedModeConfig, ProcessingMode};

/// Numerical support does not qualify timing or phase accuracy.
fn resolution_bounded_config<'a>(
    inputs: impl IntoIterator<Item = &'a ChannelMeasurementInput>,
    config: &GdOptConfig,
) -> GdOptConfig {
    let mut bounded = config.clone();
    if bounded.ap_per_channel == 0 {
        return bounded;
    }
    let cap = ChannelMeasurementInput::combined_resolution(inputs).and_then(|support| {
        let [low, high] = support.support_band_hz()?;
        bounded.ap_min_freq = bounded.ap_min_freq.max(low);
        bounded.ap_max_freq = bounded.ap_max_freq.min(high);
        let width = support.max_width_octaves([bounded.ap_min_freq, bounded.ap_max_freq])?;
        support.max_peak_q(bounded.ap_max_freq, bounded.sample_rate, width)
    });
    match cap.filter(|q| *q >= bounded.ap_min_q) {
        Some(q) if bounded.ap_max_q >= bounded.ap_min_q => {
            bounded.ap_max_q = bounded.ap_max_q.min(q);
        }
        _ => {
            bounded.ap_per_channel = 0;
            log::warn!(
                "GD all-pass filters disabled: original sampling/smoothing support cannot satisfy the requested band and Q floor; delay/polarity remain available"
            );
        }
    }
    bounded
}

fn bootstrap_improvement_is_significant(mean_improvement: f64, sigma: f64) -> bool {
    mean_improvement > 0.0
        && (sigma <= 1e-15 || (mean_improvement / sigma) > BOOTSTRAP_SIGMA_THRESHOLD)
}

/// Run the group-delay optimiser on a set of channel measurements.
///
/// Returns `Err` if fewer than 2 channels are provided or measurements are
/// incompatible.
pub fn optimize_group_delay(
    channels: &[ChannelMeasurementInput],
    band: (f64, f64),
    config: &GdOptConfig,
) -> Result<GroupDelayOptResult, String> {
    optimize_group_delay_with_initial(channels, band, config, None)
}

fn optimize_group_delay_with_initial(
    channels: &[ChannelMeasurementInput],
    band: (f64, f64),
    config: &GdOptConfig,
    initial_result: Option<&GroupDelayOptResult>,
) -> Result<GroupDelayOptResult, String> {
    let n_ch = channels.len();
    if n_ch < 2 {
        return Err("GD-Opt requires at least 2 channels".into());
    }

    // Validate all channels share the same frequency grid values.
    let n_freq = channels[0].freq.len();
    for (i, ch) in channels.iter().enumerate() {
        if ch.freq.len() != n_freq
            || ch.spl.len() != n_freq
            || ch.phase.len() != n_freq
            || ch.coherence.len() != n_freq
        {
            return Err(format!("Channel {} has inconsistent array lengths", i));
        }
        if i > 0 && !same_frequency_grid(&channels[0].freq, &ch.freq) {
            return Err(format!(
                "Channel {} frequency grid does not match the reference channel",
                i
            ));
        }
    }

    let bounded_config = resolution_bounded_config(channels, config);
    let config = &bounded_config;

    // Find indices within band
    let band_indices: Vec<usize> = (0..n_freq)
        .filter(|&i| channels[0].freq[i] >= band.0 && channels[0].freq[i] <= band.1)
        .collect();

    if band_indices.is_empty() {
        return Err("No frequency bins within the specified band".into());
    }

    // Compute mean coherence (weighted across all channels)
    let mean_coherence = compute_mean_coherence(channels, &band_indices);

    // Compute pre-optimisation sum GD RMS
    let identity_config = GdOptConfig {
        ap_per_channel: 0,
        optimize_polarity: false,
        ..config.clone()
    };
    let identity_params = vec![0.0; param_count(n_ch, &identity_config)];
    let sum_gd_pre_rms_ms =
        compute_sum_gd_rms(channels, &identity_params, &band_indices, &identity_config);

    // Build bounds for DE
    let bounds = build_bounds(n_ch, config);

    let channels_ref = channels;
    let band_indices_ref = &band_indices;
    let config_ref = config;

    let loss_fn = |x: &[f64]| -> f64 { gd_loss(channels_ref, x, band_indices_ref, config_ref) };

    let mut initial = initial_result
        .map(|result| encode_result_as_params(result, config))
        .unwrap_or_else(|| {
            let empty = GroupDelayOptResult {
                band,
                per_channel: (0..n_ch)
                    .map(|_| super::types::ChannelGdResult {
                        delay_ms: 0.0,
                        polarity_inverted: false,
                        ap_filters: Vec::new(),
                        channel_gd_pre_rms_ms: 0.0,
                        channel_gd_post_rms_ms: 0.0,
                    })
                    .collect(),
                sum_gd_pre_rms_ms: 0.0,
                sum_gd_post_rms_ms: 0.0,
                mean_coherence: 0.0,
                improvement_db: 0.0,
            };
            encode_result_as_params(&empty, config)
        });
    if initial_result.is_some() {
        let stride = 1 + config.ap_per_channel * 2 + usize::from(config.optimize_polarity);
        let delays: Vec<_> = (0..n_ch).map(|channel| initial[channel * stride]).collect();
        let min_delay = delays.iter().copied().fold(f64::INFINITY, f64::min);
        let max_delay = delays.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let common_delay = 0.5 * (min_delay + max_delay);
        for channel in 0..n_ch {
            initial[channel * stride] = (initial[channel * stride] - common_delay)
                .clamp(-config.max_delay_ms, config.max_delay_ms);
        }
    }
    let report = optimize_bounded_scalar(
        &bounds,
        &initial,
        &ScalarOptimConfig {
            algorithm: config.algorithm.clone(),
            max_iter: config.max_iter,
            population: config.popsize,
            tolerance: config.tol,
            atolerance: config.tol,
            strategy: config.strategy.clone(),
            seed: config.seed,
        },
        loss_fn,
    )?;

    let best_params = report.x.as_slice();

    // Acceptance-side budget record: the emitted delays/polarity/all-pass
    // controls must honor the search budgets.
    let (lower, upper): (Vec<f64>, Vec<f64>) = bounds.iter().copied().unzip();
    autoeq_optim::optim::verify_joint_budgets("group-delay", best_params, &lower, &upper)
        .map_err(|reason| format!("group-delay candidate refused at emission: {reason}"))?;

    // Compute post-optimisation sum GD RMS
    let sum_gd_post_rms_ms = compute_sum_gd_rms(channels, best_params, &band_indices, config);

    let improvement_db = if sum_gd_pre_rms_ms < 1e-15 {
        0.0 // Already aligned, no improvement possible
    } else if sum_gd_post_rms_ms > 1e-15 {
        20.0 * (sum_gd_pre_rms_ms / sum_gd_post_rms_ms).log10()
    } else {
        120.0 // Cap at a large but finite value
    };

    // Decode per-channel results and normalize unidentifiable common controls
    // before reporting/applying them.
    let mut per_channel = decode_per_channel(channels, best_params, &band_indices, config);
    normalize_per_channel_controls(&mut per_channel);

    Ok(GroupDelayOptResult {
        band,
        per_channel,
        sum_gd_pre_rms_ms,
        sum_gd_post_rms_ms,
        mean_coherence,
        improvement_db,
    })
}

/// Run the group-delay optimiser with adaptive AP budget (§3.3).
///
/// Instead of a fixed AP count, starts with delay-only (0 APs), then
/// incrementally adds APs up to `MAX_AP_BUDGET`, accepting each only if it
/// passes the bootstrap significance test across the per-sweep realisations.
///
/// `sweep_realisations` contains N independent measurement sets (one per sweep).
/// The main `channels` is the coherence-averaged measurement used for fitting.
pub fn optimize_group_delay_adaptive(
    channels: &[ChannelMeasurementInput],
    sweep_realisations: &[Vec<ChannelMeasurementInput>],
    band: (f64, f64),
    config: &GdOptConfig,
) -> Result<GroupDelayOptResult, String> {
    if sweep_realisations.len() < 2 {
        return Err("Adaptive AP bootstrap requires at least 2 sweep realisations (N >= 2)".into());
    }
    if sweep_realisations.len() < 4 {
        log::info!(
            "GD-Opt adaptive bootstrap has only {} sweep realisations; variance estimate is unstable below N=4",
            sweep_realisations.len()
        );
    }

    // Use every fitting and bootstrap capture before deciding trial dimensions.
    let bounded_config = resolution_bounded_config(
        channels.iter().chain(sweep_realisations.iter().flatten()),
        config,
    );
    let config = &bounded_config;

    // Start with delay-only (0 APs)
    let mut best_config = GdOptConfig {
        ap_per_channel: 0,
        ..config.clone()
    };
    let mut best_result = optimize_group_delay(channels, band, &best_config)?;

    // Incrementally try adding APs
    for k in 1..=config.ap_per_channel.min(MAX_AP_BUDGET) {
        let trial_config = GdOptConfig {
            ap_per_channel: k,
            ..config.clone()
        };

        let trial_result =
            optimize_group_delay_with_initial(channels, band, &trial_config, Some(&best_result))?;

        // Bootstrap test: for each sweep realisation, compute GD RMS
        // with and without the k-th AP filter.
        let improvements = compute_bootstrap_improvements(
            sweep_realisations,
            band,
            &best_config,
            &best_result,
            &trial_config,
            &trial_result,
        )?;

        let n = improvements.len() as f64;
        let mean_improvement = improvements.iter().sum::<f64>() / n;
        let variance = improvements
            .iter()
            .map(|&x| (x - mean_improvement).powi(2))
            .sum::<f64>()
            / (n - 1.0);
        let sigma = variance.sqrt();

        // Identical positive improvements are maximally consistent evidence,
        // not insignificant merely because their sample variance is zero.
        let significant = bootstrap_improvement_is_significant(mean_improvement, sigma);
        log::warn!(
            "GD-Opt adaptive AP trial k={k}: mean bootstrap improvement={mean_improvement:.4} ms, sigma={sigma:.4} ms, significant={significant}, aggregate RMS={:.4} -> {:.4} ms",
            best_result.sum_gd_post_rms_ms,
            trial_result.sum_gd_post_rms_ms,
        );

        if significant && trial_result.sum_gd_post_rms_ms < best_result.sum_gd_post_rms_ms {
            best_result = trial_result;
            best_config = trial_config;
        } else {
            // No significant improvement — stop adding APs
            break;
        }
    }

    Ok(best_result)
}

/// Run the group-delay optimiser with mode-specific behaviour (§3.7).
///
/// Dispatches based on `ProcessingMode`:
/// - `LowLatency`, `WarpedIir`, `KautzModal`: Full optimisation (delays + APs).
/// - `Hybrid`: Same as LowLatency but asserts `band_hi ≤ mixed_config.crossover_freq`.
/// - `MixedPhase`: Inter-channel alignment only (1 AP max per channel).
/// - `PhaseLinear`: Not applicable (returns error — use GD-3b FIR path).
pub fn optimize_group_delay_for_mode(
    channels: &[ChannelMeasurementInput],
    band: (f64, f64),
    config: &GdOptConfig,
    processing_mode: &ProcessingMode,
    mixed_mode_config: Option<&MixedModeConfig>,
) -> Result<GroupDelayOptResult, String> {
    match processing_mode {
        ProcessingMode::LowLatency | ProcessingMode::WarpedIir | ProcessingMode::KautzModal => {
            optimize_group_delay(channels, band, config)
        }

        ProcessingMode::Hybrid => {
            // Assert band_hi does not straddle the IIR/FIR crossover
            let xo_freq = mixed_mode_config.map(|m| m.crossover_freq).unwrap_or(300.0);

            if band.1 > xo_freq {
                return Err(format!(
                    "Hybrid mode: GD-Opt band_hi ({:.1} Hz) exceeds mixed_config crossover \
                     ({:.1} Hz). AP filters must stay in the IIR band.",
                    band.1, xo_freq,
                ));
            }

            optimize_group_delay(channels, band, config)
        }

        ProcessingMode::MixedPhase => {
            // After per-channel excess-phase FIR correction, only inter-channel
            // alignment remains. Typically 1 delay per channel, at most 1 AP.
            let mixed_phase_config = GdOptConfig {
                ap_per_channel: config.ap_per_channel.min(1),
                ..config.clone()
            };
            optimize_group_delay(channels, band, &mixed_phase_config)
        }

        ProcessingMode::PhaseLinear => Err("PhaseLinear mode does not use IIR AP filters. \
             Use the FIR path (GD-3b) with GdAlignmentTarget instead."
            .into()),
    }
}

#[cfg(test)]
mod significance_tests {
    use super::bootstrap_improvement_is_significant;

    #[test]
    fn identical_positive_bootstrap_improvements_are_significant() {
        assert!(bootstrap_improvement_is_significant(0.25, 0.0));
        assert!(!bootstrap_improvement_is_significant(0.0, 0.0));
        assert!(!bootstrap_improvement_is_significant(-0.25, 0.0));
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::*;
    use ndarray::Array1;

    fn source_input() -> ChannelMeasurementInput {
        let source = autoeq_core::Curve {
            freq: Array1::from_vec(vec![20.0, 40.0, 80.0, 160.0, 200.0]),
            spl: Array1::zeros(5),
            ..Default::default()
        };
        let dense = autoeq_core::interpolate_response(&Array1::linspace(20.0, 200.0, 128), &source);
        ChannelMeasurementInput {
            freq: dense.freq,
            spl: dense.spl,
            phase: Array1::zeros(128),
            coherence: Array1::ones(128),
            resolution: dense.resolution,
        }
    }

    #[test]
    fn original_support_bounds_gd_allpass_search_and_emission() {
        let channels = vec![source_input(); 2];
        let config = GdOptConfig {
            ap_per_channel: 1,
            max_iter: 20,
            popsize: 4,
            seed: Some(84),
            ..Default::default()
        };
        let bounded = resolution_bounded_config(&channels, &config);
        assert_eq!(bounded.ap_max_freq, 200.0);
        assert!(bounded.ap_max_q < 0.67);
        assert_eq!(bounded.ap_per_channel, 1);
        let result = optimize_group_delay(&channels, (20.0, 200.0), &config).unwrap();
        for channel in &result.per_channel {
            assert_eq!(channel.ap_filters.len(), 1);
            for ap in &channel.ap_filters {
                assert!(ap.q <= bounded.ap_max_q);
                assert!((20.0..=200.0).contains(&ap.freq));
            }
        }
    }

    #[test]
    fn bootstrap_smoothing_floor_disables_gd_allpass_trials() {
        let channels = vec![source_input(); 2];
        let mut sweeps = vec![channels.clone(); 2];
        let support = sweeps[1][1].resolution.as_ref().unwrap();
        sweeps[1][1].resolution = support.with_smoothing(vec![(20.0, 4.0), (200.0, 4.0)]);
        let config = GdOptConfig {
            ap_per_channel: 2,
            max_iter: 20,
            popsize: 4,
            seed: Some(84),
            ..Default::default()
        };
        let bounded =
            resolution_bounded_config(channels.iter().chain(sweeps.iter().flatten()), &config);
        assert_eq!(bounded.ap_per_channel, 0);
        assert_eq!(bounded.max_delay_ms, config.max_delay_ms);
        assert_eq!(bounded.optimize_polarity, config.optimize_polarity);
        let result =
            optimize_group_delay_adaptive(&channels, &sweeps, (20.0, 200.0), &config).unwrap();
        assert!(
            result
                .per_channel
                .iter()
                .all(|channel| channel.ap_filters.is_empty())
        );
    }

    #[test]
    fn fir_alignment_target_does_not_invent_unemitted_sections() {
        let channels = vec![source_input(); 2];
        let config = GdOptConfig {
            ap_per_channel: 0,
            max_iter: 20,
            popsize: 4,
            seed: Some(84),
            ..Default::default()
        };
        let result = optimize_group_delay(&channels, (20.0, 200.0), &config).unwrap();
        let actual = super::super::build::build_gd_alignment_target(&channels, &result, &config);
        let requested = GdOptConfig {
            ap_per_channel: 2,
            ..config
        };
        let target = super::super::build::build_gd_alignment_target(&channels, &result, &requested);
        assert!(!target.sum_gd_reference_ms.is_empty());
        assert_eq!(actual.sum_gd_reference_ms, target.sum_gd_reference_ms);
        assert_eq!(actual.freq, target.freq);
    }
}
