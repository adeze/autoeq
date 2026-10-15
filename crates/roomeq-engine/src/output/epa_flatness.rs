//! EPA spectral-error comparisons on an existing configured reference.

// Rust guideline compliant 2026-02-21

use autoeq_core::Curve;
use autoeq_optim::loss::enhanced_weights::PreparedWeightedLoss;
use ndarray::Array1;
use roomeq_model::{EpaConfig, FrozenGridResidualEvidence, StageCheck, StageCheckKind};

// Numeric comparison tolerance only; this is not a listening threshold.
const ROUNDING_TOLERANCE_DB: f64 = 1e-9;

/// Compare configured EPA spectral error using an existing frozen target reference.
///
/// Uses the optimizer's ERB/band-weighted flatness kernel without deadband,
/// null suppression, or a newly fitted normalization. Transfer-spectrum
/// composite preference, loudness, and roughness descriptors do not enter this
/// calculation. The returned check describes target error, not listener benefit.
///
/// # Errors
/// Returns an error for mismatched receipt/source data, missing delivered support,
/// invalid spectral weights, inactive weighted bands, or nonfinite losses.
pub fn compute_epa_spectral_flatness(
    pre: &Curve,
    post: &Curve,
    target: Option<&Curve>,
    receipt: &FrozenGridResidualEvidence,
    config: &EpaConfig,
) -> Result<StageCheck, String> {
    let config = crate::config_adapter::to_optimizer_epa(config);
    let mixes = [config.flatness_erb_weight, config.flatness_band_weight];
    let bands = &config.flatness_band_weights;
    let weights = [bands.bass_weight, bands.mid_weight, bands.treble_weight];
    if mixes
        .iter()
        .chain(weights.iter())
        .any(|weight| !weight.is_finite() || *weight < 0.0)
        || mixes.iter().sum::<f64>() <= 0.0
        || !mixes.iter().sum::<f64>().is_finite()
        || [
            (bands.bass_min, bands.bass_max),
            (bands.mid_min, bands.mid_max),
            (bands.treble_min, bands.treble_max),
        ]
        .iter()
        .any(|(low, high)| !low.is_finite() || !high.is_finite() || *low <= 0.0 || high < low)
    {
        return Err("invalid EPA spectral-flatness configuration".into());
    }
    let samples = roomeq_quality::frozen_grid_residual_samples(pre, post, target, receipt)?;
    if config.flatness_erb_weight == 0.0
        && !samples.frequencies_hz.iter().any(|frequency| {
            let weight = if *frequency >= bands.bass_min && *frequency <= bands.bass_max {
                bands.bass_weight
            } else if *frequency >= bands.mid_min && *frequency <= bands.mid_max {
                bands.mid_weight
            } else if *frequency >= bands.treble_min && *frequency <= bands.treble_max {
                bands.treble_weight
            } else {
                0.0
            };
            weight > 0.0
        })
    {
        return Err("EPA spectral-flatness bands have no weighted measured support".into());
    }
    let frequencies = Array1::from(samples.frequencies_hz);
    let kernel = PreparedWeightedLoss::new(
        &frequencies,
        receipt.evaluated_band_hz[0],
        receipt.evaluated_band_hz[1],
        config.flatness_band_weights.clone(),
    );
    // Spectral flatness is a shape metric: remove each error's own
    // broadband offset before scoring. Broadband level loss remains
    // gated by useful-output and seat-benefit evidence, which use the
    // uncentered residuals.
    let evaluate =
        |error: Vec<f64>| kernel.evaluate_centered(&Array1::from(error), mixes[0], mixes[1]);
    let before = evaluate(samples.pre_error_db);
    let after = evaluate(samples.post_error_db);
    if !before.is_finite() || !after.is_finite() {
        return Err("nonfinite EPA spectral-flatness loss".into());
    }
    let delta = after - before;
    Ok(StageCheck {
        id: "epa_spectral_flatness".into(),
        kind: StageCheckKind::Quality,
        passed: delta <= ROUNDING_TOLERANCE_DB,
        observed: Some(delta),
        limit: Some(ROUNDING_TOLERANCE_DB),
        diagnostic: Some(
            serde_json::json!({
                "method": "configured_frozen_epa_spectral_flatness_v1",
                "scope": "target_spectral_error_not_measured_listener_preference",
                "comparison": "at_most", "unit": "dB",
                "signed_violation_margin": delta - ROUNDING_TOLERANCE_DB,
                "pre_loss_db": before, "post_loss_db": after,
                "frequency_grid_sha256": receipt.frequency_grid_sha256,
                "baseline_samples_sha256": receipt.baseline_samples_sha256,
                "target_samples_sha256": receipt.target_samples_sha256,
                "sample_count": receipt.sample_count,
                "evaluated_band_hz": receipt.evaluated_band_hz,
                "normalized_to_pre": receipt.normalized_to_pre,
                "level_offset_db": receipt.level_offset_db,
                "post_reference_gain_db": receipt.post_reference_gain_db,
                "erb_weight": mixes[0], "band_weight": mixes[1],
                "band_weights": bands,
            })
            .to_string(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epa_spectral_flatness_protects_target_error_and_ignores_transfer_descriptors() {
        let pre = Curve {
            freq: vec![20.0, 100.0, 1000.0, 2000.0, 10_000.0, 20_000.0].into(),
            spl: vec![80.0, 82.0, 90.0, 82.0, 80.0, 78.0].into(),
            ..Default::default()
        };
        let target = Curve {
            spl: Array1::zeros(pre.freq.len()),
            ..pre.clone()
        };
        let score = roomeq_quality::evaluate_acoustic_quality(
            std::slice::from_ref(&pre),
            std::slice::from_ref(&pre),
            &[],
            &[],
            Some(&target),
            roomeq_quality::QualityEvaluationConfig {
                min_freq_hz: 20.0,
                max_freq_hz: 20_000.0,
                normalize_level: true,
                schroeder_hz: None,
            },
            Default::default(),
        )
        .unwrap();
        let receipt = &score.training.frozen_grid_residual[0];
        let corrected = Curve {
            spl: Array1::from_elem(pre.freq.len(), receipt.level_offset_db),
            ..pre.clone()
        };
        let worse = Curve {
            spl: pre.spl.mapv(|level| 2.0 * level - receipt.level_offset_db),
            ..pre.clone()
        };
        let config = EpaConfig::default();
        let identity =
            compute_epa_spectral_flatness(&pre, &pre, Some(&target), receipt, &config).unwrap();
        assert_eq!(identity.observed, Some(0.0));
        let healthy =
            compute_epa_spectral_flatness(&pre, &corrected, Some(&target), receipt, &config)
                .unwrap();
        assert!(healthy.passed && healthy.observed.unwrap() < -1.0);
        let regression =
            compute_epa_spectral_flatness(&pre, &worse, Some(&target), receipt, &config).unwrap();
        assert!(!regression.passed && regression.observed.unwrap() > 1.0);
        let mut diagnostic_only = config;
        diagnostic_only.listening_level_phon = 95.0;
        diagnostic_only.activity_weight = 100.0;
        diagnostic_only.target_sharpness = 4.0;
        assert_eq!(
            healthy,
            compute_epa_spectral_flatness(
                &pre,
                &corrected,
                Some(&target),
                receipt,
                &diagnostic_only
            )
            .unwrap()
        );
        let mut invalid = diagnostic_only;
        invalid.flatness_erb_weight = -1.0;
        assert!(
            compute_epa_spectral_flatness(&pre, &corrected, Some(&target), receipt, &invalid)
                .is_err()
        );
        invalid.flatness_erb_weight = 0.0;
        invalid.flatness_band_weight = 1.0;
        invalid.flatness_band_weights.bass_weight = 0.0;
        invalid.flatness_band_weights.mid_weight = 0.0;
        invalid.flatness_band_weights.treble_weight = 0.0;
        assert!(
            compute_epa_spectral_flatness(&pre, &corrected, Some(&target), receipt, &invalid)
                .is_err()
        );
    }
}
