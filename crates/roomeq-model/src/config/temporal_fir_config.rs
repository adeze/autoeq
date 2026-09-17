use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Optional, saved-IR temporal candidate search for one independent FIR output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemporalFirConfig {
    pub channel: String,
    pub evidence: Vec<TemporalIrEvidence>,
    pub frequencies_hz: Vec<f64>,
    pub frequency_weights: Vec<f64>,
    pub window_seconds: f64,
    pub starts_seconds: Vec<f64>,
    pub delays_seconds: Vec<f64>,
    pub strengths: Vec<f64>,
    pub minimum_late_improvement_db: f64,
    pub maximum_early_change_db: f64,
    pub maximum_spectral_change_db: f64,
    /// Optional tighter, signed acceptance limits from the calibrated study.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_relative_tail_improvement_db: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_absolute_late_improvement_db: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_early_change_db: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_early_increase_db: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_positions: Option<usize>,
}

/// Identity and processing declaration stored in a linked measurement sidecar.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemporalIrEvidence {
    pub sidecar_path: PathBuf,
    pub position_id: String,
    pub partition: TemporalPartition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TemporalPartition {
    Training,
    HeldOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemporalIrSidecar {
    pub wav_path: PathBuf,
    pub sha256: String,
    pub anchor_sample: usize,
    pub processing_state: String,
    pub stimulus_reference: String,
    pub capture_rate_hz: u32,
    pub channel: String,
    pub position_id: String,
    pub partition: TemporalPartition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orcmeasurement: Option<TemporalArrayReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemporalArrayReference {
    pub package_path: PathBuf,
    pub array_path: String,
    pub logical_sha256: String,
    pub source_digest: String,
    pub campaign_id: String,
}
