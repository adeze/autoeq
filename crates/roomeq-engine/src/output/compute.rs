use super::misc::same_frequency_grid;
use autoeq_optim::loss::epa::score::{
    EpaChannelRole, compute_epa_multichannel_normalized, compute_epa_normalized,
    epa_channel_energy_weight, infer_epa_channel_role, reference_band_energy_db,
};
use roomeq_model::EpaConfig;
use roomeq_model::{
    ChannelDspChain, CurveData, EpaChannelMetrics, EpaMultichannelMetrics, LevelMatchedEpaMetrics,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Compute per-channel EPA metrics (pre-EQ and post-EQ) from each
/// channel's `initial_curve` and `final_curve`.
///
/// `CurveData.spl` is mean-subtracted around 1–2 kHz (level-relative),
/// so we call [`compute_epa_normalized`] which denormalizes the curve
/// against `config.listening_level_phon` before running the
/// psychoacoustic model. Without that calibration step the loudness
/// and loudness-balance components would be dominated by the absolute
/// threshold of hearing.
///
/// Returns `None` if no channel has both curves populated.
pub fn compute_epa_per_channel(
    channels: &HashMap<String, ChannelDspChain>,
    config: &EpaConfig,
) -> Option<HashMap<String, EpaChannelMetrics>> {
    let config = crate::config_adapter::to_optimizer_epa(config);
    let mut out: HashMap<String, EpaChannelMetrics> = HashMap::new();
    for (name, chain) in channels {
        let (Some(initial), Some(final_)) = (&chain.initial_curve, &chain.final_curve) else {
            continue;
        };
        let pre = crate::report_adapter::to_epa_score(compute_epa_normalized(
            &initial.freq,
            &initial.spl,
            &config,
        ));
        let post = crate::report_adapter::to_epa_score(compute_epa_normalized(
            &final_.freq,
            &final_.spl,
            &config,
        ));
        let level_matched = level_matched_epa(&[(initial, final_, EpaChannelRole::Main)], &config);
        out.insert(
            name.clone(),
            EpaChannelMetrics {
                pre,
                post,
                level_matched,
            },
        );
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Compute aggregate EPA metrics from all channel curves using BS.1770-style
/// channel energy weights.
///
/// This is a frequency-response approximation for room-EQ reports. It does
/// not replace time-domain LUFS metering, but it avoids treating stereo or
/// surround systems as unrelated monaural measurements.
pub fn compute_epa_multichannel(
    channels: &HashMap<String, ChannelDspChain>,
    config: &EpaConfig,
) -> Option<EpaMultichannelMetrics> {
    let config = crate::config_adapter::to_optimizer_epa(config);
    let mut entries: Vec<_> = channels
        .iter()
        .filter_map(|(name, chain)| {
            let (Some(initial), Some(final_)) = (&chain.initial_curve, &chain.final_curve) else {
                return None;
            };
            let role = infer_epa_channel_role(name);
            (epa_channel_energy_weight(role) > 0.0).then_some((
                name.as_str(),
                initial,
                final_,
                role,
            ))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let (_, first_initial, _, _) = entries.first()?;
    let freqs = first_initial.freq.as_slice();
    if freqs.is_empty() {
        return None;
    }

    let grids_match = entries.iter().all(|(_, initial, final_, _)| {
        same_frequency_grid(freqs, &initial.freq) && same_frequency_grid(freqs, &final_.freq)
    });
    let aligned = if grids_match {
        None
    } else {
        let curves: Vec<_> = entries
            .iter()
            .flat_map(|(_, initial, final_, _)| [*initial, *final_])
            .collect();
        Some(align_epa_curves(&curves)?)
    };
    let freqs = aligned.as_ref().map_or(freqs, |(grid, _)| grid.as_slice());

    let pre_channels: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(i, (_, initial, _, role))| {
            (
                aligned
                    .as_ref()
                    .map_or(initial.spl.as_slice(), |(_, spl)| spl[2 * i].as_slice()),
                *role,
            )
        })
        .collect();
    let post_channels: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(i, (_, _, final_, role))| {
            (
                aligned
                    .as_ref()
                    .map_or(final_.spl.as_slice(), |(_, spl)| spl[2 * i + 1].as_slice()),
                *role,
            )
        })
        .collect();

    let pre = crate::report_adapter::to_epa_score(compute_epa_multichannel_normalized(
        freqs,
        &pre_channels,
        &config,
    )?);
    let post = crate::report_adapter::to_epa_score(compute_epa_multichannel_normalized(
        freqs,
        &post_channels,
        &config,
    )?);

    Some(EpaMultichannelMetrics {
        pre,
        post,
        standard: "BS.1770-style channel energy aggregation over EPA spectra".to_string(),
        level_matched: level_matched_epa(
            &entries
                .iter()
                .map(|(_, pre, post, role)| (*pre, *post, *role))
                .collect::<Vec<_>>(),
            &config,
        ),
    })
}

/// Compare one response pair with fixed-band level-matched EPA.
///
/// Uses the baseline frequency grid and equal 1–2 kHz pressure energy.
/// Returns `None` for invalid data or insufficient shared matching-band support.
/// This spectral diagnostic does not establish measured listener preference.
pub fn compute_level_matched_epa(
    pre: &CurveData,
    post: &CurveData,
    config: &EpaConfig,
) -> Option<LevelMatchedEpaMetrics> {
    level_matched_epa(
        &[(pre, post, EpaChannelRole::Main)],
        &crate::config_adapter::to_optimizer_epa(config),
    )
}

/// A frozen baseline grid and one scalar level match for the whole channel set.
/// Per-channel calls use a singleton; aggregate calls preserve channel balance.
fn level_matched_epa(
    entries: &[(&CurveData, &CurveData, EpaChannelRole)],
    config: &autoeq_optim::loss::epa::score::EpaConfig,
) -> Option<LevelMatchedEpaMetrics> {
    let (first, _, _) = *entries.first()?;
    let valid = |curve: &CurveData| {
        curve.freq.len() >= 2
            && curve.freq.len() == curve.spl.len()
            && curve
                .freq
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && curve.freq.windows(2).all(|pair| pair[0] < pair[1])
            && curve.spl.iter().all(|value| value.is_finite())
    };
    if entries
        .iter()
        .any(|(pre, post, _)| !valid(pre) || !valid(post))
    {
        return None;
    }
    let low = entries
        .iter()
        .map(|(pre, _, _)| pre.freq[0])
        .fold(0.0_f64, f64::max);
    let high = entries
        .iter()
        .map(|(pre, _, _)| *pre.freq.last().unwrap())
        .fold(f64::INFINITY, f64::min);
    if low > 1_000.0
        || high < 2_000.0
        || entries
            .iter()
            .any(|(_, post, _)| post.freq[0] > low || *post.freq.last().unwrap() < high)
    {
        return None;
    }
    let mut grid: Vec<_> = first
        .freq
        .iter()
        .copied()
        .filter(|frequency| *frequency >= low && *frequency <= high)
        .collect();
    grid.extend([low, high, 1_000.0, 2_000.0]);
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    let frequencies = ndarray::Array1::from(grid.clone());
    let sample = |curve: &CurveData| {
        let source: crate::Curve = curve.clone().into();
        autoeq_core::interpolate_log_space(&frequencies, &source)
            .spl
            .to_vec()
    };
    let mut pre: Vec<_> = entries.iter().map(|(curve, _, _)| sample(curve)).collect();
    let mut post: Vec<_> = entries.iter().map(|(_, curve, _)| sample(curve)).collect();
    let weights: Vec<_> = entries
        .iter()
        .map(|(_, _, role)| epa_channel_energy_weight(*role))
        .collect();
    let pre_reference_db = reference_band_energy_db(&grid, &pre, &weights)?;
    let post_reference_db = reference_band_energy_db(&grid, &post, &weights)?;
    for levels in &mut pre {
        for level in levels {
            *level -= pre_reference_db;
        }
    }
    for levels in &mut post {
        for level in levels {
            *level -= post_reference_db;
        }
    }
    let pre_channels: Vec<_> = pre
        .iter()
        .zip(entries)
        .map(|(levels, (_, _, role))| (levels.as_slice(), *role))
        .collect();
    let post_channels: Vec<_> = post
        .iter()
        .zip(entries)
        .map(|(levels, (_, _, role))| (levels.as_slice(), *role))
        .collect();
    let pre = crate::report_adapter::to_epa_score(compute_epa_multichannel_normalized(
        &grid,
        &pre_channels,
        config,
    )?);
    let post = crate::report_adapter::to_epa_score(compute_epa_multichannel_normalized(
        &grid,
        &post_channels,
        config,
    )?);
    let mut digest = Sha256::new();
    digest.update(b"roomeq-level-matched-epa-grid-v1\0");
    for frequency in &grid {
        digest.update(frequency.to_le_bytes());
    }
    Some(LevelMatchedEpaMetrics {
        method: "log_frequency_pressure_energy_1_2_khz_v1".into(),
        reference_band_hz: [1_000.0, 2_000.0],
        frequency_grid_sha256: digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        sample_count: grid.len(),
        pre_reference_db,
        post_reference_db,
        listening_level_phon: config.listening_level_phon,
        preference_delta: post.preference - pre.preference,
        pre,
        post,
    })
}

/// Align EPA magnitudes to the union of native bins within shared support.
/// Never extrapolate a channel or change its SPL normalization.
fn align_epa_curves(curves: &[&CurveData]) -> Option<(Vec<f64>, Vec<Vec<f64>>)> {
    if curves.is_empty()
        || curves.iter().any(|curve| {
            curve.freq.len() < 2
                || curve.freq.len() != curve.spl.len()
                || curve.freq.iter().any(|f| !f.is_finite() || *f <= 0.0)
                || curve.freq.windows(2).any(|pair| pair[0] >= pair[1])
                || curve.spl.iter().any(|spl| !spl.is_finite())
        })
    {
        return None;
    }
    let low = curves
        .iter()
        .map(|c| c.freq[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let high = curves
        .iter()
        .map(|c| c.freq[c.freq.len() - 1])
        .fold(f64::INFINITY, f64::min);
    if low >= high {
        return None;
    }
    let mut grid: Vec<_> = curves
        .iter()
        .flat_map(|c| c.freq.iter().copied())
        .filter(|f| *f >= low && *f <= high)
        .collect();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    let frequencies = ndarray::Array1::from(grid.clone());
    let levels = curves
        .iter()
        .map(|curve| {
            let source = crate::Curve {
                freq: ndarray::Array1::from(curve.freq.clone()),
                spl: ndarray::Array1::from(curve.spl.clone()),
                ..Default::default()
            };
            autoeq_core::interpolate_log_space(&frequencies, &source)
                .spl
                .to_vec()
        })
        .collect();
    Some((grid, levels))
}

/// Compute the EQ filter response curve from initial and final curves.
///
/// Returns a `CurveData` whose SPL values are `final - initial` (the correction in dB).
pub fn compute_eq_response(initial: &CurveData, final_curve: &CurveData) -> CurveData {
    let spl: Vec<f64> = final_curve
        .spl
        .iter()
        .zip(initial.spl.iter())
        .map(|(&f, &i)| f - i)
        .collect();
    CurveData {
        freq: initial.freq.clone(),
        spl,
        phase: None,
        norm_range: None,
        ..Default::default()
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::*;

    fn curve(freq: Vec<f64>) -> CurveData {
        CurveData {
            spl: freq.iter().map(|f| 3.0 * f.log2()).collect(),
            freq,
            phase: None,
            norm_range: None,
            ..Default::default()
        }
    }

    #[test]
    fn level_matched_epa_removes_gain_and_freezes_baseline_grid() {
        let pre = curve(vec![
            20.0, 100.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 20_000.0,
        ]);
        let mut post = curve(vec![
            20.0, 50.0, 100.0, 500.0, 750.0, 1_000.0, 1_500.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
        ]);
        post.spl.iter_mut().for_each(|level| *level -= 12.0);
        let config = crate::config_adapter::to_optimizer_epa(&EpaConfig::default());
        let identity = level_matched_epa(&[(&pre, &pre, EpaChannelRole::Main)], &config).unwrap();
        let matched = level_matched_epa(&[(&pre, &post, EpaChannelRole::Main)], &config).unwrap();
        assert!((matched.pre_reference_db - matched.post_reference_db - 12.0).abs() < 1e-10);
        assert!(matched.preference_delta.abs() < 1e-9);
        assert_eq!(
            matched.frequency_grid_sha256,
            identity.frequency_grid_sha256
        );
        assert_eq!(matched.sample_count, pre.freq.len());
    }

    #[test]
    fn level_matched_epa_preserves_aggregate_channel_balance() {
        let mut left = curve(vec![
            20.0, 100.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 20_000.0,
        ]);
        left.spl.fill(0.0);
        left.spl[0] = 6.0;
        left.spl[1] = 6.0;
        let mut right = left.clone();
        right.spl.fill(0.0);
        let mut boosted_left = left.clone();
        boosted_left.spl.iter_mut().for_each(|level| *level += 3.0);
        let config = crate::config_adapter::to_optimizer_epa(&EpaConfig::default());
        let singleton =
            level_matched_epa(&[(&left, &boosted_left, EpaChannelRole::Main)], &config).unwrap();
        assert!(singleton.preference_delta.abs() < 1e-9);
        let aggregate = level_matched_epa(
            &[
                (&left, &boosted_left, EpaChannelRole::Main),
                (&right, &right, EpaChannelRole::Main),
            ],
            &config,
        )
        .unwrap();
        let expected_reference = 10.0 * ((10.0_f64.powf(0.3) + 1.0) / 2.0).log10();
        assert!((aggregate.post_reference_db - expected_reference).abs() < 1e-10);
        assert!(aggregate.preference_delta.abs() > 1e-6);
    }

    #[test]
    fn level_matched_epa_ignores_zero_weight_lfe_reference_energy() {
        let main = curve(vec![20.0, 100.0, 1_000.0, 2_000.0, 20_000.0]);
        let mut lfe = main.clone();
        lfe.spl.fill(4_000.0);
        let config = crate::config_adapter::to_optimizer_epa(&EpaConfig::default());
        let singleton =
            level_matched_epa(&[(&main, &main, EpaChannelRole::Main)], &config).unwrap();
        let aggregate = level_matched_epa(
            &[
                (&main, &main, EpaChannelRole::Main),
                (&lfe, &lfe, EpaChannelRole::Lfe),
            ],
            &config,
        )
        .unwrap();
        assert_eq!(aggregate.pre_reference_db, singleton.pre_reference_db);
        assert_eq!(aggregate.post_reference_db, singleton.post_reference_db);
        assert_eq!(aggregate.pre.preference, singleton.pre.preference);
        assert_eq!(aggregate.post.preference, singleton.post.preference);
        assert_eq!(aggregate.preference_delta, 0.0);
    }

    #[test]
    fn level_matched_epa_refuses_missing_band_or_candidate_support() {
        let config = crate::config_adapter::to_optimizer_epa(&EpaConfig::default());
        let pre = curve(vec![20.0, 100.0, 1_000.0, 2_000.0, 20_000.0]);
        for post in [
            curve(vec![100.0, 1_000.0, 2_000.0, 20_000.0]),
            curve(vec![20.0, 100.0, 1_000.0, 2_000.0]),
        ] {
            assert!(level_matched_epa(&[(&pre, &post, EpaChannelRole::Main)], &config).is_none());
        }
        let sub = curve(vec![20.0, 100.0, 200.0]);
        assert!(level_matched_epa(&[(&sub, &sub, EpaChannelRole::Main)], &config).is_none());
    }

    #[test]
    fn alignment_interpolates_log_frequency_only_inside_shared_support() {
        let a = curve(vec![20.0, 40.0, 80.0, 160.0]);
        let b = curve(vec![30.0, 60.0, 120.0, 240.0]);
        let (grid, levels) = align_epa_curves(&[&a, &b]).unwrap();
        assert_eq!(grid, vec![30.0, 40.0, 60.0, 80.0, 120.0, 160.0]);
        for spl in levels {
            for (f, value) in grid.iter().zip(spl) {
                assert!((value - 3.0 * f.log2()).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn alignment_rejects_disjoint_or_invalid_data() {
        let a = curve(vec![20.0, 40.0]);
        assert!(align_epa_curves(&[&a, &curve(vec![80.0, 160.0])]).is_none());
        assert!(align_epa_curves(&[&a, &curve(vec![40.0, 80.0])]).is_none());
        for freq in [
            vec![],
            vec![20.0],
            vec![40.0, 20.0],
            vec![20.0, 20.0],
            vec![0.0, 40.0],
            vec![20.0, f64::NAN],
        ] {
            assert!(align_epa_curves(&[&a, &curve(freq)]).is_none());
        }
        let mut bad = a.clone();
        bad.spl.pop();
        assert!(align_epa_curves(&[&a, &bad]).is_none());
        bad = a.clone();
        bad.spl[0] = f64::NAN;
        assert!(align_epa_curves(&[&a, &bad]).is_none());
    }
}
