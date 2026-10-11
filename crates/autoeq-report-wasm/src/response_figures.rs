//! Builds standard before-and-after response figures from serialized curves.
//!
//! The builders preserve the standalone report's chart schema and fixed labels.
//! Curve smoothing delegates to the existing shared curve implementation, and
//! log-frequency resampling is limited to the display grid used by these plots.

// Rust guideline compliant 2026-02-21

use serde_json::{Value, json};

use crate::acoustic_curves::smooth_curve;
use crate::report_data::{average_spl, compute_y_range};

const CHANNEL_SMOOTHING_OCTAVES: f64 = 1.0 / 6.0;
const DEFAULT_ZOOM_MIN_HZ: f64 = 20.0;
const DEFAULT_ZOOM_MAX_HZ: f64 = 1200.0;
const DEFAULT_ZOOM_Y_RANGE_DB: f64 = 25.0;
const DEFAULT_SMOOTHED_OCTAVES: f64 = 1.0;

/// Builds the four standard response figures for one channel.
///
/// The input contains a channel name and optional initial, final, target, and
/// prepared LFE-sum curves. `min_freq`, `max_freq`, and `y_range` configure the
/// zoomed figure; `smoothed_octaves` configures the separate smoothed figure.
/// The LFE-plus-channel response is supplied by the existing routing/DSP
/// boundary because it depends on the saved graph and deployed processing.
///
/// # Examples
/// ```
/// use serde_json::json;
/// use autoeq_report_wasm::response_figures::response_figure_report;
///
/// let result = response_figure_report(&json!({"channel_name":"L"}));
/// assert!(result["channel_figure"].is_object());
/// assert!(result["smoothed_figure"].is_null());
/// ```
///
/// # Panics
/// This function does not panic for malformed or incomplete JSON values.
pub fn response_figure_report(data: &Value) -> Value {
    let channel_name = data
        .get("channel_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let initial = data.get("initial_curve").filter(|curve| is_truthy(curve));
    let final_curve = data.get("final_curve").filter(|curve| is_truthy(curve));
    let target = data.get("target_curve").filter(|curve| is_truthy(curve));
    let lfe_sum = data
        .get("lfe_plus_channel_curve")
        .filter(|curve| is_truthy(curve));
    let tab = data.get("tab").cloned().unwrap_or(Value::Null);
    let title_suffix = data
        .get("title_suffix")
        .and_then(Value::as_str)
        .unwrap_or("");

    let response = channel_figure(
        channel_name,
        initial,
        final_curve,
        title_suffix,
        tab.clone(),
    );
    let overlaid = add_channel_response_overlays(response.clone(), channel_name, target, lfe_sum);

    let min_freq = finite_number(data.get("min_freq")).unwrap_or(DEFAULT_ZOOM_MIN_HZ);
    let max_freq = finite_number(data.get("max_freq")).unwrap_or(DEFAULT_ZOOM_MAX_HZ);
    let y_range = finite_number(data.get("y_range")).unwrap_or(DEFAULT_ZOOM_Y_RANGE_DB);
    let smoothed_octaves =
        finite_number(data.get("smoothed_octaves")).unwrap_or(DEFAULT_SMOOTHED_OCTAVES);
    let zoomed = zoomed_figure(
        channel_name,
        initial,
        final_curve,
        min_freq,
        max_freq,
        y_range,
        tab.clone(),
    );
    let smoothed = smoothed_figure(channel_name, initial, final_curve, smoothed_octaves, tab);

    json!({
        "channel_figure": response,
        "channel_figure_with_overlays": overlaid,
        "zoomed_figure": zoomed,
        "smoothed_figure": smoothed,
    })
}

/// Resamples an SPL curve onto a display grid using log-frequency interpolation.
///
/// Source pairs are finite-filtered, sorted, and edge-clamped, matching the
/// previous Python helper. If resampling cannot be performed, the source SPL
/// array is returned unchanged.
///
/// # Panics
/// This function does not panic for malformed JSON arrays.
pub fn resample_spl_onto_grid(
    source_freq: &Value,
    source_spl: &Value,
    target_freq: &Value,
) -> Value {
    let (Some(source_freq), Some(source_spl), Some(target_freq)) = (
        source_freq.as_array(),
        source_spl.as_array(),
        target_freq.as_array(),
    ) else {
        return source_spl.clone();
    };
    if source_freq.is_empty() || target_freq.is_empty() || source_freq.len() != source_spl.len() {
        return Value::Array(source_spl.clone());
    }
    if source_freq.len() == target_freq.len()
        && source_freq
            .iter()
            .zip(target_freq)
            .all(|(source, target)| source == target)
    {
        return Value::Array(source_spl.clone());
    }

    let mut pairs: Vec<(f64, f64)> = source_freq
        .iter()
        .zip(source_spl)
        .filter_map(|(frequency, level)| {
            let frequency = python_number(frequency)?;
            let level = python_number(level)?;
            (frequency.is_finite() && level.is_finite() && frequency > 0.0)
                .then_some((frequency, level))
        })
        .collect();
    pairs.sort_by(|left, right| left.0.total_cmp(&right.0));
    if pairs.is_empty() {
        return Value::Array(source_spl.clone());
    }
    if pairs.len() == 1 {
        return Value::Array(target_freq.iter().map(|_| json!(pairs[0].1)).collect());
    }

    let log_frequencies: Vec<f64> = pairs
        .iter()
        .map(|(frequency, _)| frequency.log10())
        .collect();
    let levels: Vec<f64> = pairs.iter().map(|(_, level)| *level).collect();
    let output: Vec<Value> = target_freq
        .iter()
        .map(|frequency| {
            let Some(frequency) = python_number(frequency) else {
                return Value::Null;
            };
            if frequency <= 0.0 {
                return json!(levels[0]);
            }
            let log_frequency = frequency.log10();
            if log_frequency <= log_frequencies[0] {
                return json!(levels[0]);
            }
            if log_frequency >= *log_frequencies.last().unwrap_or(&log_frequency) {
                return json!(*levels.last().unwrap_or(&levels[0]));
            }
            let upper = log_frequencies.partition_point(|source| *source <= log_frequency);
            let lower = upper - 1;
            let span = log_frequencies[upper] - log_frequencies[lower];
            let position = if span > 0.0 {
                (log_frequency - log_frequencies[lower]) / span
            } else {
                0.0
            };
            json!(levels[lower] + position * (levels[upper] - levels[lower]))
        })
        .collect();
    Value::Array(output)
}

fn channel_figure(
    channel_name: &str,
    initial: Option<&Value>,
    final_curve: Option<&Value>,
    title_suffix: &str,
    tab: Value,
) -> Value {
    let mut series = Vec::new();
    let initial_freq = initial.and_then(|curve| curve.get("freq"));
    if let Some(curve) = initial {
        if let Some(smoothed) = smoothed_levels(curve, CHANNEL_SMOOTHING_OCTAVES) {
            series.push(make_series(
                "Before EQ",
                curve.get("freq").unwrap_or(&Value::Null),
                &smoothed,
                "rgba(255, 100, 100, 0.8)",
                2.0,
                "solid",
            ));
        }
    }
    if let Some(curve) = final_curve {
        let target_freq = initial_freq.unwrap_or_else(|| curve.get("freq").unwrap_or(&Value::Null));
        let plot_spl = resample_spl_onto_grid(
            curve.get("freq").unwrap_or(&Value::Null),
            curve.get("spl").unwrap_or(&Value::Null),
            target_freq,
        );
        if let Some(levels) = array_numbers(&plot_spl) {
            let frequencies = if initial_freq.is_some() {
                target_freq.clone()
            } else {
                curve.get("freq").cloned().unwrap_or(Value::Null)
            };
            let smooth_curve_value = json!({"freq":frequencies,"spl":levels});
            if let Some(smoothed) = smoothed_levels(&smooth_curve_value, CHANNEL_SMOOTHING_OCTAVES)
            {
                series.push(make_series(
                    "After EQ",
                    smooth_curve_value.get("freq").unwrap_or(&Value::Null),
                    &smoothed,
                    "rgba(100, 200, 100, 0.9)",
                    2.0,
                    "solid",
                ));
            }
        }
    }
    let curves = json!([
        initial.cloned().unwrap_or(Value::Null),
        final_curve.cloned().unwrap_or(Value::Null)
    ]);
    let (y_min, y_max) = compute_y_range(&curves);
    figure(
        format!("Channel: {channel_name}{title_suffix}"),
        axis("Frequency (Hz)", "log", Some(20.0), Some(20_000.0)),
        axis("SPL (dB)", "linear", Some(y_min), Some(y_max)),
        series,
        tab,
    )
}

/// Adds target and prepared LFE-sum series to an existing response figure.
///
/// This separate entry point preserves callers that already built a channel
/// figure and then append overlays in a later report stage.
pub fn add_channel_response_overlays(
    mut figure: Value,
    channel_name: &str,
    target: Option<&Value>,
    lfe_sum: Option<&Value>,
) -> Value {
    let Some(series) = figure["figure"]["series"].as_array_mut() else {
        return figure;
    };
    if let Some(curve) = target.filter(|curve| curve_is_plot_ready(curve)) {
        series.push(make_series(
            "Target",
            curve.get("freq").unwrap_or(&Value::Null),
            curve.get("spl").unwrap_or(&Value::Null),
            "rgba(40, 40, 40, 0.9)",
            2.0,
            "dot",
        ));
    }
    if let Some(curve) = lfe_sum.filter(|curve| curve_is_plot_ready(curve)) {
        if let Some(smoothed) = smoothed_levels(curve, CHANNEL_SMOOTHING_OCTAVES) {
            series.push(make_series(
                format!("LFE + {channel_name}"),
                curve.get("freq").unwrap_or(&Value::Null),
                &smoothed,
                "rgba(148, 103, 189, 0.95)",
                2.5,
                "solid",
            ));
        }
    }
    figure
}

fn zoomed_figure(
    channel_name: &str,
    initial: Option<&Value>,
    final_curve: Option<&Value>,
    min_freq: f64,
    max_freq: f64,
    y_range: f64,
    tab: Value,
) -> Value {
    let reference = final_curve.or(initial);
    let average = reference
        .map(|curve| average_spl(curve, min_freq, max_freq))
        .unwrap_or(0.0);
    let mut series = Vec::new();
    if let Some(curve) = initial {
        if let Some(smoothed) = smoothed_levels(curve, CHANNEL_SMOOTHING_OCTAVES) {
            series.push(make_series(
                "Before EQ",
                curve.get("freq").unwrap_or(&Value::Null),
                &smoothed,
                "rgba(255, 100, 100, 0.8)",
                2.0,
                "solid",
            ));
        }
    }
    if let Some(curve) = final_curve {
        let plot_spl = resample_spl_onto_grid(
            curve.get("freq").unwrap_or(&Value::Null),
            curve.get("spl").unwrap_or(&Value::Null),
            initial
                .and_then(|initial| initial.get("freq"))
                .unwrap_or_else(|| curve.get("freq").unwrap_or(&Value::Null)),
        );
        if let Some(levels) = array_numbers(&plot_spl) {
            let frequencies = initial
                .and_then(|initial| initial.get("freq"))
                .cloned()
                .unwrap_or_else(|| curve.get("freq").cloned().unwrap_or(Value::Null));
            let resampled = json!({"freq":frequencies,"spl":levels});
            if let Some(smoothed) = smoothed_levels(&resampled, CHANNEL_SMOOTHING_OCTAVES) {
                series.push(make_series(
                    "After EQ",
                    resampled.get("freq").unwrap_or(&Value::Null),
                    &smoothed,
                    "rgba(100, 200, 100, 0.9)",
                    2.0,
                    "solid",
                ));
            }
        }
    }
    series.push(make_series(
        format!("Average ({average:.1} dB)"),
        &json!([min_freq, max_freq]),
        &json!([average, average]),
        "rgba(150, 150, 150, 0.5)",
        1.0,
        "dash",
    ));
    figure(
        format!(
            "Channel: {channel_name} (Zoom {}-{} Hz)",
            min_freq as i64, max_freq as i64
        ),
        axis("Frequency (Hz)", "log", Some(min_freq), Some(max_freq)),
        axis(
            "SPL (dB)",
            "linear",
            Some(average - y_range),
            Some(average + y_range),
        ),
        series,
        tab,
    )
}

fn smoothed_figure(
    channel_name: &str,
    initial: Option<&Value>,
    final_curve: Option<&Value>,
    octaves: f64,
    tab: Value,
) -> Value {
    if initial.is_none() && final_curve.is_none() {
        return Value::Null;
    }
    let mut series = Vec::new();
    for (label, curve, color) in [
        (
            "Before EQ (1-oct smoothed)",
            initial,
            "rgba(255, 100, 100, 0.8)",
        ),
        (
            "After EQ (1-oct smoothed)",
            final_curve,
            "rgba(100, 200, 100, 0.9)",
        ),
    ] {
        if let Some(curve) = curve.filter(|curve| curve_is_plot_ready(curve)) {
            if let Some(smoothed) = smoothed_levels(curve, octaves) {
                series.push(make_series(
                    label,
                    curve.get("freq").unwrap_or(&Value::Null),
                    &smoothed,
                    color,
                    2.0,
                    "solid",
                ));
            }
        }
    }
    let curves = json!([
        initial.cloned().unwrap_or(Value::Null),
        final_curve.cloned().unwrap_or(Value::Null)
    ]);
    let (y_min, y_max) = compute_y_range(&curves);
    figure(
        format!("Smoothed response (1 oct): {channel_name}"),
        axis("Frequency (Hz)", "log", Some(20.0), Some(20_000.0)),
        axis("SPL (dB)", "linear", Some(y_min), Some(y_max)),
        series,
        tab,
    )
}

fn smoothed_levels(curve: &Value, octaves: f64) -> Option<Value> {
    let source = curve.get("spl")?.as_array()?;
    if source.is_empty() {
        return Some(Value::Array(Vec::new()));
    }
    smooth_curve(curve, octaves)
        .and_then(|smoothed| smoothed.get("spl").cloned())
        .or_else(|| Some(Value::Array(source.clone())))
}

fn curve_is_plot_ready(curve: &Value) -> bool {
    curve
        .get("freq")
        .and_then(Value::as_array)
        .is_some_and(|values| !values.is_empty())
        && curve
            .get("spl")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.is_empty())
}

