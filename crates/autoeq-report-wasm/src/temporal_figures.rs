//! Builds temporal and acoustic report figures from validated JSON evidence.
//!
//! The builders preserve the report payload contract used by the existing
//! renderer. They do not compute measurements: supplied impulse responses,
//! arrival rows, band-energy curves, T60 estimates, and pair sums remain the
//! evidence source. Missing or rejected evidence produces JSON `null` where
//! the Python builders previously returned `None`; gap samples remain JSON
//! `null` in line series.
//!
//! # Examples
//! ```
//! use autoeq_report_wasm::temporal_figures::build_temporal_figure;
//! use serde_json::json;
//!
//! let figure = build_temporal_figure(
//!     "create_tof_figure",
//!     &json!({"tof_rows": [{"name": "L", "before_ms": 0.25}], "tab": "timing"}),
//! ).expect("known figure builder");
//! assert_eq!(figure["kind"], "bar");
//! ```

// Rust guideline compliant 2026-02-21

use serde_json::{Value, json};

const IR_FLOOR_DB: f64 = -80.0;
const RELATIVE_SUM_DIFFERENCE_CEILING_DB: f64 = 40.0;

/// Builds one temporal figure payload or helper result by builder name.
///
/// `request` contains the original named arguments used by the matching
/// Python `scripts.src.figures` function. Figure builders return the same
/// render-only JSON object, and unavailable figures return JSON `null`.
/// `to_db` and `finite_or_gap` are exposed for callers that used those local
/// Python helpers directly.
///
/// # Examples
/// ```
/// use autoeq_report_wasm::temporal_figures::build_temporal_figure;
/// use serde_json::json;
///
/// let figure = build_temporal_figure(
///     "create_ir_figure",
///     &json!({"channel_name": "L", "pre_ir": null, "post_ir": null}),
/// ).unwrap();
/// assert!(figure.is_null());
/// ```
///
/// # Errors
/// Returns an error when `builder` is not one of the supported names.
pub fn build_temporal_figure(builder: &str, request: &Value) -> Result<Value, String> {
    let result = match builder {
        "create_ir_figure" => create_ir_figure(request),
        "create_tof_figure" => create_tof_figure(request),
        "create_early_late_figure" => create_early_late_figure(request),
        "create_t60_octaves_figure" => create_t60_octaves_figure(request),
        "create_symmetric_pair_figure" => create_symmetric_pair_figure(request),
        "to_db" => to_db(request),
        "finite_or_gap" => finite_or_gap(request),
        _ => return Err(format!("unknown temporal figure builder: {builder}")),
    };
    Ok(result)
}

fn create_ir_figure(request: &Value) -> Value {
    let pre_ir = request.get("pre_ir").unwrap_or(&Value::Null);
    let post_ir = request.get("post_ir").unwrap_or(&Value::Null);
    if !is_truthy(pre_ir) && !is_truthy(post_ir) {
        return Value::Null;
    }
    let channel_name = string_arg(request, "channel_name", "");
    let mut traces = Vec::new();
    if is_truthy(pre_ir) {
        traces.push(series(
            "Before EQ",
            pre_ir.get("time_ms").unwrap_or(&Value::Null),
            pre_ir.get("amplitude").unwrap_or(&Value::Null),
            Some("rgba(255, 100, 100, 0.8)"),
            1.0,
            "solid",
            0,
        ));
    }
    if is_truthy(post_ir) {
        traces.push(series(
            "After EQ",
            post_ir.get("time_ms").unwrap_or(&Value::Null),
            post_ir.get("amplitude").unwrap_or(&Value::Null),
            Some("rgba(100, 200, 100, 0.9)"),
            1.0,
            "solid",
            0,
        ));
    }
    let display_ms = request
        .get("display_ms")
        .cloned()
        .unwrap_or_else(|| json!(100.0));
    figure(
        format!("Impulse Response: {channel_name}"),
        axis("Time (ms)", "linear", json!(0), display_ms),
        axis("Amplitude (normalized)", "linear", json!(-1.1), json!(1.1)),
        traces,
        vec![hline(
            0.0,
            "rgba(150, 150, 150, 0.4)",
            "dash",
            1.0,
            Value::Null,
        )],
        None,
        request.get("tab").cloned().unwrap_or(Value::Null),
    )
}

