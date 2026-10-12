//! Builds the shared upstream report fragments consumed by native and Python callers.

// Rust guideline compliant 2026-02-21

use serde_json::{Map, Value};

use crate::bass_management::bass_management_sections;
use crate::{crossover_config_html, gain_summary_html, signal_flow_sections, time_of_flight_html};

/// Builds all renderer-neutral report sections from one full RoomEQ result value.
///
/// Strings hold HTML fragments. `signal_flow_sections` remains an array of
/// ordinary report section objects so the shared report shell can render it
/// through its existing section schema.
pub fn build_shared_sections(data: &Value) -> Map<String, Value> {
    let mut sections = Map::new();
    let metadata = data.get("metadata").unwrap_or(&Value::Null);
    let bass = metadata.get("bass_management").unwrap_or(&Value::Null);
    let bass_sections = bass_management_sections(bass);

    sections.insert(
        "time_of_flight_html".to_string(),
        Value::String(time_of_flight_html(metadata)),
    );
    sections.insert(
        "crossover_config_html".to_string(),
        Value::String(crossover_config_html(data)),
    );
    sections.insert(
        "gain_summary_html".to_string(),
        Value::String(gain_summary_html(data)),
    );
    sections.insert(
        "bass_management_summary_html".to_string(),
        Value::String(bass_sections.summary_html),
    );
    sections.insert(
        "bass_management_groups_table_html".to_string(),
        Value::String(bass_sections.groups_table_html),
    );
    sections.insert(
        "bass_management_sub_outputs_table_html".to_string(),
        Value::String(bass_sections.sub_outputs_table_html),
    );
    sections.insert(
        "signal_flow_sections".to_string(),
        Value::Array(signal_flow_sections(data)),
    );
    sections
}

/// Builds global evidence HTML from an already prepared graph and its
/// externally verified payload-binding result.
pub fn build_global_evidence_sections(
    data: &Value,
    label: &str,
    payload_binding: &Value,
) -> Map<String, Value> {
    crate::evidence_sections::global_evidence_sections(data, label, payload_binding)
}

#[cfg(test)]
mod tests {
    use super::build_shared_sections;

    #[test]
    fn builds_all_shared_report_sections_in_one_value() {
        let data = serde_json::json!({
            "global_plugins": [{"plugin_type":"gain","parameters":{"gain_db":-1.0}}],
            "channels":{"L":{"plugins":[{"plugin_type":"gain","parameters":{"gain_db":-2.0}}]}},
            "metadata":{"time_of_flight":{"measurements":[]}, "bass_management":{"enabled":true}}
        });
        let sections = build_shared_sections(&data);
        assert!(
            sections["gain_summary_html"]
                .as_str()
                .unwrap()
                .contains("-1.000 dB")
        );
        assert!(
            sections["bass_management_summary_html"]
                .as_str()
                .unwrap()
                .contains("Bass Management")
        );
        assert!(sections["time_of_flight_html"].is_string());
        assert!(sections["crossover_config_html"].is_string());
        assert!(sections["signal_flow_sections"].is_array());
    }
}
