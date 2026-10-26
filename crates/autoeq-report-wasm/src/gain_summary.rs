//! Canonical HTML summary for deployed gain plugins.

// Rust guideline compliant 2026-02-21

use serde_json::Value;

use crate::report_helpers::{channel_sort_key, escape_html, python_string};

/// Renders the deployed gain plugins in global, channel, and driver order.
///
/// The rows describe individual plugin entries; values are not summed because
/// route-owned gains and crossover responses also affect playback.
pub fn gain_summary_html(data: &Value) -> String {
    let mut owners: Vec<(String, Vec<&Value>)> = Vec::new();
    owners.push((
        "Global".to_string(),
        object_array(data.get("global_plugins")),
    ));

    if let Some(channels) = data.get("channels").and_then(Value::as_object) {
        let mut names: Vec<&str> = channels.keys().map(String::as_str).collect();
        names.sort_by(|left, right| channel_sort_key(left).cmp(&channel_sort_key(right)));
        for name in names {
            let Some(channel) = channels.get(name).and_then(Value::as_object) else {
                continue;
            };
            owners.push((
                format!("Channel {name}"),
                object_array(channel.get("plugins")),
            ));
            if let Some(drivers) = channel.get("drivers").and_then(Value::as_array) {
                for (index, driver) in drivers.iter().enumerate() {
                    let Some(driver) = driver.as_object() else {
                        continue;
                    };
                    let label = driver
                        .get("name")
                        .filter(|name| is_truthy(name))
                        .map(python_string)
                        .unwrap_or_else(|| index.to_string());
                    owners.push((
                        format!("Channel {name} / driver {label}"),
                        object_array(driver.get("plugins")),
                    ));
                }
            }
        }
    }

    let mut rows = String::new();
    for (owner, plugins) in owners {
        for (index, plugin) in plugins.into_iter().enumerate() {
            if plugin.get("plugin_type").and_then(Value::as_str) != Some("gain") {
                continue;
            }
            let params = plugin.get("parameters").and_then(Value::as_object);
            let gain_value = params.and_then(|parameters| parameters.get("gain_db"));
            let gain = match gain_value {
                None => "+0.000 dB".to_string(),
                Some(value) => crate::report_helpers::numeric_value(value)
                    .map(|gain| format!("{gain:+.3} dB"))
                    .unwrap_or_else(|| "invalid".to_string()),
            };
            let stage = params
                .and_then(|parameters| parameters.get("room_eq_stage"))
                .map(python_string)
                .unwrap_or_else(|| "unspecified".to_string());
            let label = params
                .and_then(|parameters| parameters.get("label"))
                .map(python_string)
                .unwrap_or_else(|| "unlabelled gain".to_string());
            rows.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape_html(&owner),
                index + 1,
                escape_html(&stage),
                escape_html(&label),
                gain,
            ));
        }
    }
    if rows.is_empty() {
        return String::new();
    }

    format!(
        concat!(
            "<div class=\"plot-container\"><h2>Level Gains and Safety Attenuation</h2>",
            "<p class=\"epa-footer\">The EQ-only overview does not show these gain plugins. ",
            "They affect the post-DSP response even when the EQ trace is flat. ",
            "Entries are listed in plugin order within each owner; they are not a summed ",
            "input-to-output gain. Routing gains and crossover responses also affect playback. ",
            "Route-owned entries describe routing and must not be counted twice.</p>",
            "<table class=\"bm-table\"><thead><tr><th>Owner</th><th>Plugin #</th>",
            "<th>Stage</th><th>Purpose</th><th>Gain</th></tr></thead><tbody>{rows}",
            "</tbody></table></div>"
        ),
        rows = rows
    )
}

fn object_array(value: Option<&Value>) -> Vec<&Value> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter(|item| item.is_object()).collect())
        .unwrap_or_default()
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
        Value::Bool(true) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::gain_summary_html;

    const PYTHON_GOLDEN: &str = concat!(
        "<div class=\"plot-container\"><h2>Level Gains and Safety Attenuation</h2><p class=\"epa-footer\">The EQ-only overview does not show these gain plugins. ",
        "They affect the post-DSP response even when the EQ trace is flat. Entries are listed in plugin order within each owner; they are not a summed input-to-output gain. ",
        "Routing gains and crossover responses also affect playback. Route-owned entries describe routing and must not be counted twice.</p><table class=\"bm-table\"><thead><tr><th>Owner</th><th>Plugin #</th><th>Stage</th><th>Purpose</th><th>Gain</th></tr></thead><tbody>",
        "<tr><td>Global</td><td>1</td><td>pre_route</td><td>global trim</td><td>-1.000 dB</td></tr>",
        "<tr><td>Channel L</td><td>1</td><td>pre_route</td><td>final_electrical_headroom</td><td>-13.112 dB</td></tr>",
        "<tr><td>Channel L</td><td>2</td><td>post_route</td><td>level alignment</td><td>-9.429 dB</td></tr>",
        "<tr><td>Channel LFE / driver woofer&lt;script&gt;</td><td>1</td><td>post_route</td><td>safety&lt;script&gt;</td><td>-3.560 dB</td></tr>",
        "</tbody></table></div>"
    );

    #[test]
    fn exactly_matches_captured_python_gain_summary() {
        let data = serde_json::json!({
            "global_plugins": [{"plugin_type":"gain","parameters":{"gain_db":-1.0,"room_eq_stage":"pre_route","label":"global trim"}}],
            "channels": {
                "L": {"plugins": [
                    {"plugin_type":"gain","parameters":{"gain_db":-13.11163,"room_eq_stage":"pre_route","label":"final_electrical_headroom"}},
                    {"plugin_type":"gain","parameters":{"gain_db":-9.42897,"room_eq_stage":"post_route","label":"level alignment"}}
                ]},
                "LFE": {"drivers": [{"name":"woofer<script>","plugins":[
                    {"plugin_type":"gain","parameters":{"gain_db":-3.56015,"room_eq_stage":"post_route","label":"safety<script>"}}
                ]}]}
            }
        });
        assert_eq!(gain_summary_html(&data), PYTHON_GOLDEN);
    }

    #[test]
    fn empty_and_invalid_gain_entries_are_safe() {
        assert_eq!(
            gain_summary_html(&serde_json::json!({"channels":{"L":{"plugins":[]}}})),
            ""
        );
        let html = gain_summary_html(&serde_json::json!({"global_plugins":[
            {"plugin_type":"gain","parameters":{"gain_db":"bad","label":"x'\"&"}}
        ]}));
        assert!(html.contains("invalid"));
        assert!(html.contains("x&#x27;&quot;&amp;"));
    }
}
