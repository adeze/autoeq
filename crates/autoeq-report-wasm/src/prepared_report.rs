//! Versioned in-memory inputs for portable report preparation.
//!
//! This module defines a transport DTO only. It performs no file access, DSP,
//! or HTML generation. Graph data stays as JSON so newer graph fields survive
//! older report-preparation code, while target and replay state remain explicit.

// Rust guideline compliant 2026-02-21

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Version of the prepared report input contract.
pub const PREPARED_REPORT_SCHEMA_VERSION: u32 = 1;

/// Portable input snapshot passed to report preparation.
///
/// The graph and optional metadata values preserve fields unknown to this
/// crate. `target_snapshot` retains the native loader's tagged snapshot shape,
/// and `replay` retains the native producer's metadata-only statuses and
/// context. The native producer keeps decoded FIR taps outside its serialized
/// transport; this DTO does not itself inspect arbitrary opaque context fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreparedReportData {
    /// Version of this DTO contract.
    pub schema_version: u32,
    /// Hydrated graph JSON, including unknown forward-compatible fields.
    pub graph: Value,
    /// Optional host-supplied provenance metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
    /// Optional captured measurement index, including unrecognized fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement_index: Option<Value>,
    /// Optional symmetric-pair metadata retained from the measurement index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symmetric_pairs: Option<Value>,
    /// Explicit target snapshot with `state` set to not_configured, available, or unavailable.
    pub target_snapshot: Value,
    /// Optional native replay statuses and context; absence means no replay context was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay: Option<PreparedReplay>,
}

/// Native replay context with its authoritative optional playback sample rate.
///
/// Every field other than `sample_rate_hz` is retained in `context`, preserving
/// channel, driver, routed-source, and FIR availability statuses. The native
/// producer omits captured FIR taps from its serialized transport; this DTO
/// treats the remaining context as opaque JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedReplay {
    /// Explicit report playback rate, when the host supplied one.
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
    /// Remaining native replay transport fields, including unknown future fields.
    #[serde(flatten)]
    pub context: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedReportDataWire {
    schema_version: u32,
    graph: Value,
    #[serde(default)]
    provenance: Option<Value>,
    #[serde(default)]
    measurement_index: Option<Value>,
    #[serde(default)]
    symmetric_pairs: Option<Value>,
    target_snapshot: Value,
    #[serde(default)]
    replay: Option<PreparedReplay>,
}

impl<'de> Deserialize<'de> for PreparedReportData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PreparedReportDataWire::deserialize(deserializer)?;
        let prepared = Self {
            schema_version: wire.schema_version,
            graph: wire.graph,
            provenance: wire.provenance,
            measurement_index: wire.measurement_index,
            symmetric_pairs: wire.symmetric_pairs,
            target_snapshot: wire.target_snapshot,
            replay: wire.replay,
        };
        prepared.validate().map_err(de::Error::custom)?;
        Ok(prepared)
    }
}

impl PreparedReportData {
    /// Construct and validate a prepared report input.
    ///
    /// # Examples
    /// ```
    /// use autoeq_report_wasm::PreparedReportData;
    /// use serde_json::json;
    ///
    /// let prepared = PreparedReportData::new(
    ///     json!({"channels": {}}),
    ///     None,
    ///     None,
    ///     None,
    ///     json!({"schema_version": 1, "state": "not_configured", "target": null}),
    ///     None,
    /// ).unwrap();
    /// assert_eq!(prepared.schema_version, 1);
    /// ```
    ///
    /// # Errors
    /// Returns an error when the schema version, JSON object shapes, target
    /// state, or optional replay rate violates this DTO's contract.
    pub fn new(
        graph: Value,
        provenance: Option<Value>,
        measurement_index: Option<Value>,
        symmetric_pairs: Option<Value>,
        target_snapshot: Value,
        replay: Option<PreparedReplay>,
    ) -> Result<Self, String> {
        let prepared = Self {
            schema_version: PREPARED_REPORT_SCHEMA_VERSION,
            graph,
            provenance,
            measurement_index,
            symmetric_pairs,
            target_snapshot,
            replay,
        };
        prepared.validate()?;
        Ok(prepared)
    }

