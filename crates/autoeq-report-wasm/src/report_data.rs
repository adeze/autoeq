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

fn plottable_drivers(channel: &Value) -> Vec<(usize, &Value)> {
    channel["drivers"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, driver)| {
            let initial = &driver["initial_curve"];
            populated(&initial["freq"]) && populated(&initial["spl"])
        })
        .collect()
}

fn configured_driver_names(data: &Value, channel_name: &str, driver_count: usize) -> Vec<String> {
    if driver_count == 0 {
        return Vec::new();
    }
    let effective = &data["metadata"]["effective_config"];
    let speaker_key = effective["system"]["speakers"][channel_name].as_str();
    let subwoofers = speaker_key
        .and_then(|key| effective["speakers"][key]["subwoofers"].as_array())
        .into_iter()
        .flatten();
    let names: Vec<_> = subwoofers
        .filter(|entry| entry.is_object() && populated(&entry["name"]))
        .map(|entry| crate::report_helpers::python_string(&entry["name"]))
        .collect();
    if names.len() == driver_count {
        names
    } else {
        Vec::new()
    }
}

/// Expands sorted channels into their report tab entries.
///
/// Driver numbers are positions in the filtered plottable-driver list. This
/// keeps the existing Python consumer contract when source driver arrays have
/// missing or unplottable entries; source `index` values only supply fallback
/// labels and do not replace those display positions.
pub fn display_channel_entries(data: &Value) -> Value {
    let Some(channels) = data["channels"].as_object() else {
        return json!([]);
    };
    let mut names: Vec<_> = channels.keys().collect();
    names.sort_by(|left, right| {
        crate::report_helpers::channel_sort_key(left)
            .cmp(&crate::report_helpers::channel_sort_key(right))
    });

    let mut entries = Vec::new();
    for channel_name in names {
        let channel = &channels[channel_name];
        let drivers = plottable_drivers(channel);
        if drivers.is_empty() {
            entries.push(json!({
                "label": channel_name,
                "channel": channel_name,
                "driver": null,
            }));
            continue;
        }

        let configured_names = configured_driver_names(data, channel_name, drivers.len());
        for (display_index, (_, driver)) in drivers.into_iter().enumerate() {
            let label = configured_names
                .get(display_index)
                .cloned()
                .unwrap_or_else(|| {
                    let fallback = if populated(&driver["name"]) {
                        &driver["name"]
                    } else if let Some(source_index) =
                        driver.as_object().and_then(|object| object.get("index"))
                    {
                        source_index
                    } else {
                        return display_index.to_string();
                    };
                    crate::report_helpers::python_string(fallback)
                });
            entries.push(json!({
                "label": label,
                "channel": channel_name,
                "driver": display_index,
            }));
        }
    }
    json!(entries)
}

/// Expands channel tabs while preserving each driver's raw graph identity.
///
/// `driver` remains the filtered display position used by existing report
/// consumers. The `source_*` fields identify the original driver entry.
pub fn driver_display_entries(data: &Value) -> Value {
    let Some(channels) = data["channels"].as_object() else {
        return json!([]);
    };
    let mut names: Vec<_> = channels.keys().collect();
    names.sort_by(|left, right| {
        crate::report_helpers::channel_sort_key(left)
            .cmp(&crate::report_helpers::channel_sort_key(right))
    });
    let mut entries = Vec::new();
    for channel_name in names {
        let channel = &channels[channel_name];
        let drivers = plottable_drivers(channel);
        if drivers.is_empty() {
            entries.push(json!({
                "label": channel_name,
                "channel": channel_name,
                "driver": null,
                "source_driver_position": null,
                "source_driver_index": null,
                "source_driver_name": null,
            }));
            continue;
        }
        let configured_names = configured_driver_names(data, channel_name, drivers.len());
        for (display_index, (source_position, driver)) in drivers.into_iter().enumerate() {
            let source_name = populated(&driver["name"]).then(|| driver["name"].clone());
            let source_index = driver.get("index").cloned();
            let label = configured_names
                .get(display_index)
                .cloned()
                .unwrap_or_else(|| {
                    source_name
                        .as_ref()
                        .or(source_index.as_ref())
                        .map(crate::report_helpers::python_string)
                        .unwrap_or_else(|| display_index.to_string())
                });
            entries.push(json!({
                "label": label,
                "channel": channel_name,
                "driver": display_index,
                "source_driver_position": source_position,
                "source_driver_index": source_index,
                "source_driver_name": source_name,
            }));
        }
    }
    json!(entries)
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
        "channel_display_entries" => Ok(display_channel_entries(&request["data"])),
        "driver_display_entries" => Ok(driver_display_entries(&request["data"])),
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

    #[test]
    fn channel_display_entries_match_captured_python_cases() {
        let cases: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/channel_display_entries_python.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let request = json!({
                "helper": "channel_display_entries",
                "data": case["data"],
            });
            assert_eq!(
                helper_response(&request).unwrap(),
                case["expected"],
                "case {}",
                case["name"]
            );
        }
    }

    #[test]
    fn driver_display_entries_retain_filtered_and_source_identity() {
        let data = json!({
            "channels": {
                "LFE": {"drivers": [
                    {"index": 12, "name": "missing", "initial_curve": {"freq": [], "spl": []}},
                    {"index": 28, "name": "Sub <A>", "initial_curve": {"freq": [20], "spl": [70]}},
                    {"name": "", "initial_curve": {"freq": [30], "spl": [71]}}
                ]}
            }
        });
        let entries = driver_display_entries(&data);
        assert_eq!(entries[0]["driver"], 0);
        assert_eq!(entries[0]["source_driver_position"], 1);
        assert_eq!(entries[0]["source_driver_index"], 28);
        assert_eq!(entries[0]["source_driver_name"], "Sub <A>");
        assert_eq!(entries[0]["label"], "Sub <A>");
        assert_eq!(entries[1]["driver"], 1);
        assert_eq!(entries[1]["source_driver_position"], 2);
        assert_eq!(entries[1]["source_driver_index"], Value::Null);
        assert_eq!(entries[1]["label"], "1");
    }
}
