//! Builds renderer-neutral figures for bass routing and headroom.
//!
//! Routing views preserve saved route order and add one fan-out link for each
//! physical output driver in canonical channel order. Headroom views preserve
//! per-output order and show absent numeric readings as zero, matching the
//! standalone report. Crossover transfer functions are not calculated here.
//!
//! # Examples
//!
//! ```
//! use autoeq_report_wasm::bass_figures::{headroom_figure, routing_figure};
//! use serde_json::json;
//! let data = json!({"metadata":{"bass_management":{"routing_graph":{"routes":[]}}}});
//! assert!(routing_figure(&data, None).is_none());
//! assert!(headroom_figure(&data, None).is_none());
//! ```
//!
//! # Errors
//!
//! Missing figure inputs return `None`; malformed bar samples become zero as
//! in the legacy figure builder.
//!
//! # Panics
//!
//! This module does not intentionally panic for caller-provided report values.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;

use serde_json::Value;

use crate::report_helpers::{channel_sort_key, numeric_value, python_string};
use crate::schema::{BarChart, BarGroup, DashOption, LineMark, SankeyChart, SankeyLink, Section};

/// Keeps routing links translucent like the legacy bass graph palette.
const ROUTE_ALPHA: f64 = 0.58;
/// Preserves a visible band for zero-gain branches in the Sankey renderer.
const MIN_LINK_VALUE: f64 = 0.05;

/// Builds a Sankey figure for the saved bass-management routing graph.
///
/// Routes remain in serialized order. Driver fan-out links follow the report's
/// established classical channel order and retain each driver's saved order.
///
/// # Examples
///
/// ```
/// use autoeq_report_wasm::bass_figures::routing_figure;
/// use serde_json::json;
/// let data = json!({"metadata":{"bass_management":{"routing_graph":{"routes":[
///     {"source_channel":"L","destination":"L","route_kind":"main_highpass_to_self"}
/// ]}}}});
/// let figure = routing_figure(&data, Some("Bass")).unwrap();
/// assert_eq!(figure["kind"], "sankey");
/// assert_eq!(figure["tab"], "Bass");
/// ```
///
/// # Errors
///
/// Returns `None` when the saved routing graph has no routes.
///
/// # Panics
///
/// This function does not intentionally panic for malformed report data.
pub fn routing_figure(data: &Value, tab: Option<&str>) -> Option<Value> {
    let report = bass_management_report(data);
    let routes = report
        .pointer("/routing_graph/routes")
        .and_then(Value::as_array)?;
    if routes.is_empty() {
        return None;
    }

    let mut node_index: HashMap<String, usize> = HashMap::new();
    let mut labels = Vec::new();
    let mut links = Vec::new();
    for route in routes {
        let source_name = string_or(route.get("source_channel"), "?");
        let destination_name = string_or(route.get("destination"), "?");
        let route_kind = string_or(route.get("route_kind"), "route");
        let gain = truthy(route.get("gain_linear"))
            .then(|| route.get("gain_linear"))
            .flatten()
            .or_else(|| {
                truthy(route.get("matrix_gain"))
                    .then(|| route.get("matrix_gain"))
                    .flatten()
            })
            .and_then(coerce_float)
            .filter(|value| value.is_finite())
            .unwrap_or(1.0);
        let source = add_node("in", &source_name, &mut node_index, &mut labels);
        let target = add_node("out", &destination_name, &mut node_index, &mut labels);
        links.push(SankeyLink {
            source,
            target,
            value: gain.abs().max(MIN_LINK_VALUE),
            color: Some(route_color(&route_kind)),
        });
    }

    let mut channels: Vec<(&str, &Value)> = data
        .get("channels")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|channels| {
            channels
                .iter()
                .map(|(name, channel)| (name.as_str(), channel))
        })
        .collect();
    channels.sort_by(|(left, _), (right, _)| channel_sort_key(left).cmp(&channel_sort_key(right)));
    for (channel_name, channel_data) in channels {
        let bus_key = format!("out:{channel_name}");
        let Some(&bus_index) = node_index.get(&bus_key) else {
            continue;
        };
        let Some(drivers) = channel_data.get("drivers").and_then(Value::as_array) else {
            continue;
        };
        for (index, driver) in drivers.iter().enumerate() {
            if !driver.is_object() {
                continue;
            }
            let driver_name = driver
                .get("name")
                .map(python_string)
                .unwrap_or_else(|| format!("driver_{index}"));
            let gain_db = driver_alignment_gain_db(driver);
            let value = 10.0_f64.powf(gain_db / 20.0);
            let value = if value.is_finite() {
                value.max(MIN_LINK_VALUE)
            } else {
                1.0
            };
            let target = add_node("sub", &driver_name, &mut node_index, &mut labels);
            links.push(SankeyLink {
                source: bus_index,
                target,
                value,
                color: Some(driver_link_color()),
            });
        }
    }

    serialize_section(Section::Sankey {
        chart: SankeyChart {
            node_boxes: false,
            title: "Bass Management Routing Graph".to_string(),
            nodes: labels,
            links,
        },
        tab: tab.map(str::to_string),
    })
}

