//! Validates report grids and exposes canonical serialized filter responses.
// Rust guideline compliant 2026-02-21

use autoeq_core::serialized_dsp::serialized_filter_response as core_response;
use serde_json::Value;

// Bounds JSON parsing/allocation for report transport grids to about 1.6 MiB.
pub(crate) const MAX_RESPONSE_FREQUENCIES: usize = 100_000;

/// Evaluate a serialized filter over a finite report frequency grid.
///
/// The report boundary accepts frequencies from DC through Nyquist and returns
/// each complex response as a `[real, imaginary]` pair. The core API remains
/// scalar and preserves the wider periodic-frequency behavior used by the DSP
/// engine.
///
/// # Errors
/// Returns an error for an invalid sample rate, oversized or invalid grid,
/// invalid serialized filter, or non-finite response.
///
/// # Panics
/// This function does not panic for caller-provided JSON values.
pub fn serialized_filter_response(
    filter: &Value,
    frequencies_hz: &[f64],
    sample_rate_hz: f64,
) -> Result<Vec<[f64; 2]>, String> {
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
        return Err("EQ sample rate must be finite and positive".to_string());
    }
    if frequencies_hz.len() > MAX_RESPONSE_FREQUENCIES {
        return Err(format!(
            "EQ response grid exceeds {MAX_RESPONSE_FREQUENCIES} frequencies"
        ));
    }
    let nyquist_hz = sample_rate_hz / 2.0;
    if frequencies_hz
        .iter()
        .any(|frequency| !frequency.is_finite() || *frequency < 0.0 || *frequency > nyquist_hz)
    {
        return Err("EQ evaluation frequency must be between DC and Nyquist".to_string());
    }

    // The Python contract validates the filter even for an empty grid. A DC
    // evaluation keeps that behavior without introducing a separate validator.
    if frequencies_hz.is_empty() {
        core_response(filter, 0.0, sample_rate_hz).map_err(|error| error.to_string())?;
    }

    frequencies_hz
        .iter()
        .map(|frequency| {
            let response = core_response(filter, *frequency, sample_rate_hz)
                .map_err(|error| error.to_string())?;
            if !response.re.is_finite() || !response.im.is_finite() {
                return Err("serialized EQ response is non-finite".to_string());
            }
            Ok([response.re, response.im])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{MAX_RESPONSE_FREQUENCIES, serialized_filter_response};
    use serde_json::json;

    #[test]
    fn rejects_out_of_range_and_oversized_report_grids() {
        let filter = json!({"filter_type":"peak","freq":1000.0,"q":1.0,"db_gain":0.0});
        assert!(serialized_filter_response(&filter, &[0.0, 24_001.0], 48_000.0).is_err());
        assert!(serialized_filter_response(&filter, &[f64::NAN], 48_000.0).is_err());
        assert!(
            serialized_filter_response(&filter, &[0.0; MAX_RESPONSE_FREQUENCIES + 1], 48_000.0)
                .is_err()
        );
    }

    #[test]
    fn encodes_complex_response_pairs() {
        let filter = json!({"filter_type":"peak","freq":1000.0,"q":1.0,"db_gain":0.0});
        assert_eq!(
            serialized_filter_response(&filter, &[0.0], 48_000.0).unwrap(),
            vec![[1.0, 0.0]]
        );
    }

    #[test]
    fn validates_filter_before_returning_an_empty_grid() {
        assert!(
            serialized_filter_response(&json!({"topology":"unsupported"}), &[], 48_000.0).is_err()
        );
        assert!(
            serialized_filter_response(
                &json!({"filter_type":"peak","freq":1000.0,"q":1.0,"db_gain":0.0}),
                &[],
                48_000.0,
            )
            .unwrap()
            .is_empty()
        );
    }
}