fn create_tof_figure(request: &Value) -> Value {
    let after = request
        .get("after")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (key, title, color) = if after {
        (
            "after_ms",
            "Calculated arrival after DSP",
            "rgba(100, 200, 100, 0.8)",
        )
    } else {
        (
            "before_ms",
            "Measured arrival before DSP",
            "rgba(255, 100, 100, 0.8)",
        )
    };
    let mut categories = Vec::new();
    let mut values = Vec::new();
    if let Some(rows) = request.get("tof_rows").and_then(Value::as_array) {
        for row in rows {
            let Some(value) = row.get(key).and_then(non_boolean_number) else {
                continue;
            };
            if !value.is_finite() {
                continue;
            }
            categories.push(row.get("name").cloned().unwrap_or(Value::Null));
            values.push(json!(value));
        }
    }
    if categories.is_empty() {
        return Value::Null;
    }
    json!({
        "kind": "bar",
        "chart": {
            "title": title,
            "categories": categories,
            "groups": [{"name": title, "values": values, "color": color}],
            "ylabel": "Arrival (ms)",
            "ymin": null,
            "ymax": null,
            "legend": false,
            "hlines": [],
        },
        "tab": request.get("tab").cloned().unwrap_or(Value::Null),
    })
}

fn finite_or_gap(request: &Value) -> Value {
    let values = request
        .get("values")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    non_boolean_number(value)
                        .filter(|value| value.is_finite())
                        .map_or(Value::Null, |value| json!(value))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(values)
}

fn create_symmetric_pair_figure(request: &Value) -> Value {
    let label = string_arg(request, "label", "");
    let frequency = request.get("freq").and_then(Value::as_array);
    let absolute = request.get("abs_spl").and_then(Value::as_array);
    let (Some(frequency), Some(absolute)) = (frequency, absolute) else {
        return Value::Null;
    };
    let absolute_clean = finite_or_gap(&json!({"values": absolute}));
    let absolute_clean = absolute_clean.as_array().expect("helper returns an array");
    let complex = request.get("complex_spl").filter(|value| !value.is_null());
    let has_complex = complex.and_then(Value::as_array).is_some_and(|values| {
        !frequency.is_empty()
            && values.len() == frequency.len()
            && absolute.len() == frequency.len()
    });

    let mut traces = vec![series(
        &format!("{label} absolute sum"),
        &Value::Array(frequency.clone()),
        &Value::Array(absolute_clean.clone()),
        Some("rgba(74, 144, 217, 0.9)"),
        2.0,
        "solid",
        0,
    )];
    let mut y2 = Value::Null;
    let mut sum_traces = vec![absolute_clean.to_vec()];
    if has_complex {
        let complex = complex
            .and_then(Value::as_array)
            .expect("complex array was validated");
        let complex_clean = finite_or_gap(&json!({"values": complex}));
        let complex_clean = complex_clean.as_array().expect("helper returns an array");
        let difference = absolute_clean
            .iter()
            .zip(complex_clean)
            .map(|(absolute_value, complex_value)| {
                match (absolute_value.as_f64(), complex_value.as_f64()) {
                    (Some(absolute_value), Some(complex_value)) => {
                        json!(
                            (absolute_value - complex_value)
                                .min(RELATIVE_SUM_DIFFERENCE_CEILING_DB)
                        )
                    }
                    _ => Value::Null,
                }
            })
            .collect::<Vec<_>>();
        traces.push(series(
            &format!("{label} complex sum"),
            &Value::Array(frequency.clone()),
            &Value::Array(complex_clean.clone()),
            Some("rgba(46, 160, 67, 0.9)"),
            2.0,
            "solid",
            0,
        ));
        traces.push(series(
            &format!("{label} sum difference"),
            &Value::Array(frequency.clone()),
            &Value::Array(difference),
            Some("rgba(214, 69, 65, 0.9)"),
            2.0,
            "dash",
            1,
        ));
        y2 = axis("Sum difference (dB)", "linear", Value::Null, json!(40.0));
        sum_traces.push(complex_clean.to_vec());
    }
    let maximum = sum_traces
        .iter()
        .flat_map(|trace| trace.iter().filter_map(Value::as_f64))
        .filter(|value| value.is_finite())
        .reduce(f64::max);
    let ymax = maximum.map_or(0.0, |maximum| ((maximum + 3.0) / 5.0).ceil() * 5.0);
    let subtitle = if has_complex {
        "absolute and complex sums with their difference"
    } else {
        "absolute sum only; complex sum needs measured phase"
    };
    figure(
        format!("Symmetric pair: {label} ({subtitle})"),
        axis("Frequency (Hz)", "log", json!(20), json!(20000)),
        axis("SPL (dB)", "linear", json!(ymax - 50.0), json!(ymax)),
        traces,
        Vec::new(),
        Some(y2),
        request.get("tab").cloned().unwrap_or(Value::Null),
    )
}

