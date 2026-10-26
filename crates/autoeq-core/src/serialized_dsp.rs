//! Evaluate serialized biquad and Kautz filter transfer functions.
// Rust guideline compliant 2026-02-21

use crate::{
    AutoeqError, Result,
    iir::{Biquad, BiquadFilterType, KautzFilter, KautzSection, bark_lambda, warp_frequency},
};
use num_complex::Complex64;
use serde_json::Value;

/// Evaluate one serialized filter using the RoomEQ playback transfer contract.
///
/// `filter` accepts the canonical serialized biquad, warped biquad, and Kautz
/// shapes emitted by RoomEQ. The evaluation frequency may exceed Nyquist, as
/// the engine's scalar response API permits periodic frequency evaluation.
/// Filter design frequencies must remain strictly below Nyquist.
///
/// # Errors
/// Returns an error when the sample rate, evaluation frequency, filter type,
/// topology, or required serialized filter values are invalid.
pub fn serialized_filter_response(
    filter: &Value,
    frequency_hz: f64,
    sample_rate_hz: f64,
) -> Result<Complex64> {
    validate_sample_rate(sample_rate_hz)?;
    if !frequency_hz.is_finite() || frequency_hz < 0.0 {
        return Err(invalid(format!(
            "invalid DSP realization frequency {frequency_hz}"
        )));
    }

    let topology = optional_string(filter, "topology", "serialized RoomEQ filter")?;
    match topology {
        Some("kautz_filter") => return kautz_response(filter, frequency_hz, sample_rate_hz),
        None | Some("biquad" | "warped_biquad") => {}
        Some(other) => {
            return Err(invalid(format!(
                "unsupported serialized RoomEQ filter topology '{other}'"
            )));
        }
    }

    let filter_type = filter
        .get("filter_type")
        .and_then(Value::as_str)
        .and_then(parse_biquad_filter_type)
        .ok_or_else(|| {
            invalid(format!(
                "unsupported serialized RoomEQ biquad filter {filter}"
            ))
        })?;
    let center_hz = required_frequency(filter, "freq", "serialized RoomEQ biquad", sample_rate_hz)?;
    let q = required_positive_number(filter, "q", "serialized RoomEQ biquad")?;
    let gain_db = required_finite_number(filter, "db_gain", "serialized RoomEQ biquad")?;

    let (design_hz, evaluation_hz) = if topology == Some("warped_biquad") {
        let lambda = optional_finite_number(
            filter,
            "lambda",
            bark_lambda(sample_rate_hz),
            "serialized warped biquad",
        )?;
        if !(-1.0..1.0).contains(&lambda) {
            return Err(invalid(format!(
                "serialized warped biquad lambda must be between -1 and 1, got {lambda}"
            )));
        }
        (
            warp_frequency(center_hz, sample_rate_hz, lambda),
            warp_frequency(frequency_hz, sample_rate_hz, lambda),
        )
    } else {
        (center_hz, frequency_hz)
    };

    // Biquad::complex_response uses the same normalized coefficients and
    // arithmetic order as the former engine math-dsp helper.
    Ok(
        Biquad::new(filter_type, design_hz, sample_rate_hz, q, gain_db)
            .complex_response(evaluation_hz),
    )
}

fn kautz_response(filter: &Value, frequency_hz: f64, sample_rate_hz: f64) -> Result<Complex64> {
    // Keep playback's canonical field, legacy alias, and single-section
    // fallback. A malformed explicit value is never treated as absent.
    let values = match (filter.get("kautz_sections"), filter.get("sections")) {
        (Some(_), Some(_)) => {
            return Err(malformed(
                "Kautz filter",
                "duplicate section fields",
                filter,
            ));
        }
        (Some(value), None) | (None, Some(value)) => Some(
            value
                .as_array()
                .ok_or_else(|| malformed("Kautz filter", "kautz_sections", filter))?,
        ),
        (None, None) => None,
    };
    let sections = if let Some(values) = values.filter(|values| !values.is_empty()) {
        values
            .iter()
            .map(|section| {
                let fields: Vec<_> = ["pole_freq", "freq", "frequency", "pole_freq_hz"]
                    .into_iter()
                    .filter(|key| section.get(*key).is_some())
                    .collect();
                if fields.len() != 1 {
                    return Err(malformed("Kautz section", "unique pole frequency", section));
                }
                let pole_hz = required_frequency(
                    section,
                    fields[0],
                    "serialized Kautz section",
                    sample_rate_hz,
                )?;
                let q = required_positive_number(section, "q", "serialized Kautz section")?;
                let gain =
                    optional_finite_number(section, "gain", 0.0, "serialized Kautz section")?;
                Ok(KautzSection::new(pole_hz, q, gain, sample_rate_hz))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![KautzSection::new(
            required_frequency(filter, "freq", "serialized Kautz fallback", sample_rate_hz)?,
            required_positive_number(filter, "q", "serialized Kautz fallback")?,
            optional_finite_number(filter, "db_gain", 0.0, "serialized Kautz fallback")?,
            sample_rate_hz,
        )]
    };

    // Playback adds the correction bank to unity; it is neither a cascade nor
    // a magnitude-only sum. Section gains are linear weights, including the
    // legacy db_gain fallback field.
    Ok(Complex64::new(1.0, 0.0)
        + KautzFilter {
            sections,
            srate: sample_rate_hz,
        }
        .complex_response(frequency_hz))
}

fn validate_sample_rate(sample_rate_hz: f64) -> Result<()> {
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 || sample_rate_hz > u32::MAX as f64 {
        return Err(invalid(format!(
            "invalid DSP realization sample rate {sample_rate_hz}"
        )));
    }
    Ok(())
}

fn invalid(message: String) -> AutoeqError {
    AutoeqError::InvalidConfiguration { message }
}

fn malformed(owner: &str, field: &str, value: &Value) -> AutoeqError {
    invalid(format!("malformed {owner} field '{field}': {value}"))
}

fn required_finite_number(object: &Value, field: &str, owner: &str) -> Result<f64> {
    object
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| malformed(owner, field, object.get(field).unwrap_or(&Value::Null)))
}

fn optional_finite_number(object: &Value, field: &str, default: f64, owner: &str) -> Result<f64> {
    match object.get(field) {
        None => Ok(default),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| malformed(owner, field, value)),
    }
}

