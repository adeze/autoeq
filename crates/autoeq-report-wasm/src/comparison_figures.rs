//! Assembles comparison figures from prepared plot arrays and labels.
//!
//! Smoothing, filter responses, phase wrapping, group delay, and impulse
//! response values are prepared by the existing report boundary. This module
//! only creates the shared figure-section JSON schema.

// Rust guideline compliant 2026-02-21

use serde_json::{Value, json};

/// Builds comparison figure sections from prepared figure descriptors.
///
/// Each input descriptor contains a title, axes, prepared series, and optional
/// annotations or tab. The returned array preserves descriptor order.
///
/// # Examples
/// ```
/// use autoeq_report_wasm::comparison_figures::comparison_figure_sections;
/// use serde_json::json;
///
/// let figures = comparison_figure_sections(&json!({"figures": []}));
/// assert!(figures.as_array().unwrap().is_empty());
/// ```
///
/// # Panics
/// This function does not panic for malformed or incomplete JSON values.
pub fn comparison_figure_sections(prepared: &Value) -> Value {
    let sections = prepared["figures"]
        .as_array()
        .into_iter()
        .flatten()
        .map(figure_section)
        .collect::<Vec<_>>();
    Value::Array(sections)
}

fn figure_section(spec: &Value) -> Value {
    let series = spec["series"]
        .as_array()
        .into_iter()
        .flatten()
        .map(trace)
        .collect::<Vec<_>>();
    json!({
        "kind":"figure",
        "figure": {
            "title":spec.get("title").cloned().unwrap_or_else(|| json!("")),
            "x":spec.get("x").cloned().unwrap_or(Value::Null),
            "y":spec.get("y").cloned().unwrap_or(Value::Null),
            "y2":spec.get("y2").cloned().unwrap_or(Value::Null),
            "series":series,
            "hlines":spec.get("hlines").and_then(Value::as_array).cloned().unwrap_or_default(),
            "vlines":spec.get("vlines").and_then(Value::as_array).cloned().unwrap_or_default(),
            "xranges":spec.get("xranges").and_then(Value::as_array).cloned().unwrap_or_default(),
            "annotations":spec.get("annotations").and_then(Value::as_array).cloned().unwrap_or_default(),
            "legend":spec.get("legend").and_then(Value::as_bool).unwrap_or(true),
        },
        "tab":spec.get("tab").cloned().unwrap_or(Value::Null),
    })
}

fn trace(spec: &Value) -> Value {
    let numeric_width = spec["width"].as_f64().filter(|width| width.is_finite());
    json!({
        "name":spec.get("name").cloned().unwrap_or_else(|| json!("")),
        "x":spec.get("x").and_then(Value::as_array).cloned().unwrap_or_default(),
        "y":spec.get("y").and_then(Value::as_array).cloned().unwrap_or_default(),
        "color":spec.get("color").cloned().unwrap_or(Value::Null),
        "width":numeric_width.unwrap_or(2.0),
        "dash":spec.get("dash").and_then(Value::as_str).unwrap_or("solid"),
        "visible":true,
        "y_axis":0,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::comparison_figure_sections;

    #[test]
    fn assembles_figures_without_changing_series_order_or_tabs() {
        let result = comparison_figure_sections(&json!({
            "figures":[
                {"title":"L: Mode Comparison","x":{"label":"Frequency (Hz)","scale":"log","min":20,"max":20000},
                 "y":{"label":"SPL (dB)","scale":"linear","min":45,"max":95},"tab":"L",
                 "series":[{"name":"Before EQ","x":[20],"y":[70]},{"name":"IIR","x":[20],"y":[69]}]}
            ]
        }));
        assert_eq!(result[0]["kind"], "figure");
        assert_eq!(result[0]["figure"]["series"][0]["name"], "Before EQ");
        assert_eq!(result[0]["figure"]["series"][1]["name"], "IIR");
        assert_eq!(result[0]["tab"], "L");
        assert_eq!(result[0]["figure"]["y2"], serde_json::Value::Null);
        assert_eq!(result[0]["figure"]["hlines"], json!([]));
    }
}