fn create_early_late_figure(request: &Value) -> Value {
    let Some(report) = request.get("report").filter(|value| value.is_object()) else {
        return Value::Null;
    };
    if report.get("method").and_then(Value::as_str) != Some("incoherent_band_energy")
        || report.get("reference").and_then(Value::as_str) != Some("full_peak_band")
        || report.get("smoothing").and_then(Value::as_str) != Some("third_octave")
        || report.get("split_ms").and_then(Value::as_f64) != Some(20.0)
    {
        return Value::Null;
    }
    let curves =
        ["full", "early", "late"].map(|key| report.get(key).filter(|curve| curve.is_object()));
    let [Some(full), Some(early), Some(late)] = curves else {
        return Value::Null;
    };
    let frequency = full.get("freq").and_then(Value::as_array);
    let (Some(frequency), Some(full_spl), Some(early_spl), Some(late_spl)) = (
        frequency,
        full.get("spl").and_then(Value::as_array),
        early.get("spl").and_then(Value::as_array),
        late.get("spl").and_then(Value::as_array),
    ) else {
        return Value::Null;
    };
    if frequency.is_empty()
        || !all_finite_positive_numbers(frequency)
        || !all_finite_numbers(full_spl)
        || !all_finite_numbers(early_spl)
        || !all_finite_numbers(late_spl)
        || full_spl.len() != frequency.len()
        || early_spl.len() != frequency.len()
        || late_spl.len() != frequency.len()
        || early.get("freq") != Some(&Value::Array(frequency.clone()))
        || late.get("freq") != Some(&Value::Array(frequency.clone()))
        || full_spl
            .iter()
            .zip(early_spl)
            .zip(late_spl)
            .any(|((full, early), late)| {
                full.as_f64().unwrap_or_default() + 1.0e-6 < early.as_f64().unwrap_or_default()
                    || full.as_f64().unwrap_or_default() + 1.0e-6
                        < late.as_f64().unwrap_or_default()
            })
    {
        return Value::Null;
    }
    let label = string_arg(request, "label", "");
    let traces = vec![
        series(
            "Full",
            &Value::Array(frequency.clone()),
            &Value::Array(full_spl.clone()),
            Some("#333333"),
            2.5,
            "solid",
            0,
        ),
        series(
            "Early",
            &Value::Array(frequency.clone()),
            &Value::Array(early_spl.clone()),
            Some("#3089c5"),
            2.0,
            "solid",
            0,
        ),
        series(
            "Late",
            &Value::Array(frequency.clone()),
            &Value::Array(late_spl.clone()),
            Some("#e38836"),
            2.0,
            "solid",
            0,
        ),
    ];
    figure(
        format!("{label}: early vs late band energy (20 ms split)"),
        axis("Frequency (Hz)", "log", json!(20), json!(20000)),
        axis(
            "Level vs full peak band (dB)",
            "linear",
            Value::Null,
            Value::Null,
        ),
        traces,
        Vec::new(),
        None,
        request.get("tab").cloned().unwrap_or(Value::Null),
    )
}

