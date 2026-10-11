//! Bounded FIR response operations for native and WASM report transports.

// Rust guideline compliant 2026-02-21

use autoeq_core::fir_response::fir_response_at;
use serde_json::Value;

// Match native replay's tap/work limits; never truncate a captured response.
const MAX_FIR_TAPS: usize = 4_000_000;
const MAX_RESPONSE_WORK: usize = 100_000_000;

/// Evaluate a complete FIR over a validated report grid.
///
/// Empty taps are unavailable at the report boundary. The portable scalar
/// evaluator separately preserves playback's empty-tap unity convention.
///
/// # Errors
/// Returns an error for empty/non-finite taps, invalid sample rate, invalid
/// grid, excessive tap/grid/work size, or non-finite response.
pub fn fir_response(
    taps: &[f64],
    frequencies: &[f64],
    sample_rate: f64,
) -> Result<Vec<[f64; 2]>, String> {
    if taps.is_empty() || taps.len() > MAX_FIR_TAPS {
        return Err(format!(
            "FIR response requires 1 through {MAX_FIR_TAPS} taps"
        ));
    }
    if taps.iter().any(|tap| !tap.is_finite()) {
        return Err("FIR taps must be finite".into());
    }
    if !sample_rate.is_finite() || sample_rate <= 0.0 || sample_rate > u32::MAX as f64 {
        return Err(
            "FIR sample rate must be finite and positive within the supported range".into(),
        );
    }
    if frequencies.len() > crate::serialized_dsp::MAX_RESPONSE_FREQUENCIES {
        return Err("FIR response grid exceeds the supported frequency count".into());
    }
    if taps
        .len()
        .checked_mul(frequencies.len())
        .is_none_or(|work| work > MAX_RESPONSE_WORK)
    {
        return Err(format!(
            "FIR response work exceeds limit {MAX_RESPONSE_WORK}"
        ));
    }
    if frequencies.iter().any(|frequency| {
        !frequency.is_finite() || *frequency < 0.0 || *frequency > sample_rate / 2.0
    }) {
        return Err("FIR evaluation frequency must be between DC and Nyquist".into());
    }
    frequencies
        .iter()
        .map(|frequency| {
            let response = fir_response_at(taps, *frequency, sample_rate);
            if !response.re.is_finite() || !response.im.is_finite() {
                return Err("FIR response is non-finite".into());
            }
            Ok([response.re, response.im])
        })
        .collect()
}

/// Evaluate owned taps using a report request's grid and sample rate.
///
/// Native hosts resolve resource keys against their captured owner before
/// calling this operation; the shared operation performs no path or key lookup.
///
/// # Errors
/// Returns an error for missing/non-numeric rate or grid and any FIR response
/// validation error. Grid size is checked before allocating numeric values.
pub fn response_with_taps(taps: &[f64], request: &Value) -> Result<Vec<[f64; 2]>, String> {
    let grid = request["frequencies"]
        .as_array()
        .ok_or("missing FIR frequency grid")?;
    if grid.len() > crate::serialized_dsp::MAX_RESPONSE_FREQUENCIES {
        return Err("FIR response grid exceeds the supported frequency count".into());
    }
    let frequencies = grid
        .iter()
        .map(|value| value.as_f64().ok_or("FIR frequency must be numeric"))
        .collect::<Result<Vec<_>, _>>()?;
    let rate = request["sample_rate"]
        .as_f64()
        .ok_or("missing FIR sample rate")?;
    fir_response(taps, &frequencies, rate)
}

/// Evaluate a portable request containing numeric taps.
///
/// # Errors
/// Returns an error for missing/non-numeric taps or any response validation
/// error. Resource-key requests require a native owner and fail here.
pub fn response_from_request(request: &Value) -> Result<Vec<[f64; 2]>, String> {
    if request.get("resource_key").is_some() {
        return Err("FIR resource-key lookup requires a native captured owner".into());
    }
    let values = request["taps"].as_array().ok_or("missing FIR taps")?;
    if values.is_empty() || values.len() > MAX_FIR_TAPS {
        return Err(format!(
            "FIR response requires 1 through {MAX_FIR_TAPS} taps"
        ));
    }
    let taps = values
        .iter()
        .map(|value| value.as_f64().ok_or("FIR taps must be numeric"))
        .collect::<Result<Vec<_>, _>>()?;
    response_with_taps(&taps, request)
}

#[cfg(test)]
mod tests {
    use super::{fir_response, response_from_request};
    use serde_json::json;

    #[test]
    fn preserves_signed_complex_phase_and_dc_cancellation() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            let values = fir_response(&[1.0, -1.0], &[0.0, rate / 4.0, rate / 2.0], rate).unwrap();
            assert_eq!(values[0], [0.0, 0.0]);
            assert!((values[1][0] - 1.0).abs() < 1e-14);
            assert!((values[1][1] - 1.0).abs() < 1e-14);
            assert!((values[2][0] - 2.0).abs() < 1e-14);
        }
    }

    #[test]
    fn rejects_missing_evidence_invalid_grids_and_excessive_work_without_truncating() {
        assert!(fir_response(&[], &[], 48_000.0).is_err());
        assert!(fir_response(&[f64::NAN], &[], 48_000.0).is_err());
        assert!(fir_response(&[1.0], &[24_001.0], 48_000.0).is_err());
        assert!(fir_response(&[1.0], &[0.0], f64::INFINITY).is_err());
        assert!(fir_response(&vec![0.0; 1_001], &vec![0.0; 100_000], 48_000.0).is_err());
        assert!(response_from_request(&json!({"resource_key":"private.wav","taps":[1.0],"frequencies":[0.0],"sample_rate":48000})).is_err());
        assert!(
            response_from_request(&json!({"taps":[true],"frequencies":[0.0],"sample_rate":48000}))
                .is_err()
        );
    }
}