fn array_numbers(values: &Value) -> Option<Vec<Value>> {
    let values = values.as_array()?;
    values
        .iter()
        .map(|value| {
            let number = python_number(value)?;
            number.is_finite().then(|| json!(number))
        })
        .collect()
}

fn python_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_bool().map(|value| if value { 1.0 } else { 0.0 }))
}

fn finite_number(value: Option<&Value>) -> Option<f64> {
    let number = value.and_then(python_number)?;
    number.is_finite().then_some(number)
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn make_series(
    name: impl Into<String>,
    x: &Value,
    y: &Value,
    color: &str,
    width: f64,
    dash: &str,
) -> Value {
    json!({
        "name": name.into(),
        "x": x.as_array().cloned().unwrap_or_default().into_iter().map(|value| {
            json!(python_number(&value).unwrap_or(0.0))
        }).collect::<Vec<_>>(),
        "y": y.as_array().cloned().unwrap_or_default().into_iter().map(|value| {
            if value.is_null() { Value::Null } else { json!(python_number(&value).unwrap_or(0.0)) }
        }).collect::<Vec<_>>(),
        "color": color,
        "width": width,
        "dash": dash,
        "visible": true,
        "y_axis": 0,
    })
}

fn axis(label: &str, scale: &str, min: Option<f64>, max: Option<f64>) -> Value {
    json!({"label":label,"scale":scale,"min":min,"max":max})
}

fn figure(title: String, x: Value, y: Value, series: Vec<Value>, tab: Value) -> Value {
    json!({
        "kind":"figure",
        "figure":{
            "title":title,
            "x":x,
            "y":y,
            "y2":null,
            "series":series,
            "hlines":[],
            "vlines":[],
            "xranges":[],
            "annotations":[],
            "legend":true,
        },
        "tab":tab,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{resample_spl_onto_grid, response_figure_report};

    fn close(actual: &Value, expected: &Value) {
        match (actual, expected) {
            (Value::Number(actual), Value::Number(expected)) => {
                let actual = actual.as_f64().unwrap();
                let expected = expected.as_f64().unwrap();
                assert!(
                    (actual - expected).abs() <= 1.0e-10,
                    "{actual} != {expected}"
                );
            }
            (Value::Array(actual), Value::Array(expected)) => {
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.iter().zip(expected) {
                    close(actual, expected);
                }
            }
            (Value::Object(actual), Value::Object(expected)) => {
                assert_eq!(actual.len(), expected.len());
                for (key, expected) in expected {
                    close(actual.get(key).unwrap_or(&Value::Null), expected);
                }
            }
            _ => assert_eq!(actual, expected),
        }
    }

    #[test]
    fn response_figure_payloads_match_pre_port_python_goldens() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../scripts/tests/fixtures/response_figures_python_goldens.json"
        ))
        .unwrap();
        assert_eq!(
            fixtures["schema"],
            "autoeq-response-figures-python-golden-v1"
        );
        for case in fixtures["cases"].as_object().unwrap().values() {
            close(
                &response_figure_report(&case["input"]),
                &Value::Object(
                    [
                        ("channel_figure".to_string(), case["channel_figure"].clone()),
                        (
                            "channel_figure_with_overlays".to_string(),
                            case["channel_figure_with_overlays"].clone(),
                        ),
                        ("zoomed_figure".to_string(), case["zoomed_figure"].clone()),
                        (
                            "smoothed_figure".to_string(),
                            case["smoothed_figure"].clone(),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            );
        }
    }

    #[test]
    fn resampling_filters_sorts_and_edge_clamps_display_points() {
        assert_eq!(
            resample_spl_onto_grid(
                &serde_json::json!([1000, 100, 10]),
                &serde_json::json!([30, 10, 0]),
                &serde_json::json!([1, 10, 100, 1000, 10000]),
            ),
            serde_json::json!([0.0, 0.0, 10.0, 30.0, 30.0])
        );
    }
}