fn create_t60_octaves_figure(request: &Value) -> Value {
    let rows = request.get("rows").and_then(Value::as_array);
    let Some(rows) = rows.filter(|rows| {
        !rows.is_empty()
            && rows
                .iter()
                .any(|row| row.get("t60_s").is_some_and(|value| !value.is_null()))
    }) else {
        return Value::Null;
    };
    let centers: Vec<Value> = rows
        .iter()
        .map(|row| row.get("centre_hz").cloned().unwrap_or(Value::Null))
        .collect();
    let Some(first_center) = centers.first().and_then(number_like) else {
        return Value::Null;
    };
    let Some(last_center) = centers.last().and_then(number_like) else {
        return Value::Null;
    };
    let mut edges = vec![json!(first_center / std::f64::consts::SQRT_2)];
    for pair in centers.windows(2) {
        let (Some(left), Some(right)) = (number_like(&pair[0]), number_like(&pair[1])) else {
            return Value::Null;
        };
        edges.push(json!((left * right).sqrt()));
    }
    edges.push(json!(last_center * std::f64::consts::SQRT_2));
    let mut frequency = Vec::with_capacity(rows.len() * 2);
    let mut t60 = Vec::with_capacity(rows.len() * 2);
    for (index, row) in rows.iter().enumerate() {
        frequency.push(edges[index].clone());
        frequency.push(edges[index + 1].clone());
        let value = row.get("t60_s").cloned().unwrap_or(Value::Null);
        t60.push(value.clone());
        t60.push(value);
    }
    let mut traces = vec![series(
        "T60 (octave bands)",
        &Value::Array(frequency),
        &Value::Array(t60),
        Some("#3089c5"),
        2.0,
        "solid",
        0,
    )];
    if let Some(itu) = request
        .get("itu_reference")
        .filter(|value| !value.is_null())
    {
        let Some(tm) = itu.get("tm").and_then(number_like) else {
            return Value::Null;
        };
        traces.push(series(
            "ITU-R BS.1116-3 upper limit",
            &json!([63, 200, 4000, 4000, 8000]),
            &json!([tm + 0.30, tm + 0.05, tm + 0.05, tm + 0.10, tm + 0.10]),
            Some("#60ad42"),
            2.0,
            "solid",
            0,
        ));
        traces.push(series(
            "ITU-R BS.1116-3 lower limit",
            &json!([100, 4000, 4000, 8000]),
            &json!([tm - 0.05, tm - 0.05, tm - 0.10, tm - 0.10]),
            Some("#60ad42"),
            2.0,
            "solid",
            0,
        ));
    }
    if let Some(window) = request.get("window").filter(|value| !value.is_null()) {
        if let Some(window) = window.as_array() {
            for (name, value) in ["Lower flatness limit", "Upper flatness limit"]
                .into_iter()
                .zip(window)
            {
                let Some(value) = number_like(value) else {
                    continue;
                };
                traces.push(series(
                    &format!("{name}: {value:.3} s"),
                    &json!([20, 20000]),
                    &json!([value, value]),
                    Some("#60ad42"),
                    2.0,
                    "solid",
                    0,
                ));
            }
        }
    }
    figure(
        format!(
            "{}: measured octave-band T60",
            string_arg(request, "label", "")
        ),
        axis("Frequency (Hz)", "log", json!(20), json!(20000)),
        axis("T60 (s)", "linear", json!(0), Value::Null),
        traces,
        Vec::new(),
        None,
        request.get("tab").cloned().unwrap_or(Value::Null),
    )
}

fn to_db(request: &Value) -> Value {
    let output = request
        .get("amplitude")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    let Some(amplitude) = number_like(value) else {
                        return json!(IR_FLOOR_DB);
                    };
                    if amplitude.abs() > 1.0e-10 {
                        json!((20.0 * amplitude.abs().log10()).max(IR_FLOOR_DB))
                    } else {
                        json!(IR_FLOOR_DB)
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Value::Array(output)
}

fn figure(
    title: String,
    x: Value,
    y: Value,
    series: Vec<Value>,
    hlines: Vec<Value>,
    y2: Option<Value>,
    tab: Value,
) -> Value {
    json!({
        "kind": "figure",
        "figure": {
            "title": title,
            "x": x,
            "y": y,
            "y2": y2.unwrap_or(Value::Null),
            "series": series,
            "hlines": hlines,
            "vlines": [],
            "xranges": [],
            "annotations": [],
            "legend": true,
        },
        "tab": tab,
    })
}

fn axis(label: &str, scale: &str, minimum: Value, maximum: Value) -> Value {
    json!({"label": label, "scale": scale, "min": minimum, "max": maximum})
}

fn series(
    name: &str,
    x: &Value,
    y: &Value,
    color: Option<&str>,
    width: f64,
    dash: &str,
    y_axis: i64,
) -> Value {
    let x_values = x
        .as_array()
        .map(|values| values.iter().map(as_float_value).collect::<Vec<_>>())
        .unwrap_or_default();
    let y_values = y
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    if value.is_null() {
                        Value::Null
                    } else {
                        as_float_value(value)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "name": name,
        "x": x_values,
        "y": y_values,
        "color": color,
        "width": width,
        "dash": dash,
        "visible": true,
        "y_axis": y_axis,
    })
}

fn hline(at: f64, color: &str, dash: &str, width: f64, label: Value) -> Value {
    json!({"at": at, "color": color, "dash": dash, "width": width, "label": label})
}

fn non_boolean_number(value: &Value) -> Option<f64> {
    (!value.is_boolean()).then(|| value.as_f64()).flatten()
}

fn number_like(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_bool().map(|value| f64::from(u8::from(value))))
}

fn as_float_value(value: &Value) -> Value {
    number_like(value).map_or(Value::Null, |value| json!(value))
}

