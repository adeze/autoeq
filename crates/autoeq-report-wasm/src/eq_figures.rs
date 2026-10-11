//! Shapes prepared equalizer responses into renderer-neutral report figures.
//!
//! This module owns EQ figure titles, labels, colors, axes, and reference
//! traces. It deliberately does not calculate filter transfer functions:
//! callers provide the combined response and each individual or pass response
//! prepared by the authoritative DSP implementation.
//!
//! # Examples
//!
//! ```
//! use autoeq_report_wasm::eq_figures::build_eq_figure;
//! use serde_json::json;
//!
//! let figure = build_eq_figure(
//!     "single",
//!     &json!({
//!         "channel_name": "L",
//!         "freq_points": [20.0, 20000.0],
//!         "combined_response": [1.0, -1.0],
//!         "filters": [],
//!     }),
//! ).unwrap();
//! assert_eq!(figure["kind"], "figure");
//! ```
//!
//! # Errors
//!
//! Returns an error when the requested builder is unsupported.

// Rust guideline compliant 2026-02-21

use serde_json::{Value, json};

const COMBINED_EQ_COLOR: &str = "rgba(100, 100, 255, 0.9)";
const COMBINED_PASSES_COLOR: &str = "rgba(50, 50, 50, 0.9)";
const REFERENCE_COLOR: &str = "rgba(150, 150, 150, 0.5)";
const FILTER_COLORS: [&str; 7] = [
    "rgba(255, 150, 150, 0.6)",
    "rgba(150, 255, 150, 0.6)",
    "rgba(150, 150, 255, 0.6)",
    "rgba(255, 255, 150, 0.6)",
    "rgba(255, 150, 255, 0.6)",
    "rgba(150, 255, 255, 0.6)",
    "rgba(200, 200, 200, 0.6)",
];

/// Builds a single-filter or labeled-multipass EQ figure from prepared arrays.
///
/// `request` carries the named arguments from the Python adapter. Its response
/// arrays are already computed by the production DSP boundary. An empty
/// combined response produces JSON `null`, matching the former figure builder.
///
/// # Examples
///
/// ```
/// use autoeq_report_wasm::eq_figures::build_eq_figure;
/// use serde_json::json;
/// let unavailable = build_eq_figure("single", &json!({"combined_response": []})).unwrap();
/// assert!(unavailable.is_null());
/// ```
///
/// # Errors
///
/// Returns an error for an unsupported builder name.
pub fn build_eq_figure(builder: &str, request: &Value) -> Result<Value, String> {
    match builder {
        "single" => Ok(single_eq_figure(request)),
        "multipass" => Ok(multipass_eq_figure(request)),
        _ => Err(format!("unknown EQ figure builder: {builder}")),
    }
}

fn single_eq_figure(request: &Value) -> Value {
    let combined = array(request, "combined_response");
    if combined.is_empty() {
        return Value::Null;
    }
    let frequency = array(request, "freq_points");
    let channel_name = string(request, "channel_name", "");
    let mut traces = vec![line_series(
        "Combined EQ",
        &frequency,
        &combined,
        COMBINED_EQ_COLOR,
        2.0,
        "solid",
    )];

    if let Some(filters) = request.get("filters").and_then(Value::as_array) {
        for (index, filter) in filters.iter().enumerate() {
            let metadata = filter.get("metadata").unwrap_or(&Value::Null);
            let response = array(filter, "response");
            traces.push(line_series(
                &filter_label(metadata),
                &frequency,
                &response,
                FILTER_COLORS[index % FILTER_COLORS.len()],
                1.0,
                "dot",
            ));
        }
    }
    traces.push(line_series(
        "0 dB",
        &reference_frequency(&frequency),
        &[json!(0.0), json!(0.0)],
        REFERENCE_COLOR,
        1.0,
        "dash",
    ));

    let y_range = single_y_range(&channel_name, &frequency, &combined);
    figure(
        format!("EQ Response: {channel_name}"),
        axis("Frequency (Hz)", "log", json!(20), json!(20000)),
        axis("Gain (dB)", "linear", json!(y_range.0), json!(y_range.1)),
        traces,
        request.get("tab").cloned().unwrap_or(Value::Null),
    )
}

fn multipass_eq_figure(request: &Value) -> Value {
    if request
        .get("passes")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Value::Null;
    }
    let frequency = array(request, "freq_points");
    let combined_frequency = request
        .get("combined_freq_points")
        .filter(|value| !value.is_null())
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| frequency.clone());
    let combined = array(request, "combined_response");
    let mut traces = Vec::new();
    if !combined.is_empty() {
        traces.push(line_series(
            "Combined (all passes)",
            &combined_frequency,
            &combined,
            COMBINED_PASSES_COLOR,
            2.5,
            "solid",
        ));
    }
    if let Some(passes) = request.get("passes").and_then(Value::as_array) {
        for pass in passes {
            let response = array(pass, "response");
            if response.is_empty() {
                continue;
            }
            traces.push(line_series(
                string(pass, "display_name", "Room EQ"),
                &frequency,
                &response,
                string(pass, "color", "rgba(100, 100, 255, 0.9)"),
                2.0,
                "solid",
            ));
        }
    }
    traces.push(line_series(
        "0 dB",
        &reference_frequency(&frequency),
        &[json!(0.0), json!(0.0)],
        REFERENCE_COLOR,
        1.0,
        "dash",
    ));
    let (y_min, y_max) = response_y_range(&combined);
    figure(
        format!(
            "EQ Response (3-Pass): {}",
            string(request, "channel_name", "")
        ),
        axis("Frequency (Hz)", "log", json!(20), json!(20000)),
        axis("Gain (dB)", "linear", json!(y_min), json!(y_max)),
        traces,
        request.get("tab").cloned().unwrap_or(Value::Null),
    )
}

