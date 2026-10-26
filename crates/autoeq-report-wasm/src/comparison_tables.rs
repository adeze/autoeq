//! Renders comparison-only tables from prepared serialized evidence.
//!
//! These builders format existing report evidence and do not calculate
//! acoustic or psychoacoustic metrics.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::evidence_sections::{EPA_FIELDS, evidence_section};
use crate::report_helpers::escape_html;

/// Builds per-channel EPA comparison tables from prepared mode evidence.
///
/// Each channel maps to its ordered list of mode labels and saved `pre` / `post`
/// objects. Channels without comparable evidence produce an empty HTML string.
/// The result is a JSON object keyed by channel name so a report can render all
/// comparison tables with one producer call.
///
/// # Examples
/// ```
/// use autoeq_report_wasm::comparison_tables::epa_comparison_tables;
/// use serde_json::json;
///
/// let tables = epa_comparison_tables(&json!({"channels": {"L": []}})).unwrap();
/// assert_eq!(tables["L"], "");
/// ```
///
/// # Errors
/// Returns a producer error if the shared EPA delta formatter fails.
///
/// # Panics
/// This function does not panic for malformed or incomplete JSON values.
pub fn epa_comparison_tables(prepared: &Value) -> Result<Value, String> {
    let mut output = Map::new();
    if let Some(channels) = prepared["channels"].as_object() {
        for (channel, entries) in channels {
            output.insert(
                channel.clone(),
                json!(render_epa_comparison_table(entries)?),
            );
        }
    }
    Ok(Value::Object(output))
}

/// Builds the categorical pre/post score chart used by mode comparison reports.
///
/// The prepared values are authoritative metadata; this function only selects
/// labels, explanatory title text, and the shared bar-chart schema.
///
/// # Examples
/// ```
/// use autoeq_report_wasm::comparison_tables::score_comparison_figure;
/// use serde_json::json;
///
/// let figure = score_comparison_figure(&json!({"scores": [], "loss_types": []}));
/// assert_eq!(figure["kind"], "bar");
/// ```
///
/// # Panics
/// This function does not panic for malformed or incomplete JSON values.
pub fn score_comparison_figure(prepared: &Value) -> Value {
    let scores = prepared["scores"].as_array().cloned().unwrap_or_default();
    let categories = scores
        .iter()
        .map(|score| score["label"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    let before = scores
        .iter()
        .map(|score| numeric(&score["pre"]).map_or(Value::Null, |number| json!(number)))
        .collect::<Vec<_>>();
    let after = scores
        .iter()
        .map(|score| numeric(&score["post"]).map_or(Value::Null, |number| json!(number)))
        .collect::<Vec<_>>();
    let distinct_losses: BTreeSet<_> = prepared["loss_types"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|loss_type| !loss_type.is_empty())
        .collect();
    let mut title = "Flat-loss before / after EQ (lower is better)".to_string();
    if distinct_losses.len() >= 2 {
        title.push_str(" — Note: bars show the flat-loss metric for every run, even ones that optimized EPA — this is so all modes stay on a single comparable scale. For perceptual comparison see the EPA Pref columns above.");
    }
    json!({
        "kind": "bar",
        "chart": {
            "title": title,
            "categories": categories,
            "groups": [
                {"name":"Before EQ","values":before,"color":"rgba(200, 200, 200, 0.7)"},
                {"name":"After EQ","values":after,"color":null}
            ],
            "ylabel":"Flat loss (lower is better)",
            "ymin":null,
            "ymax":null,
            "legend":true,
            "hlines":[]
        },
        "tab":prepared.get("tab").cloned().unwrap_or(Value::Null)
    })
}

fn render_epa_comparison_table(entries: &Value) -> Result<String, String> {
    let entries = entries.as_array().cloned().unwrap_or_default();
    let Some(baseline) = entries.first() else {
        return Ok(String::new());
    };

    let mode_headers = entries
        .iter()
        .map(|entry| {
            format!(
                "<th>{} (post)</th>",
                escape_html(entry["mode_label"].as_str().unwrap_or_default())
            )
        })
        .collect::<String>();
    let header_row = format!("<tr><th>Metric</th><th>Before EQ</th>{mode_headers}</tr>");
    let mut body_rows = Vec::new();

    for (field, label, precision) in EPA_FIELDS {
        let pre = &baseline["pre"][*field];
        let pre_value = numeric(pre);
        let pre_string = pre_value
            .map(|value| format!("{value:+.precision$}"))
            .unwrap_or_else(|| "-".to_string());
        let mut mode_cells = String::new();
        for entry in &entries {
            let post = &entry["post"][*field];
            let cell = if let Some(post_value) = numeric(post) {
                let post_string = format!("{post_value:+.precision$}");
                if pre_value.is_some() {
                    let delta = evidence_section(&json!({
                        "builder": "delta",
                        "field": field,
                        "pre": pre,
                        "post": post,
                    }))?;
                    let delta_html = delta
                        .as_str()
                        .unwrap_or("<span style=\"color:#999\">-</span>");
                    format!(
                        "<td>{post_string} <span style=\"font-size:0.85em\">{delta_html}</span></td>"
                    )
                } else {
                    format!("<td>{post_string}</td>")
                }
            } else {
                "<td style=\"color:#999\">-</td>".to_string()
            };
            mode_cells.push_str(&cell);
        }
        body_rows.push(format!(
            "<tr><td>{label}</td><td>{pre_string}</td>{mode_cells}</tr>"
        ));
    }

    Ok(format!(
        "<div class=\"epa-section\">\n    <h3>EPA Psychoacoustic Scores (by mode)</h3>\n    <table class=\"epa-table\">\n        <thead>{header_row}</thead>\n        <tbody>\n            {}\n        </tbody>\n    </table>\n    <p class=\"epa-footer\">Higher is better for Preference / Evaluation / Total loudness / Loudness balance; lower is better for Activity / Sharpness deviation / Roughness. Δ colours show whether each mode improved the metric versus the pre-EQ baseline.</p>\n</div>\n",
        body_rows.join("\n            ")
    ))
}

fn numeric(value: &Value) -> Option<f64> {
    value.as_f64().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::epa_comparison_tables;

    #[test]
    fn renders_missing_values_and_escapes_mode_labels() {
        let output = epa_comparison_tables(&json!({
            "channels": {
                "L": [
                    {"mode_label":"<img src=x>","pre":{"preference":1.0},"post":{"preference":2.0}},
                    {"mode_label":"FIR","pre":{},"post":{"preference":null}}
                ],
                "Empty": []
            }
        }))
        .unwrap();
        let table = output["L"].as_str().unwrap();
        assert!(table.contains("&lt;img src=x&gt; (post)"));
        assert!(!table.contains("<img"));
        assert!(table.contains("<td style=\"color:#999\">-</td>"));
        assert_eq!(output["Empty"], "");
    }

    #[test]
    fn formats_score_comparison_chart_and_mixed_loss_note() {
        let figure = super::score_comparison_figure(&json!({
            "scores":[{"label":"IIR (EPA)","pre":0.5,"post":0.4}],
            "loss_types":["flat","epa"],
            "tab":"Summary"
        }));
        assert_eq!(figure["kind"], "bar");
        assert_eq!(figure["chart"]["categories"][0], "IIR (EPA)");
        assert_eq!(figure["chart"]["groups"][0]["values"][0], 0.5);
        assert!(
            figure["chart"]["title"]
                .as_str()
                .unwrap()
                .contains("For perceptual comparison see the EPA Pref columns above.")
        );
        assert_eq!(figure["tab"], "Summary");
    }
}
