//! Builds the renderer-neutral combined RoomEQ overview figures.
//!
//! DSP replay, response smoothing, channel ordering, and display-range analysis
//! remain at the established Python preparation boundary. This module turns
//! those prepared traces into the three ordered `autoeq-report-data-v1` figure
//! sections used by native and WebAssembly renderers.
// Rust guideline compliant 2026-02-21

use serde_json::{Value, json};

const CHANNEL_COLORS: [&str; 10] = [
    "rgba(31, 119, 180, 0.9)",
    "rgba(255, 127, 14, 0.9)",
    "rgba(44, 160, 44, 0.9)",
    "rgba(214, 39, 40, 0.9)",
    "rgba(148, 103, 189, 0.9)",
    "rgba(140, 86, 75, 0.9)",
    "rgba(227, 119, 194, 0.9)",
    "rgba(127, 127, 127, 0.9)",
    "rgba(188, 189, 34, 0.9)",
    "rgba(23, 190, 207, 0.9)",
];

/// Converts prepared combined-overview traces into ordered figure sections.
///
/// `prepared` carries series roles, labels, coordinates, and axis domains
/// after the Python report boundary has completed DSP replay and smoothing.
/// Missing fields produce empty series or established default axis domains.
/// The `tab` value is copied into every returned section.
///
/// # Examples
/// ```
/// use serde_json::json;
/// use autoeq_report_wasm::combined_figures::combined_figures;
///
/// let sections = combined_figures(&json!({"has_channels": true}), None);
/// assert_eq!(sections.len(), 3);
/// ```
///
/// # Panics
/// This function does not panic for malformed or incomplete JSON values.
pub fn combined_figures(prepared: &Value, tab: Option<&Value>) -> Vec<Value> {
    let tab = tab.cloned().unwrap_or(Value::Null);
    if !prepared["has_channels"].as_bool().unwrap_or(false) {
        return vec![figure_section(
            "Combined Overview — Before EQ",
            "SPL (dB)",
            -20.0,
            30.0,
            Vec::new(),
            Vec::new(),
            tab,
        )];
    }

    let before = prepared_series(prepared.get("before_series"));
    let mut eq = prepared_series(prepared.get("eq_series"));
    if let Some(span) = prepared["eq_frequency_extent"].as_array()
        && span.len() == 2
    {
        eq.push(json!({
            "name": "0 dB",
            "x": span,
            "y": [0.0, 0.0],
            "color": "rgba(150, 150, 150, 0.5)",
            "width": 1.0,
            "dash": "dash",
            "visible": true,
            "y_axis": 0
        }));
    }
    let corrected = prepared_series(prepared.get("corrected_series"));
    let marks = crossover_marks(prepared.get("crossovers"));
    let spl_min = finite_number(&prepared["spl_y_min"]).unwrap_or(-20.0);
    let spl_max = finite_number(&prepared["spl_y_max"]).unwrap_or(30.0);
    let eq_min = finite_number(&prepared["eq_y_lower"]).unwrap_or(-15.0);
    let eq_max = finite_number(&prepared["eq_y_upper"]).unwrap_or(15.0);

    vec![
        figure_section(
            "Combined Overview — Before EQ",
            "SPL (dB)",
            spl_min,
            spl_max,
            before,
            marks.clone(),
            tab.clone(),
        ),
        figure_section(
            "Combined Overview — EQ",
            "Gain (dB)",
            eq_min,
            eq_max,
            eq,
            marks.clone(),
            tab.clone(),
        ),
        figure_section(
            "Combined Overview — Corrected",
            "SPL (dB)",
            spl_min,
            spl_max,
            corrected,
            marks,
            tab,
        ),
    ]
}

fn prepared_series(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let color = item
                .get("color_index")
                .and_then(Value::as_u64)
                .map(|index| CHANNEL_COLORS[(index % CHANNEL_COLORS.len() as u64) as usize]);
            let color = color.unwrap_or(CHANNEL_COLORS[0]);
            let dash = item["dash"].as_str().unwrap_or("solid");
            let width = finite_number(&item["width"]).unwrap_or(2.0);
            json!({
                "name": item["name"].as_str().unwrap_or(""),
                "x": item["x"].as_array().cloned().unwrap_or_default(),
                "y": item["y"].as_array().cloned().unwrap_or_default(),
                "color": color,
                "width": width,
                "dash": dash,
                "visible": true,
                "y_axis": 0
            })
        })
        .collect()
}

fn crossover_marks(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let frequency = finite_number(value)?;
            let label = if frequency >= 1000.0 {
                format!("{:.1}k", frequency / 1000.0)
            } else {
                format!("{frequency:.0}")
            };
            Some(json!({
                "at": frequency,
                "color": "rgba(180, 80, 180, 0.7)",
                "dash": "dashdot",
                "width": 1.5,
                "label": format!("Xover {label} Hz")
            }))
        })
        .collect()
}

fn figure_section(
    title: &str,
    y_label: &str,
    y_min: f64,
    y_max: f64,
    series: Vec<Value>,
    vlines: Vec<Value>,
    tab: Value,
) -> Value {
    json!({
        "kind": "figure",
        "figure": {
            "title": title,
            "x": {"label": "Frequency (Hz)", "scale": "log", "min": 20.0, "max": 20000.0},
            "y": {"label": y_label, "scale": "linear", "min": y_min, "max": y_max},
            "y2": null,
            "series": series,
            "hlines": [],
            "vlines": vlines,
            "xranges": [],
            "annotations": [],
            "legend": true
        },
        "tab": tab
    })
}

fn finite_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::combined_figures;

    #[test]
    fn creates_three_panels_and_keeps_trace_order_and_tab() {
        let prepared = json!({
            "has_channels": true,
            "before_series": [{"name":"L before","x":[20,40],"y":[70,71],"color_index":0}],
            "eq_series": [{"name":"EQ: L","x":[20,40],"y":[0.5,-0.5],"color_index":0}],
            "corrected_series": [{"name":"L corrected","x":[20,40],"y":[71,72],"color_index":0}],
            "eq_frequency_extent": [20,40],
            "crossovers": [80, 1200],
            "spl_y_min": 20,
            "spl_y_max": 70,
            "eq_y_lower": -10,
            "eq_y_upper": 10
        });
        let tab = json!("Overview");
        let result = combined_figures(&prepared, Some(&tab));
        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["figure"]["series"][0]["name"], "L before");
        assert_eq!(result[1]["figure"]["series"][1]["name"], "0 dB");
        assert_eq!(result[1]["figure"]["vlines"][1]["label"], "Xover 1.2k Hz");
        assert!(result.iter().all(|section| section["tab"] == tab));
    }

    #[test]
    fn empty_channel_input_preserves_legacy_single_panel() {
        let result = combined_figures(&json!({"has_channels": false}), None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["figure"]["y"]["min"], -20.0);
        assert_eq!(result[0]["figure"]["y"]["max"], 30.0);
    }

    #[test]
    fn malformed_prepared_series_are_tolerated() {
        let result = combined_figures(
            &json!({"has_channels": true, "before_series": [null, 4]}),
            None,
        );
        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["figure"]["series"][0]["name"], "");
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(serde_json::from_str::<Value>(&encoded).is_ok());
    }
}