/// Builds grouped per-output bars for the saved bass headroom simulation.
///
/// Missing or unparseable measurements are zero, matching the legacy report.
///
/// # Examples
///
/// ```
/// use autoeq_report_wasm::bass_figures::headroom_figure;
/// use serde_json::json;
/// let data = json!({"metadata":{"bass_management":{"headroom_simulation":{"per_output":[]}}}});
/// assert!(headroom_figure(&data, None).is_none());
/// ```
///
/// # Errors
///
/// Returns `None` when there are no per-output simulation rows.
///
/// # Panics
///
/// This function does not intentionally panic for malformed report data.
pub fn headroom_figure(data: &Value, tab: Option<&str>) -> Option<Value> {
    let report = bass_management_report(data);
    let outputs = report
        .pointer("/headroom_simulation/per_output")
        .and_then(Value::as_array)?;
    if outputs.is_empty() {
        return None;
    }
    let categories = outputs
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            entry
                .get("output_role")
                .map(python_string)
                .unwrap_or_else(|| format!("out {}", index + 1))
        })
        .collect();
    let values_for = |key: &str| {
        outputs
            .iter()
            .map(|entry| coerce_bar_value(entry.get(key)))
            .collect()
    };
    let margin = report
        .pointer("/headroom_simulation/headroom_margin_db")
        .and_then(numeric_value);
    let hlines = margin
        .filter(|value| value.is_finite())
        .map(|value| {
            vec![LineMark {
                at: value,
                color: "rgba(40, 40, 40, 0.7)".to_string(),
                dash: DashOption::Dash,
                width: 2.0,
                label: Some(format!("headroom limit {value:+.1} dB")),
            }]
        })
        .unwrap_or_default();
    serialize_section(Section::Bar {
        chart: BarChart {
            title: "Bass Bus Headroom Simulation".to_string(),
            categories,
            groups: vec![
                BarGroup {
                    name: "RMS programme gain".to_string(),
                    values: values_for("rms_bus_gain_db"),
                    color: Some("rgba(74, 144, 217, 0.75)".to_string()),
                },
                BarGroup {
                    name: "Coherent peak gain".to_string(),
                    values: values_for("coherent_peak_gain_db"),
                    color: Some("rgba(231, 76, 60, 0.78)".to_string()),
                },
                BarGroup {
                    name: "LFE contribution".to_string(),
                    values: values_for("lfe_contribution_db"),
                    color: Some("rgba(230, 126, 34, 0.72)".to_string()),
                },
            ],
            ylabel: "Gain vs programme reference (dB)".to_string(),
            hlines,
            ymin: None,
            ymax: None,
            legend: true,
        },
        tab: tab.map(str::to_string),
    })
}

fn bass_management_report(data: &Value) -> &Value {
    data.pointer("/metadata/bass_management")
        .filter(|value| truthy(Some(value)))
        .unwrap_or(&Value::Null)
}

fn add_node(
    prefix: &str,
    name: &str,
    indexes: &mut HashMap<String, usize>,
    labels: &mut Vec<String>,
) -> usize {
    let key = format!("{prefix}:{name}");
    if let Some(index) = indexes.get(&key) {
        return *index;
    }
    let index = labels.len();
    indexes.insert(key, index);
    labels.push(format!("{prefix}: {name}"));
    index
}