fn string_arg(request: &Value, key: &str, default: &str) -> String {
    request
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned()
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn all_finite_positive_numbers(values: &[Value]) -> bool {
    values
        .iter()
        .all(|value| number_like(value).is_some_and(|number| number.is_finite() && number > 0.0))
}

fn all_finite_numbers(values: &[Value]) -> bool {
    values
        .iter()
        .all(|value| number_like(value).is_some_and(f64::is_finite))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::build_temporal_figure;

    #[test]
    fn figure_builders_match_pre_port_python_goldens() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/temporal_figures_python_goldens.json"
        ))
        .expect("Python figure golden fixture parses");
        let inputs = &fixture["inputs"];
        let cases = [
            (
                "ir_before_after",
                "create_ir_figure",
                json!({
                    "channel_name": "L",
                    "pre_ir": inputs["ir_pre"],
                    "post_ir": inputs["ir_post"],
                    "display_ms": 80.0,
                    "tab": "time-ir",
                }),
            ),
            (
                "ir_unavailable",
                "create_ir_figure",
                json!({"channel_name": "L", "pre_ir": null, "post_ir": null, "tab": "time-ir"}),
            ),
            (
                "tof_before",
                "create_tof_figure",
                json!({"tof_rows": inputs["tof_rows"], "after": false, "tab": "timing"}),
            ),
            (
                "tof_after",
                "create_tof_figure",
                json!({"tof_rows": inputs["tof_rows"], "after": true, "tab": "timing"}),
            ),
            (
                "tof_unavailable",
                "create_tof_figure",
                json!({"tof_rows": [{"name": "L", "before_ms": null}], "tab": "timing"}),
            ),
            (
                "early_late_valid",
                "create_early_late_figure",
                json!({"label": "L", "report": inputs["early_late"], "tab": "energy"}),
            ),
            (
                "early_late_unavailable",
                "create_early_late_figure",
                json!({
                    "label": "L",
                    "report": {"method": "incoherent_band_energy", "reference": "foreign", "smoothing": "third_octave", "split_ms": 20.0},
                    "tab": "energy",
                }),
            ),
            (
                "t60_valid",
                "create_t60_octaves_figure",
                json!({
                    "label": "Room",
                    "rows": inputs["t60_rows"],
                    "tab": "decay",
                    "window": inputs["window"],
                    "itu_reference": inputs["itu_reference"],
                }),
            ),
            (
                "t60_unavailable",
                "create_t60_octaves_figure",
                json!({"label": "Room", "rows": [{"centre_hz": 63, "t60_s": null}], "tab": "decay"}),
            ),
            (
                "symmetric_pair_complex",
                "create_symmetric_pair_figure",
                json!({
                    "label": inputs["symmetric_pair"]["label"],
                    "freq": inputs["symmetric_pair"]["freq"],
                    "abs_spl": inputs["symmetric_pair"]["abs_spl"],
                    "complex_spl": inputs["symmetric_pair"]["complex_spl"],
                    "tab": "symmetric",
                }),
            ),
            (
                "symmetric_pair_no_phase",
                "create_symmetric_pair_figure",
                json!({
                    "label": inputs["symmetric_pair"]["label"],
                    "freq": inputs["symmetric_pair"]["freq"],
                    "abs_spl": inputs["symmetric_pair"]["abs_spl"],
                    "complex_spl": null,
                    "tab": "symmetric",
                }),
            ),
            (
                "symmetric_pair_mismatched",
                "create_symmetric_pair_figure",
                json!({
                    "label": inputs["symmetric_pair"]["label"],
                    "freq": inputs["symmetric_pair"]["freq"],
                    "abs_spl": inputs["symmetric_pair"]["abs_spl"],
                    "complex_spl": [-1.0, -2.0],
                    "tab": "symmetric",
                }),
            ),
            (
                "helper_to_db",
                "to_db",
                json!({"amplitude": [1.0, 0.5, 0.0, -1e-12]}),
            ),
            (
                "helper_finite_or_gap",
                "finite_or_gap",
                json!({"values": [1.0, null, true, "bad"]}),
            ),
        ];
        for (golden_name, builder, request) in cases {
            let actual = build_temporal_figure(builder, &request).expect("known builder");
            assert_json_close(&actual, &fixture["expected"][golden_name], golden_name);
        }
    }

    #[test]
    fn unknown_builder_is_an_error() {
        assert!(build_temporal_figure("unknown", &Value::Null).is_err());
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
                    (actual - expected).abs() <= 1.0e-12,
                    "{path}: {actual} != {expected}"
                );
            }
            _ => assert_eq!(actual, expected, "{path}"),
        }
    }
}
