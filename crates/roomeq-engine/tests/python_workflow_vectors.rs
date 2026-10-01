//! Bounded interchange checks, not FIR synthesis or candidate parity.
use math_audio_dsp::response::fir_complex_response;
use math_audio_iir_fir::Fir;
use ndarray::Array1;
use roomeq_engine::eq::{EqResources, PreparedEqTarget};
use roomeq_engine::fir::prepared_fir_target_curve;
use roomeq_model::{Curve, OptimizerConfig, TargetResponseConfig, TargetShape};
use serde::Deserialize;

#[derive(Deserialize)]
struct Vectors {
    schema_version: u32,
    evidence_status: String,
    frequencies_hz: Vec<f64>,
    target_db: Vec<f64>,
    default_python_target_db: Vec<f64>,
    normalization_offset_db: f64,
    normalized_target_db: Vec<f64>,
    fir_cases: Vec<FirCase>,
}

#[derive(Deserialize)]
struct FirCase {
    tap_count: usize,
    sample_rate_hz: f64,
    taps: Vec<f64>,
    frequencies_hz: Vec<f64>,
    response_re: Vec<f64>,
    response_im: Vec<f64>,
    response_db: Vec<f64>,
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(actual.is_finite() && expected.is_finite());
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

#[test]
fn python_target_normalization_and_serialized_fir_vectors() {
    let vectors: Vectors = serde_json::from_str(include_str!(
        "../../../tests/data/python_workflow_vectors.json"
    ))
    .unwrap();
    assert_eq!(vectors.schema_version, 1);
    assert_eq!(vectors.evidence_status, "synthetic_comparison_only");
    let frequencies = Array1::from(vectors.frequencies_hz.clone());
    let target_config = TargetResponseConfig {
        shape: TargetShape::Custom,
        slope_db_per_octave: -0.4,
        reference_freq: 1000.0,
        ..TargetResponseConfig::default()
    };
    let target =
        roomeq_model::target_tilt::build_complete_target_curve(&frequencies, &target_config);
    assert_eq!(target.spl.len(), vectors.target_db.len());
    for (&actual, &expected) in target.spl.iter().zip(&vectors.target_db) {
        close(actual, expected, 2e-7); // Python target evaluation uses float32.
    }
    assert!(
        vectors
            .default_python_target_db
            .iter()
            .zip(&vectors.target_db)
            .any(|(default, shared)| (default - shared).abs() > 1.0)
    );
    let measurement = Curve {
        freq: frequencies.clone(),
        spl: Array1::from_elem(frequencies.len(), 80.0),
        ..Curve::default()
    };
    let sampled_target = Curve {
        freq: frequencies,
        spl: Array1::from(vectors.target_db.clone()),
        ..Curve::default()
    };
    let config = OptimizerConfig {
        min_freq: 120.0,
        max_freq: 15360.0,
        ..OptimizerConfig::default()
    };
    let resources = EqResources {
        target: Some(PreparedEqTarget::Curve(Box::new(sampled_target))),
        impulse_response: None,
    };
    let normalized = prepared_fir_target_curve(&measurement, &config, &resources);
    assert_eq!(normalized.spl.len(), vectors.normalized_target_db.len());
    for ((&actual, &expected), &raw) in normalized
        .spl
        .iter()
        .zip(&vectors.normalized_target_db)
        .zip(&vectors.target_db)
    {
        close(actual, expected, 2e-5); // Rust normalization accumulates float32 segment means.
        close(actual - raw, vectors.normalization_offset_db, 2e-5);
    }
    assert_eq!(vectors.fir_cases.len(), 3);
    for case in vectors.fir_cases {
        assert_eq!(case.taps.len(), case.tap_count);
        assert!(matches!(case.tap_count, 128 | 512));
        let filter = Fir::new_custom(case.taps.clone(), case.sample_rate_hz);
        let db = filter.np_log_result(&Array1::from(case.frequencies_hz.clone()));
        assert_eq!(db.len(), case.response_db.len());
        assert_eq!(db.len(), case.response_re.len());
        assert_eq!(db.len(), case.response_im.len());
        for (i, &frequency) in case.frequencies_hz.iter().enumerate() {
            let response = fir_complex_response(&case.taps, frequency, case.sample_rate_hz);
            close(response.re, case.response_re[i], 1e-11);
            close(response.im, case.response_im[i], 1e-11);
            close(db[i], case.response_db[i], 1e-9);
        }
    }
}