fn driver_alignment_gain_db(driver: &Value) -> f64 {
    let mut gain = 0.0;
    if let Some(plugins) = driver.get("plugins").and_then(Value::as_array) {
        for plugin in plugins {
            if plugin
                .get("plugin_type")
                .and_then(Value::as_str)
                .is_none_or(|kind| !kind.eq_ignore_ascii_case("gain"))
            {
                continue;
            }
            if let Some(value) = plugin
                .pointer("/parameters/gain_db")
                .and_then(coerce_float)
                .filter(|value| value.is_finite())
            {
                gain += value;
            }
        }
    }
    gain
}

fn coerce_bar_value(value: Option<&Value>) -> f64 {
    let Some(value) = value else {
        return 0.0;
    };
    let number = coerce_float(value);
    number.filter(|number| number.is_finite()).unwrap_or(0.0)
}

fn coerce_float(value: &Value) -> Option<f64> {
    numeric_value(value).or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn string_or(value: Option<&Value>, default: &str) -> String {
    value
        .map(python_string)
        .unwrap_or_else(|| default.to_string())
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(value)) => !value.is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
    }
}

fn route_color(route_kind: &str) -> String {
    let rgba = match route_kind {
        "main_highpass_to_self" => "74, 144, 217",
        "redirected_bass_lowpass_to_sub" => "46, 204, 113",
        "lfe_lowpass_to_sub" => "231, 126, 34",
        _ => "127, 140, 141",
    };
    format!("rgba({rgba}, {ROUTE_ALPHA:.2})")
}

fn driver_link_color() -> String {
    format!("rgba(148, 103, 189, {ROUTE_ALPHA:.2})")
}

fn serialize_section(section: Section) -> Option<Value> {
    let mut value = serde_json::to_value(section).ok()?;
    if value.get("kind").and_then(Value::as_str) == Some("sankey") {
        // The Python schema builder omits this optional extension when false.
        value
            .get_mut("chart")?
            .as_object_mut()?
            .remove("node_boxes");
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_routes_and_no_headroom_rows_have_no_section() {
        let data = json!({"metadata":{"bass_management":{"routing_graph":{"routes":[]},"headroom_simulation":{"per_output":[]}}}});
        assert!(routing_figure(&data, None).is_none());
        assert!(headroom_figure(&data, None).is_none());
    }

    #[test]
    fn route_order_precedes_classically_sorted_driver_fanout() {
        let data = json!({
            "channels": {
                "SUB": {"drivers": [{"name":"sub driver","plugins":[]}]},
                "LFE": {"drivers": [{"name":"LFE driver","plugins":[]}]}
            },
            "metadata": {"bass_management": {"routing_graph": {"routes": [
                {"source_channel":"LFE","destination":"LFE","route_kind":"lfe_lowpass_to_sub","gain_linear":0.25},
                {"source_channel":"LFE","destination":"SUB","route_kind":"lfe_lowpass_to_sub","gain_linear":0.25},
                {"source_channel":"L","destination":"L","route_kind":"main_highpass_to_self","gain_linear":0.5}
            ]}}}
        });
        let section = routing_figure(&data, Some("Bass")).unwrap();
        assert_eq!(section["chart"]["nodes"][0], "in: LFE");
        assert_eq!(section["chart"]["nodes"][5], "sub: LFE driver");
        assert_eq!(section["chart"]["nodes"][6], "sub: sub driver");
    }

    #[test]
    fn headroom_missing_values_are_zero_and_margin_is_labeled() {
        let data = json!({"metadata":{"bass_management":{"headroom_simulation":{
            "headroom_margin_db":-3.2,
            "per_output":[{"output_role":"Sub A","rms_bus_gain_db":null,"coherent_peak_gain_db":"2.5"}]
        }}}});
        let section = headroom_figure(&data, None).unwrap();
        assert_eq!(section["chart"]["groups"][0]["values"][0], 0.0);
        assert_eq!(section["chart"]["groups"][1]["values"][0], 2.5);
        assert_eq!(
            section["chart"]["hlines"][0]["label"],
            "headroom limit -3.2 dB"
        );
    }
}
