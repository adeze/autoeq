//! Delivered EPA acceptance protects configured-reference spectral target error.
use super::*;

// Numerical tolerance only, not a perceptual or audibility threshold.
const ROUNDING_TOLERANCE: f64 = 1e-9;

fn check(id: String, receipt: Option<&StageCheck>) -> StageCheck {
    let mut result = receipt.cloned().unwrap_or_else(|| StageCheck {
        id: String::new(),
        kind: StageCheckKind::Quality,
        passed: false,
        observed: None,
        limit: None,
        diagnostic: Some("configured frozen EPA spectral-flatness evidence unavailable".into()),
    });
    result.id = id;
    result.kind = StageCheckKind::Quality;
    result.passed &= result.limit == Some(ROUNDING_TOLERANCE)
        && result
            .observed
            .is_some_and(|delta| delta.is_finite() && delta <= ROUNDING_TOLERANCE);
    result
}

pub(super) fn assess(result: &mut RoomOptimizationResult, _config: &RoomConfig) -> Vec<StageCheck> {
    let mut checks = Vec::new();
    if let Some(quality) = result
        .metadata
        .correction_acceptance
        .as_ref()
        .and_then(|report| report.acoustic_quality.as_ref())
    {
        for (partition, score) in std::iter::once(("training", &quality.training))
            .chain(quality.held_out.as_ref().map(|score| ("held_out", score)))
        {
            let mut assessed = false;
            if let Some(comparison) = &score.configured_baseline {
                for residual in &comparison.residuals {
                    let id = format!(
                        "configured_epa_flatness_{partition}_{}_seat_{}",
                        residual.score.logical_input.as_deref().unwrap_or("unknown"),
                        residual.seat_index
                    );
                    checks.push(check(id, residual.epa_spectral_flatness.as_ref()));
                    assessed = true;
                }
                if !comparison.unavailable.is_empty() {
                    checks.push(check(
                        format!("configured_epa_flatness_{partition}_incomplete"),
                        None,
                    ));
                }
            }
            if !assessed {
                checks.push(check(
                    format!("configured_epa_flatness_{partition}_unavailable"),
                    None,
                ));
            }
        }
    }
    if checks.is_empty() {
        checks.push(check("configured_epa_flatness_unavailable".into(), None));
    }
    result
        .metadata
        .stage_outcomes
        .retain(|stage| stage.stage != "delivered_epa_non_regression");
    result.metadata.stage_outcomes.push(StageOutcome {
        stage: "delivered_epa_non_regression".into(),
        status: if checks.iter().all(|check| check.passed) { StageStatus::Applied } else { StageStatus::Degraded },
        advisories: vec![
            "criterion=configured_frozen_epa_spectral_flatness_v1".into(),
            "transfer-spectrum composite preference remains diagnostic; no measured listener-preference claim".into(),
        ],
        checks: checks.clone(),
    });
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivered_epa_requires_complete_finite_spectral_evidence() {
        let mut result = crate::test_fixtures::single_channel_room_result("L");
        let checks = assess(&mut result, &RoomConfig::default());
        assert_eq!(checks.len(), 1);
        assert!(!checks[0].passed);
        assert_eq!(checks[0].observed, None);
        let mut receipt = StageCheck {
            id: "epa_spectral_flatness".into(),
            kind: StageCheckKind::Quality,
            passed: true,
            observed: Some(-0.25),
            limit: Some(ROUNDING_TOLERANCE),
            diagnostic: None,
        };
        assert!(check("healthy".into(), Some(&receipt)).passed);
        for delta in [0.01, f64::NAN, f64::INFINITY] {
            receipt.observed = Some(delta);
            assert!(!check("invalid".into(), Some(&receipt)).passed);
        }
        receipt.observed = None;
        assert!(!check("missing".into(), Some(&receipt)).passed);
        receipt.observed = Some(-0.25);
        receipt.limit = Some(0.5);
        assert!(!check("wrong criterion".into(), Some(&receipt)).passed);
    }
}
