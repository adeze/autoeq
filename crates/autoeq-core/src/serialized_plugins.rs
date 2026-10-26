//! Backend-neutral plugin and crossover contracts for serialized DSP.

// Rust guideline compliant 2026-02-21
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Backend-neutral serialized plugin descriptor. Native adapters translate this
/// contract into their runtime types; the model does not depend on AudioEngine.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginConfigWrapper {
    pub plugin_type: String,
    pub parameters: serde_json::Value,
}

/// Crossover families accepted by canonical RoomEQ serialization.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CrossoverType {
    Butterworth2,
    #[serde(alias = "BW24", alias = "Butterworth24")]
    Butterworth4,
    LinkwitzRiley2,
    #[default]
    #[serde(alias = "LR24")]
    LinkwitzRiley4,
    #[serde(alias = "LR48")]
    LinkwitzRiley8,
    #[serde(alias = "LinearPhase")]
    LinearPhase,
    None,
}

impl std::str::FromStr for CrossoverType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "butterworth2" | "bw2" | "butterworth12" | "bw12" => Ok(Self::Butterworth2),
            "butterworth4" | "bw4" | "butterworth24" | "bw24" => Ok(Self::Butterworth4),
            "lr2" | "lr12" | "linkwitzriley2" | "linkwitzriley12" => Ok(Self::LinkwitzRiley2),
            "lr4" | "lr24" | "linkwitzriley4" | "linkwitzriley24" => Ok(Self::LinkwitzRiley4),
            "lr8" | "lr48" | "linkwitzriley8" | "linkwitzriley48" => Ok(Self::LinkwitzRiley8),
            "linearphase" | "linear_phase" | "linear-phase" | "linearphasefir" | "fir"
            | "lpfir" => Ok(Self::LinearPhase),
            "none" => Ok(Self::None),
            _ => Err(format!("Unknown crossover type: {value}")),
        }
    }
}