fn required_positive_number(object: &Value, field: &str, owner: &str) -> Result<f64> {
    let value = required_finite_number(object, field, owner)?;
    if value <= 0.0 {
        return Err(malformed(owner, field, object.get(field).unwrap()));
    }
    Ok(value)
}

fn required_frequency(
    object: &Value,
    field: &str,
    owner: &str,
    sample_rate_hz: f64,
) -> Result<f64> {
    let value = required_positive_number(object, field, owner)?;
    if value >= sample_rate_hz / 2.0 {
        return Err(invalid(format!(
            "malformed {owner} field '{field}': {value} must be below Nyquist {}",
            sample_rate_hz / 2.0
        )));
    }
    Ok(value)
}

fn optional_string<'a>(object: &'a Value, field: &str, owner: &str) -> Result<Option<&'a str>> {
    match object.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(Some)
            .ok_or_else(|| malformed(owner, field, value)),
    }
}

fn parse_biquad_filter_type(value: &str) -> Option<BiquadFilterType> {
    match value {
        "lowpass" => Some(BiquadFilterType::Lowpass),
        "highpass" => Some(BiquadFilterType::Highpass),
        "highpassvariableq" => Some(BiquadFilterType::HighpassVariableQ),
        "bandpass" => Some(BiquadFilterType::Bandpass),
        "peak" => Some(BiquadFilterType::Peak),
        "notch" => Some(BiquadFilterType::Notch),
        "lowshelf" => Some(BiquadFilterType::Lowshelf),
        "highshelf" => Some(BiquadFilterType::Highshelf),
        "allpass" => Some(BiquadFilterType::AllPass),
        "lowshelforf" => Some(BiquadFilterType::LowshelfOrf),
        "highshelforf" => Some(BiquadFilterType::HighshelfOrf),
        "peakmatched" => Some(BiquadFilterType::PeakMatched),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::serialized_filter_response;
    use serde_json::Value;

    const GOLDENS: &str =
        include_str!("../../../scripts/tests/fixtures/serialized_dsp_filter_response_goldens.json");

    #[test]
    fn matches_pre_port_engine_and_python_goldens() {
        let fixture: Value =
            serde_json::from_str(GOLDENS).expect("valid serialized DSP golden fixture");
        for case in fixture["cases"].as_array().expect("golden cases") {
            let filter = &case["filter"];
            let sample_rate = case["sample_rate"].as_f64().expect("sample rate");
            let frequencies = case["frequencies"].as_array().expect("frequency grid");
            if case["unavailable"] == true {
                assert!(
                    frequencies.iter().all(|frequency| {
                        serialized_filter_response(filter, frequency.as_f64().unwrap(), sample_rate)
                            .is_err()
                    }),
                    "{} must remain unavailable",
                    case["name"]
                );
                continue;
            }

            let result: Vec<_> = frequencies
                .iter()
                .map(|frequency| {
                    serialized_filter_response(filter, frequency.as_f64().unwrap(), sample_rate)
                        .unwrap()
                })
                .collect();
            for (index, value) in result.iter().enumerate() {
                for expected_key in ["python", "engine"] {
                    let expected = &case[expected_key][index];
                    let re = expected[0].as_f64().unwrap();
                    let im = expected[1].as_f64().unwrap();
                    assert!(
                        (value.re - re).abs() <= 1e-12,
                        "{} {expected_key} real at {index}: {} != {re}",
                        case["name"],
                        value.re
                    );
                    assert!(
                        (value.im - im).abs() <= 1e-12,
                        "{} {expected_key} imaginary at {index}: {} != {im}",
                        case["name"],
                        value.im
                    );
                }
            }
        }
    }

    #[test]
    fn scalar_engine_frequency_contract_allows_nyquist_and_above() {
        let filter = serde_json::json!({"filter_type":"peak","freq":1000.0,"q":1.0,"db_gain":3.0});
        assert!(serialized_filter_response(&filter, 24_000.0, 48_000.0).is_ok());
        assert!(serialized_filter_response(&filter, 48_000.0, 48_000.0).is_ok());
        assert!(serialized_filter_response(&filter, -1.0, 48_000.0).is_err());
    }

    #[test]
    fn rejects_invalid_filters_without_defaulting_explicit_bad_values() {
        for filter in [
            serde_json::json!({"filter_type":"peak","freq":1000.0,"q":null,"db_gain":0.0}),
            serde_json::json!({"topology":"warped_biquad","filter_type":"peak","freq":1000.0,"q":1.0,"db_gain":0.0,"lambda":1.0}),
            serde_json::json!({"topology":"kautz_filter","sections":[],"kautz_sections":[]}),
        ] {
            assert!(
                serialized_filter_response(&filter, 500.0, 48_000.0).is_err(),
                "{filter}"
            );
        }
    }
}
