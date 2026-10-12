//! Aggregates report provenance values while preserving dataset encounter order.
//!
//! The Python boundary remains responsible for resolving workspace versions and
//! collecting trusted dataset timestamps. This module canonicalizes those
//! prepared values into the report payload shared by native, CLI, and WASM.

// Rust guideline compliant 2026-02-21

use serde_json::{Map, Value};
use std::collections::HashSet;

fn unique_strings(value: &Value) -> Vec<String> {
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    if let Some(entries) = value.as_array() {
        for entry in entries {
            let Some(text) = entry.as_str() else {
                continue;
            };
            let trimmed = text.trim();
            if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                values.push(trimmed.to_string());
            }
        }
    }
    values
}

fn unique_timestamps(value: &Value) -> Vec<String> {
    let mut timestamps = Vec::new();
    let mut seen = HashSet::new();
    if let Some(entries) = value.as_array() {
        for entry in entries {
            let Some(text) = entry.as_str() else {
                continue;
            };
            let trimmed = text.trim();
            // Match the legacy Python check: compare the raw timestamp to
            // previously stored (trimmed) values, then append the trimmed one.
            if !trimmed.is_empty() && !seen.contains(text) {
                timestamps.push(trimmed.to_string());
                seen.insert(trimmed.to_string());
            }
        }
    }
    timestamps
}

/// Builds the final report provenance object from prepared dataset fields.
///
/// Versions are trimmed before deduplication, with first-seen order preserved.
/// Timestamps preserve legacy behavior: each raw value is checked against
/// previously stored trimmed values, then the trimmed value is appended. This
/// can retain a duplicate when a padded value follows its unpadded equivalent.
/// Empty collections are omitted from the result.
/// `generated_at` is copied as supplied by the host so clock ownership remains
/// with the report orchestration boundary.
///
/// # Examples
/// ```
/// use serde_json::json;
/// use autoeq_report_wasm::report_provenance::report_provenance;
///
/// let prepared = json!({
///     "versions": ["1.0", "1.0", "2.0"],
///     "timestamps": ["t1", "t1"],
///     "generated_at": "2026-10-11T12:34:56Z"
/// });
/// let output = report_provenance(&prepared);
/// assert_eq!(output["roomeq_version"], "1.0 / 2.0");
/// assert_eq!(output["data_timestamp"], "t1");
/// ```
///
/// # Panics
/// This function does not panic for JSON values.
pub fn report_provenance(prepared: &Value) -> Value {
    let mut result = Map::new();
    let versions = unique_strings(&prepared["versions"]).join(" / ");
    if !versions.is_empty() {
        result.insert("roomeq_version".to_string(), Value::String(versions));
    }
    let timestamps = unique_timestamps(&prepared["timestamps"]).join(" / ");
    if !timestamps.is_empty() {
        result.insert("data_timestamp".to_string(), Value::String(timestamps));
    }
    if let Some(generated_at) = prepared.get("generated_at").and_then(Value::as_str) {
        result.insert(
            "generated_at".to_string(),
            Value::String(generated_at.to_string()),
        );
    }
    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::report_provenance;

    #[test]
    fn matches_captured_python_report_provenance_goldens() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../scripts/tests/fixtures/report_provenance_python_goldens.json"
        ))
        .expect("golden fixture is valid JSON");
        for case in fixture["cases"].as_array().expect("cases array") {
            assert_eq!(
                report_provenance(&case["prepared"]),
                case["expected"],
                "case {}",
                case["name"]
            );
        }
    }
}