fn single_y_range(channel_name: &str, frequency: &[Value], response: &[Value]) -> (f64, f64) {
    if channel_name.to_lowercase().contains("lfe") {
        let lfe_values: Vec<f64> = frequency
            .iter()
            .zip(response)
            .filter_map(|(frequency, response)| {
                let frequency = as_number(frequency)?;
                let response = as_number(response)?;
                (20.0..=200.0).contains(&frequency).then_some(response)
            })
            .collect();
        if lfe_values.is_empty() {
            return (-15.0, 15.0);
        }
        let minimum = lfe_values.iter().copied().reduce(f64::min).unwrap_or(0.0);
        let maximum = lfe_values.iter().copied().reduce(f64::max).unwrap_or(0.0);
        let limit = (((minimum.abs().max(maximum.abs()) / 5.0).ceil() * 5.0) + 5.0).max(10.0);
        return (-limit, limit);
    }
    response_y_range(response)
}

fn response_y_range(response: &[Value]) -> (f64, f64) {
    let finite: Vec<f64> = response
        .iter()
        .filter_map(as_number)
        .filter(|value| value.is_finite())
        .collect();
    let Some(minimum) = finite.iter().copied().reduce(f64::min) else {
        return (-15.0, 15.0);
    };
    let maximum = finite.iter().copied().reduce(f64::max).unwrap_or(15.0);
    let y_max = (maximum / 5.0).ceil() * 5.0;
    let y_min = ((minimum / 5.0).floor() * 5.0).max(y_max - 50.0);
    (y_min, y_max)
}

fn filter_label(metadata: &Value) -> String {
    if metadata.get("topology").and_then(Value::as_str) == Some("kautz_filter") {
        return "KAUTZ bank (unity + weighted basis)".to_owned();
    }
    let prefix = if metadata.get("topology").and_then(Value::as_str) == Some("warped_biquad") {
        "WARPED "
    } else {
        ""
    };
    let filter_type = string(metadata, "filter_type", "peak").to_uppercase();
    let frequency = metadata.get("freq").and_then(as_number).unwrap_or(0.0);
    let gain = metadata.get("db_gain").and_then(as_number).unwrap_or(0.0);
    format!("{prefix}{filter_type} {frequency:.0}Hz {gain:+.1}dB")
}

fn reference_frequency(frequency: &[Value]) -> Vec<Value> {
    vec![
        frequency.first().cloned().unwrap_or(Value::Null),
        frequency.last().cloned().unwrap_or(Value::Null),
    ]
}

fn line_series(name: &str, x: &[Value], y: &[Value], color: &str, width: f64, dash: &str) -> Value {
    json!({
        "name": name,
        "x": x.iter().map(float_value).collect::<Vec<_>>(),
        "y": y.iter().map(float_value).collect::<Vec<_>>(),
        "color": color,
        "width": width,
        "dash": dash,
        "visible": true,
        "y_axis": 0,
    })
}

fn float_value(value: &Value) -> Value {
    as_number(value).map_or(Value::Null, |number| json!(number))
}

fn as_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| {
            value
                .as_bool()
                .map(|boolean| if boolean { 1.0 } else { 0.0 })
        })
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn array(request: &Value, key: &str) -> Vec<Value> {
    request
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn string<'a>(request: &'a Value, key: &str, default: &'a str) -> &'a str {
    request.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn axis(label: &str, scale: &str, minimum: Value, maximum: Value) -> Value {
    json!({"label": label, "scale": scale, "min": minimum, "max": maximum})
}

fn figure(title: String, x: Value, y: Value, series: Vec<Value>, tab: Value) -> Value {
    json!({
        "kind": "figure",
        "figure": {
            "title": title,
            "x": x,
            "y": y,
            "y2": null,
            "series": series,
            "hlines": [],
            "vlines": [],
            "xranges": [],
            "annotations": [],
            "legend": true,
        },
        "tab": tab,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::build_eq_figure;

    #[test]
    fn pre_port_python_payloads_match() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/eq_figures_python_goldens.json"
        ))
        .expect("Python EQ figure golden fixture parses");
        for case in fixture["cases"].as_array().expect("fixture cases") {
            let result = build_eq_figure(
                case["request"]["builder"].as_str().expect("builder"),
                &case["request"],
            )
            .expect("known EQ figure builder");
            assert_json_close(
                &result,
                &case["expected"],
                case["name"].as_str().unwrap_or("case"),
            );
        }
    }

    #[test]
    fn unsupported_builder_is_an_error() {
        assert!(build_eq_figure("other", &json!({})).is_err());
    }

    fn assert_json_close(actual: &Value, expected: &Value, path: &str) {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => {
                assert_eq!(
                    actual.keys().collect::<Vec<_>>(),
                    expected.keys().collect::<Vec<_>>(),
                    "{path}"
                );
                for (key, expected_value) in expected {
                    assert_json_close(&actual[key], expected_value, &format!("{path}.{key}"));
                }
            }
            (Value::Array(actual), Value::Array(expected)) => {
                assert_eq!(actual.len(), expected.len(), "{path}");
                for (index, (actual_value, expected_value)) in
                    actual.iter().zip(expected).enumerate()
                {
                    assert_json_close(actual_value, expected_value, &format!("{path}[{index}]"));
                }
            }
            (Value::Number(actual), Value::Number(expected)) => {
                let (Some(actual), Some(expected)) = (actual.as_f64(), expected.as_f64()) else {
                    assert_eq!(actual, expected, "{path}");
                    return;
                };
                assert!(
                    (actual - expected).abs() <= 1.0e-10,
                    "{path}: {actual} != {expected}"
                );
            }
            _ => assert_eq!(actual, expected, "{path}"),
        }
    }
}
