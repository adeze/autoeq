//! Shared display options for graph-to-dashboard report composition.
//!
//! Acquisition, target capture, DSP replay, and bundle identity belong to
//! `PreparedReportData`. These options carry report presentation and explicit
//! host evidence that cannot be established by a filesystem-free composer.

// Rust guideline compliant 2026-02-21

use crate::PreparedReportData;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Presentation options and external evidence for a dataset report.
///
/// Supplying binding or capture evidence does not itself verify that evidence.
/// Composers must match its graph identity before displaying a verified state.
/// Prepared electrical responses must come from the canonical DSP owner;
/// missing responses remain unavailable rather than using a default rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReportDatasetOptions {
    /// Fractional-octave width for response overlays; one octave is the default.
    pub smoothed_octaves: f64,
    /// Explicit canonical payload-binding result supplied by the host.
    pub payload_binding: Option<Value>,
    /// Capture verification evidence associated with the saved payload.
    pub capture_verification: Option<Value>,
    /// Native verification of each comparison's immutable capture-view payload.
    /// Entries bind comparison index, source, seat, digest, and the report graph.
    pub capture_view_bindings: Option<Value>,
    /// Renderer-neutral header provenance, including an explicit generation timestamp.
    pub header_provenance: Option<Value>,
    /// Canonical route crossover responses, keyed by logical input channel.
    pub route_crossover_db: Option<Value>,
    /// Canonical route crossover availability, keyed by logical input channel.
    pub route_crossover_status: Option<Value>,
    /// Prepared electrical EQ responses keyed by channel and stable driver identity.
    pub driver_eq_curves: Option<Value>,
    /// Explicit canonical source evidence for markerless legacy inputs only.
    pub legacy_source_curves: Option<Value>,
}

impl Default for ReportDatasetOptions {
    fn default() -> Self {
        Self {
            smoothed_octaves: 1.0,
            payload_binding: None,
            capture_verification: None,
            capture_view_bindings: None,
            header_provenance: None,
            route_crossover_db: None,
            route_crossover_status: None,
            driver_eq_curves: None,
            legacy_source_curves: None,
        }
    }
}

impl ReportDatasetOptions {
    /// Validate display controls and explicit evidence container shapes.
    ///
    /// # Errors
    /// Returns an error for invalid smoothing or non-object evidence inputs.
    pub fn validate(&self) -> Result<(), String> {
        if !self.smoothed_octaves.is_finite() || self.smoothed_octaves <= 0.0 {
            return Err("report smoothing must be a positive finite octave width".into());
        }
        for (name, evidence) in [
            ("payload_binding", &self.payload_binding),
            ("capture_verification", &self.capture_verification),
            ("capture_view_bindings", &self.capture_view_bindings),
            ("header_provenance", &self.header_provenance),
            ("route_crossover_db", &self.route_crossover_db),
            ("route_crossover_status", &self.route_crossover_status),
            ("driver_eq_curves", &self.driver_eq_curves),
            ("legacy_source_curves", &self.legacy_source_curves),
        ] {
            if evidence.as_ref().is_some_and(|value| !value.is_object()) {
                return Err(format!("report option {name} must be an object"));
            }
        }
        Ok(())
    }
}

/// Associates verifier claims with the exact prepared frozen graph identity.
///
/// A positive result requires a matching graph digest and complete verifier
/// identities; legacy or incomplete claims remain explicitly unverified.
pub(crate) fn binding_evidence(
    prepared: &PreparedReportData,
    options: &ReportDatasetOptions,
) -> Result<Value, String> {
    let Some(binding) = &options.payload_binding else {
        return Ok(serde_json::json!({
            "verified":false,
            "reason":"canonical payload-binding evidence was not supplied",
            "acceptance_evidence_verified":false,
            "acceptance_evidence_reason":"acceptance evidence was not verified"
        }));
    };
    let requested = binding["verified"]
        .as_bool()
        .ok_or("payload_binding requires boolean `verified`")?;
    let reason = binding["reason"]
        .as_str()
        .unwrap_or("payload binding was not verified");
    let acceptance_verified = binding["acceptance_evidence_verified"]
        .as_bool()
        .unwrap_or(false);
    let acceptance_reason = binding["acceptance_evidence_reason"]
        .as_str()
        .unwrap_or("acceptance evidence was not verified");
    if !requested {
        return Ok(binding_result(
            false,
            reason,
            binding,
            acceptance_verified,
            acceptance_reason,
        ));
    }
    let Some(graph_digest) = binding["slim_graph_sha256"]
        .as_str()
        .filter(|value| !value.is_empty())
    else {
        return Ok(binding_result(
            false,
            "verified payload binding has no slim graph digest",
            binding,
            acceptance_verified,
            acceptance_reason,
        ));
    };
    let loaded_digest = prepared
        .provenance
        .as_ref()
        .and_then(|value| value["slim_graph_sha256"].as_str());
    if loaded_digest != Some(graph_digest) {
        return Ok(binding_result(
            false,
            "verified binding does not match the loaded frozen graph identity",
            binding,
            acceptance_verified,
            acceptance_reason,
        ));
    }
    if binding["graph_identity"].as_str().is_none_or(str::is_empty) {
        return Ok(binding_result(
            false,
            "verified payload binding has no graph identity",
            binding,
            acceptance_verified,
            acceptance_reason,
        ));
    }
    let Some(payload_digest) = binding["canonical_payload_sha256"].as_str() else {
        return Ok(binding_result(
            false,
            "verified payload binding has no canonical payload digest",
            binding,
            acceptance_verified,
            acceptance_reason,
        ));
    };
    if payload_digest.len() != 64 || !payload_digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(binding_result(
            false,
            "verified payload digest is not a 64-character SHA-256 value",
            binding,
            acceptance_verified,
            acceptance_reason,
        ));
    }
    Ok(serde_json::json!({
        "verified":true,
        "reason":reason,
        "graph_identity":binding["graph_identity"],
        "slim_graph_sha256":graph_digest,
        "canonical_payload_sha256":payload_digest,
        "acceptance_evidence_verified":acceptance_verified,
        "acceptance_evidence_reason":acceptance_reason,
    }))
}

fn binding_result(
    verified: bool,
    reason: &str,
    binding: &Value,
    acceptance_verified: bool,
    acceptance_reason: &str,
) -> Value {
    serde_json::json!({
        "verified":verified,
        "reason":reason,
        "graph_identity":binding.get("graph_identity").cloned().unwrap_or(Value::Null),
        "slim_graph_sha256":binding.get("slim_graph_sha256").cloned().unwrap_or(Value::Null),
        "canonical_payload_sha256":binding.get("canonical_payload_sha256").cloned().unwrap_or(Value::Null),
        "acceptance_evidence_verified":acceptance_verified,
        "acceptance_evidence_reason":acceptance_reason,
    })
}