    /// Validate the versioned DTO without altering any captured JSON values.
    ///
    /// # Errors
    /// Returns an error for unsupported schema versions, malformed required
    /// objects, inconsistent target snapshot fields, or a zero replay rate.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PREPARED_REPORT_SCHEMA_VERSION {
            return Err(format!(
                "unsupported prepared report schema version {}",
                self.schema_version
            ));
        }
        require_object(&self.graph, "graph")?;
        if let Some(provenance) = &self.provenance {
            require_object(provenance, "provenance")?;
        }
        if let Some(index) = &self.measurement_index {
            require_object(index, "measurement_index")?;
        }
        if let Some(pairs) = &self.symmetric_pairs {
            require_object(pairs, "symmetric_pairs")?;
        }
        validate_target_snapshot(&self.target_snapshot)?;
        if let Some(replay) = &self.replay
            && replay.sample_rate_hz == Some(0)
        {
            return Err("replay sample_rate_hz must be a positive integer".into());
        }
        Ok(())
    }
}

fn require_object(value: &Value, field: &str) -> Result<(), String> {
    if value.is_object() {
        Ok(())
    } else {
        Err(format!("prepared report {field} must be an object"))
    }
}

fn validate_target_snapshot(snapshot: &Value) -> Result<(), String> {
    let object = snapshot
        .as_object()
        .ok_or_else(|| "prepared report target_snapshot must be an object".to_owned())?;
    if object.get("schema_version").and_then(Value::as_u64)
        != Some(u64::from(PREPARED_REPORT_SCHEMA_VERSION))
    {
        return Err("target_snapshot schema_version must be 1".into());
    }
    let state = object
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| "target_snapshot state must be a string".to_owned())?;
    let target = object
        .get("target")
        .ok_or_else(|| "target_snapshot must contain target".to_owned())?;
    match state {
        "not_configured" => {
            if !target.is_null() {
                return Err("not_configured target_snapshot must have a null target".into());
            }
        }
        "available" => {
            let curve = target
                .as_object()
                .ok_or_else(|| "available target_snapshot target must be an object".to_owned())?;
            if !curve.get("freq").is_some_and(Value::is_array)
                || !curve.get("spl").is_some_and(Value::is_array)
            {
                return Err(
                    "available target_snapshot target must contain freq and spl arrays".into(),
                );
            }
        }
        "unavailable" => {
            if !target.is_null() {
                return Err("unavailable target_snapshot must have a null target".into());
            }
            let error_kind = object.get("error_kind").and_then(Value::as_str);
            if !matches!(error_kind, Some("warning" | "csv_error")) {
                return Err(
                    "unavailable target_snapshot error_kind must be warning or csv_error".into(),
                );
            }
            if !object.get("error").is_some_and(Value::is_string) {
                return Err("unavailable target_snapshot must contain an error string".into());
            }
        }
        _ => {
            return Err(
                "target_snapshot state must be not_configured, available, or unavailable".into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PREPARED_REPORT_SCHEMA_VERSION, PreparedReplay, PreparedReportData};
    use serde_json::{Value, json};

    fn target_snapshot(state: &str, target: Value) -> Value {
        let source_integrity = if state == "not_configured" {
            "not_configured"
        } else {
            "stable_file_snapshot"
        };
        let source_verification = if state == "not_configured" {
            "not_applicable"
        } else {
            "external_unverified"
        };
        let error_kind = if state == "unavailable" {
            json!("warning")
        } else {
            Value::Null
        };
        let error = if state == "unavailable" {
            json!("target unavailable")
        } else {
            Value::Null
        };
        json!({
            "schema_version": 1,
            "state": state,
            "configured_path": null,
            "source_integrity": source_integrity,
            "source_verification": source_verification,
            "target": target,
            "error_kind": error_kind,
            "error": error,
        })
    }

    #[test]
    fn roundtrip_preserves_unknown_graph_target_and_replay_fields() {
        let input = json!({
            "schema_version": PREPARED_REPORT_SCHEMA_VERSION,
            "graph": {"channels": {}, "future_graph_field": {"opaque": [1, "two"]}},
            "provenance": {"producer": "host", "future_provenance": true},
            "measurement_index": {"version": 1, "future_index_field": "retained"},
            "symmetric_pairs": {"L-R": {"future_pair_field": 9}},
            "target_snapshot": {
                "schema_version": 1,
                "state": "available",
                "target": {"freq": [20.0, 20000.0], "spl": [75.0, 72.0]},
                "source_verification": "external_unverified",
                "future_target_field": "retained"
            },
            "replay": {
                "sample_rate_hz": 48000,
                "channels": {"L": {"status": "available"}},
                "fir_resource_statuses": {"ir.wav": {"status": "available"}},
                "future_replay_field": {"opaque": true}
            }
        });

        let parsed: PreparedReportData = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(parsed.replay.as_ref().unwrap().sample_rate_hz, Some(48_000));
        assert_eq!(
            parsed.graph["future_graph_field"],
            json!({"opaque": [1, "two"]})
        );
        assert_eq!(parsed.target_snapshot["future_target_field"], "retained");
        assert_eq!(
            parsed.replay.as_ref().unwrap().context["future_replay_field"],
            json!({"opaque": true})
        );
        assert_eq!(serde_json::to_value(parsed).unwrap(), input);
    }

    #[test]
    fn replay_without_rate_remains_explicit_and_distinct_from_absence() {
        let prepared = PreparedReportData::new(
            json!({"channels": {}}),
            None,
            None,
            None,
            target_snapshot("not_configured", Value::Null),
            Some(PreparedReplay {
                sample_rate_hz: None,
                context: serde_json::Map::from_iter([(
                    "unavailable_reason".into(),
                    json!("rate not supplied"),
                )]),
            }),
        )
        .unwrap();
        assert_eq!(prepared.replay.as_ref().unwrap().sample_rate_hz, None);
        let encoded = serde_json::to_value(prepared).unwrap();
        let decoded: PreparedReportData = serde_json::from_value(encoded).unwrap();
        assert!(decoded.replay.is_some());
    }

    #[test]
    fn rejects_invalid_schema_shapes_and_replay_rates() {
        let valid_target = target_snapshot("not_configured", Value::Null);
        let base = json!({
            "schema_version": 1,
            "graph": {"channels": {}},
            "target_snapshot": valid_target
        });
        for (invalid, expected) in [
            (
                {
                    let mut v = base.clone();
                    v["schema_version"] = json!(2);
                    v
                },
                "schema version",
            ),
            (
                {
                    let mut v = base.clone();
                    v["graph"] = json!([]);
                    v
                },
                "graph must be an object",
            ),
            (
                {
                    let mut v = base.clone();
                    v["target_snapshot"]["state"] = json!("mystery");
                    v
                },
                "target_snapshot state",
            ),
            (
                {
                    let mut v = base.clone();
                    v["replay"] = json!({"sample_rate_hz": 0});
                    v
                },
                "positive integer",
            ),
            (
                {
                    let mut v = base.clone();
                    v["replay"] = json!({"sample_rate_hz": true});
                    v
                },
                "invalid type",
            ),
        ] {
            let result = serde_json::from_value::<PreparedReportData>(invalid.clone());
            let error = result
                .expect_err("invalid DTO must be rejected")
                .to_string();
            assert!(
                error.contains(expected),
                "unexpected error: {error}; input={invalid}"
            );
        }
    }

    #[test]
    fn validates_available_and_unavailable_target_payload_shapes() {
        let available = PreparedReportData::new(
            json!({"channels": {}}),
            None,
            None,
            None,
            target_snapshot("available", json!({"freq": [20.0], "spl": [75.0]})),
            None,
        );
        assert!(available.is_ok());

        let unavailable = PreparedReportData::new(
            json!({"channels": {}}),
            None,
            None,
            None,
            target_snapshot("unavailable", Value::Null),
            None,
        );
        assert!(unavailable.is_ok());

        let bad_unavailable = PreparedReportData::new(
            json!({"channels": {}}),
            None,
            None,
            None,
            json!({"schema_version":1,"state":"unavailable","target":null,"error_kind":"unknown","error":"bad"}),
            None,
        );
        assert!(bad_unavailable.is_err());
    }
}
