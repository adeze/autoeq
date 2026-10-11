//! Extracts display values from serialized RoomEQ evidence.
//!
//! These helpers preserve the report's display contracts; they do not perform
//! measurement analysis or infer usable measurement bands from DSP extensions.

// Rust guideline compliant 2026-02-21
use crate::report_helpers::numeric_value;
use serde_json::{Value, json};

/// Computes the established fifty-decibel display range from finite curve levels.
pub fn compute_y_range(curves: &Value) -> (f64, f64) {
    let maximum = curves
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|curve| curve["spl"].as_array())
        .flatten()
        .filter_map(numeric_value)
        .filter(|value| value.is_finite())
        .max_by(f64::total_cmp);
    match maximum {
        Some(value) => {
            let upper = (value / 5.0).ceil() * 5.0;
            (upper - 50.0, upper)
        }
        None => (-20.0, 30.0),
    }
}

/// Computes a display-band mean, returning zero when no samples are present.
///
/// Frequency and level values come from the same serialized curve. The helper
/// retains legacy truncation for unequal axes; pairing different curves requires
/// explicit alignment before using this helper.
pub fn average_spl(curve: &Value, min_hz: f64, max_hz: f64) -> f64 {
    let (Some(frequencies), Some(levels)) = (curve["freq"].as_array(), curve["spl"].as_array())
    else {
        return 0.0;
    };
    let values: Vec<_> = frequencies
        .iter()
        .zip(levels)
        .filter_map(|(frequency, level)| {
            let frequency = numeric_value(frequency)?;
            let level = numeric_value(level)?;
            (frequency >= min_hz && frequency <= max_hz && level.is_finite()).then_some(level)
        })
        .collect();
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn populated(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
    }
}

/// Reports whether serialized channel or driver chains contain equalization.
pub fn channel_has_eq(channel: &Value) -> bool {
    if populated(&channel["eq_response"]["spl"]) {
        return true;
    }
    let chains =
        std::iter::once(channel).chain(channel["drivers"].as_array().into_iter().flatten());
    chains
        .flat_map(|chain| chain["plugins"].as_array().into_iter().flatten())
        .any(|plugin| plugin["plugin_type"] == "eq" && populated(&plugin["parameters"]["filters"]))
}

/// Clips matching curve arrays to a recorded measurement band when two points remain.
///
/// Missing or invalid bands and insufficient retained points preserve the original
/// curve, matching legacy display behavior. Unknown scalar fields remain intact.
pub fn clip_curve_to_measured_band(curve: &Value, driver: &Value) -> Value {
    let Some(band) = driver["measured_band_hz"].as_array() else {
        return curve.clone();
    };
    if band.len() != 2 {
        return curve.clone();
    }
    let (Some(low), Some(high)) = (numeric_value(&band[0]), numeric_value(&band[1])) else {
        return curve.clone();
    };
    if low >= high || !low.is_finite() || !high.is_finite() {
        return curve.clone();
    }
    let Some(frequencies) = curve["freq"].as_array() else {
        return curve.clone();
    };
    if frequencies.len() < 2 {
        return curve.clone();
    }
    // Match the exporter/viewer tolerance at exact measurement-band edges.
    let keep: Vec<_> = frequencies
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            numeric_value(value)
                .filter(|value| *value >= low * (1.0 - 1e-9) && *value <= high * (1.0 + 1e-9))
                .map(|_| index)
        })
        .collect();
    if keep.len() < 2 || keep.len() == frequencies.len() {
        return curve.clone();
    }
    let Some(fields) = curve.as_object() else {
        return curve.clone();
    };
    Value::Object(
        fields
            .iter()
            .map(|(key, value)| {
                let clipped = match value.as_array() {
                    Some(values) if values.len() == frequencies.len() => json!(
                        keep.iter()
                            .map(|index| values[*index].clone())
                            .collect::<Vec<_>>()
                    ),
                    _ => value.clone(),
                };
                (key.clone(), clipped)
            })
            .collect(),
    )
}

/// Extracts sorted unique configured crossover and main band-split frequencies.
pub fn crossover_frequencies(channel: &Value) -> Vec<f64> {
    let mut frequencies = Vec::new();
    let mut append = |chain: &Value, allow_band_split: bool| {
        for plugin in chain["plugins"].as_array().into_iter().flatten() {
            if plugin["plugin_type"] == "crossover"
                || (allow_band_split && plugin["plugin_type"] == "band_split")
            {
                let value = &plugin["parameters"]["frequency"];
                let frequency = numeric_value(value)
                    .or_else(|| value.as_str().and_then(|value| value.parse().ok()));
                if let Some(frequency) =
                    frequency.filter(|value| value.is_finite() && *value != 0.0)
                {
                    frequencies.push(frequency);
                }
            }
        }
    };
    append(channel, true);
    for driver in channel["drivers"].as_array().into_iter().flatten() {
        append(driver, false);
    }
    frequencies.sort_by(f64::total_cmp);
    frequencies.dedup();
    frequencies
}

/// Dispatches a report data helper without filesystem or renderer dependencies.
///
/// # Errors
/// Returns an error if the request names an unsupported helper.
pub fn helper_response(request: &Value) -> Result<Value, String> {
    let bound = |name: &str| request[name].as_f64().unwrap_or(f64::NAN);
    match request["helper"].as_str().unwrap_or_default() {
        "compute_y_range" => Ok(json!(compute_y_range(&request["curves"]))),
        "average_spl" => Ok(json!(average_spl(
            &request["curve"],
            bound("min_hz"),
            bound("max_hz")
        ))),
        "channel_has_eq" => Ok(json!(channel_has_eq(&request["channel"]))),
        "clip_curve" => Ok(clip_curve_to_measured_band(
            &request["curve"],
            &request["driver"],
        )),
        "crossover_frequencies" => Ok(json!(crossover_frequencies(&request["channel"]))),
        other => Err(format!("unsupported report data helper `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn numeric_form(value: &Value) -> Value {
        match value {
            Value::Number(number) => json!(number.as_f64().unwrap()),
            Value::Array(values) => json!(values.iter().map(numeric_form).collect::<Vec<_>>()),
            Value::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), numeric_form(value)))
                    .collect(),
            ),
            _ => value.clone(),
        }
    }
    #[test]
    fn captured_python_display_extraction_matches() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/fixtures/report_data_python.json"))
                .unwrap();
        for case in cases.as_array().unwrap() {
            assert_eq!(
                numeric_form(&helper_response(&case["request"]).unwrap()),
                numeric_form(&case["expected"])
            );
        }
    }
}
