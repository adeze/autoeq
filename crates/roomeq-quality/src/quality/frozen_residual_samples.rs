//! Recover target residuals on an already recorded, immutable comparison basis.

// Rust guideline compliant 2026-02-21

use super::{
    QualityEvaluationConfig, aligned_samples, curve_max_freq, frozen_samples_sha256, sample_log,
};
use autoeq_core::Curve;
use roomeq_model::FrozenGridResidualEvidence;

/// Target errors sharing a recorded baseline grid and level reference.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenResidualSamples {
    /// Frequencies bound by the receipt's frequency-grid digest.
    pub frequencies_hz: Vec<f64>,
    /// Baseline minus target minus the recorded baseline level offset.
    pub pre_error_db: Vec<f64>,
    /// Delivered error using the same offset and explicit permitted gain.
    pub post_error_db: Vec<f64>,
}

/// Recover target errors without fitting another level reference or grid.
///
/// The receipt must come from this baseline and target. Native sample hashes,
/// grid identity, and actual delivered coverage are checked before sampling.
/// Additional candidate bins cannot alter the comparison basis. Safety checks
/// on candidate-native bins remain the caller's separate responsibility.
///
/// # Errors
/// Returns an error for invalid curves or receipt numbers, mismatched baseline,
/// target or grid identities, or incomplete delivered support. This checks
/// consistency with a trusted receipt, not capture authentication.
pub fn frozen_grid_residual_samples(
    pre: &Curve,
    post: &Curve,
    target: Option<&Curve>,
    receipt: &FrozenGridResidualEvidence,
) -> Result<FrozenResidualSamples, String> {
    pre.validate("frozen residual baseline")
        .map_err(|error| error.to_string())?;
    post.validate("frozen residual delivered")
        .map_err(|error| error.to_string())?;
    if let Some(target) = target {
        target
            .validate("frozen residual target")
            .map_err(|error| error.to_string())?;
    }
    let config = QualityEvaluationConfig {
        min_freq_hz: receipt.evaluated_band_hz[0],
        max_freq_hz: receipt.evaluated_band_hz[1],
        schroeder_hz: None,
        normalize_level: receipt.normalized_to_pre,
    };
    config.validate()?;
    if !receipt.level_offset_db.is_finite() || !receipt.post_reference_gain_db.is_finite() {
        return Err("nonfinite frozen residual reference".into());
    }
    if !receipt.post_covers_baseline_grid
        || post.freq[0] > config.min_freq_hz
        || curve_max_freq(post)? < config.max_freq_hz
    {
        return Err("delivered response does not cover the frozen residual grid".into());
    }
    let samples = aligned_samples(pre, pre, target, config)?;
    if samples.len() != receipt.sample_count
        || frozen_samples_sha256(samples.iter().map(|sample| sample.frequency))
            != receipt.frequency_grid_sha256
        || frozen_samples_sha256(samples.iter().map(|sample| sample.pre))
            != receipt.baseline_samples_sha256
        || frozen_samples_sha256(samples.iter().map(|sample| sample.target))
            != receipt.target_samples_sha256
    {
        return Err("baseline, target, or grid differs from the frozen residual receipt".into());
    }
    Ok(FrozenResidualSamples {
        frequencies_hz: samples.iter().map(|sample| sample.frequency).collect(),
        pre_error_db: samples
            .iter()
            .map(|sample| sample.pre - sample.target - receipt.level_offset_db)
            .collect(),
        post_error_db: samples
            .iter()
            .map(|sample| {
                sample_log(post, sample.frequency, &post.spl)
                    - sample.target
                    - receipt.level_offset_db
                    - receipt.post_reference_gain_db
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{evaluate_acoustic_quality_with_permitted_gain, metrics::weighted_rms};

    fn curve(frequencies: &[f64], levels: &[f64]) -> Curve {
        Curve {
            freq: frequencies.to_vec().into(),
            spl: levels.to_vec().into(),
            ..Default::default()
        }
    }

    fn receipt(pre: &Curve, post: &Curve, target: &Curve, gain: f64) -> FrozenGridResidualEvidence {
        evaluate_acoustic_quality_with_permitted_gain(
            std::slice::from_ref(pre),
            std::slice::from_ref(post),
            &[],
            &[],
            Some(target),
            QualityEvaluationConfig {
                min_freq_hz: 20.0,
                max_freq_hz: 20_000.0,
                normalize_level: true,
                schroeder_hz: None,
            },
            Default::default(),
            gain,
        )
        .unwrap()
        .training
        .frozen_grid_residual
        .remove(0)
    }

    #[test]
    fn frozen_residual_samples_reproduce_recorded_scores_without_releveling() {
        let frequencies = [20.0, 80.0, 200.0, 1000.0, 2000.0, 20_000.0];
        let pre = curve(&frequencies, &[87.0, 82.0, 76.0, 80.0, 78.0, 73.0]);
        let post = curve(&frequencies, &[81.0, 79.0, 77.0, 79.0, 78.0, 75.0]);
        let target = curve(&[20.0, 20_000.0], &[6.0, -3.0]);
        let fixed = receipt(&pre, &post, &target, 0.0);
        let samples = frozen_grid_residual_samples(&pre, &post, Some(&target), &fixed).unwrap();
        let mut dense_frequencies = frequencies.to_vec();
        dense_frequencies.extend([40.0, 350.0, 1500.0, 10_000.0]);
        dense_frequencies.sort_by(f64::total_cmp);
        let dense =
            autoeq_core::curve_transforms::interpolate_log_space(&dense_frequencies.into(), &post);
        let densified = frozen_grid_residual_samples(&pre, &dense, Some(&target), &fixed).unwrap();
        assert_eq!(
            densified, samples,
            "candidate bins changed the frozen basis"
        );
        assert!(
            (weighted_rms(&samples.frequencies_hz, &samples.pre_error_db)
                - fixed.pre_weighted_rms_db)
                .abs()
                < 1e-12
        );
        assert!(
            (weighted_rms(&samples.frequencies_hz, &samples.post_error_db)
                - fixed.post_weighted_rms_db.unwrap())
            .abs()
                < 1e-12
        );
        let mut down = post.clone();
        down.spl.mapv_inplace(|value| value - 12.0);
        let shifted = frozen_grid_residual_samples(&pre, &down, Some(&target), &fixed).unwrap();
        assert_eq!(shifted.pre_error_db, samples.pre_error_db);
        for (actual, previous) in shifted.post_error_db.iter().zip(&samples.post_error_db) {
            assert!((actual - previous + 12.0).abs() < 1e-12);
        }
        let permitted = receipt(&pre, &down, &target, -12.0);
        let restored =
            frozen_grid_residual_samples(&pre, &down, Some(&target), &permitted).unwrap();
        for (actual, expected) in restored.post_error_db.iter().zip(&samples.post_error_db) {
            assert!((actual - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn frozen_residual_samples_preserve_absolute_targets() {
        let pre = curve(&[20.0, 1000.0, 20_000.0], &[80.0; 3]);
        let target = curve(&[20.0, 20_000.0], &[86.0; 2]);
        let post = curve(&[20.0, 1000.0, 20_000.0], &[86.0; 3]);
        let score = evaluate_acoustic_quality_with_permitted_gain(
            std::slice::from_ref(&pre),
            std::slice::from_ref(&post),
            &[],
            &[],
            Some(&target),
            QualityEvaluationConfig {
                min_freq_hz: 20.0,
                max_freq_hz: 20_000.0,
                normalize_level: false,
                schroeder_hz: None,
            },
            Default::default(),
            0.0,
        )
        .unwrap();
        let fixed = &score.training.frozen_grid_residual[0];
        let samples = frozen_grid_residual_samples(&pre, &post, Some(&target), fixed).unwrap();
        assert_eq!(fixed.level_offset_db, 0.0);
        assert_eq!(samples.pre_error_db, vec![-6.0; 3]);
        assert_eq!(samples.post_error_db, vec![0.0; 3]);
    }

    #[test]
    fn frozen_residual_samples_reject_changed_sources_and_missing_coverage() {
        let pre = curve(&[20.0, 1000.0, 2000.0, 20_000.0], &[81.0, 80.0, 79.0, 75.0]);
        let target = curve(&[20.0, 20_000.0], &[0.0, 0.0]);
        let fixed = receipt(&pre, &pre, &target, 0.0);
        let mut changed = pre.clone();
        changed.spl[0] += 0.1;
        assert!(frozen_grid_residual_samples(&changed, &pre, Some(&target), &fixed).is_err());
        let changed_target = curve(&[20.0, 20_000.0], &[0.0, 1.0]);
        assert!(frozen_grid_residual_samples(&pre, &pre, Some(&changed_target), &fixed).is_err());
        let short = curve(&[20.0, 1000.0], &[80.0, 80.0]);
        assert!(frozen_grid_residual_samples(&pre, &short, Some(&target), &fixed).is_err());
        let mut invalid = fixed.clone();
        invalid.frequency_grid_sha256.clear();
        assert!(frozen_grid_residual_samples(&pre, &pre, Some(&target), &invalid).is_err());
    }
}
