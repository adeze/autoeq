//! Serializable provenance for one optimizer invocation.
//!
//! This deliberately sits at the setup/result boundary. Objective evaluation
//! remains numeric and does not carry provenance through hot optimization loops.

use super::ObjectiveData;
use crate::OptimParams;
use serde::{Deserialize, Serialize};

/// Bounds for one encoded optimizer parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterBounds {
    pub lower: f64,
    pub upper: f64,
}

/// Execution identity for a reproducible optimizer invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizerExecutionPlatform {
    pub operating_system: String,
    pub architecture: String,
    pub compiler: String,
}

/// Provenance payload for an optimizer run, suitable for a transformation
/// ledger entry owned by the workflow layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptimizationRunDescriptor {
    pub schema: String,
    pub schema_version: u32,
    pub objective: String,
    pub parameter_bounds: Vec<ParameterBounds>,
    pub constraints: Vec<String>,
    pub seed: Option<u64>,
    pub backend: String,
    pub backend_version: String,
    pub stopping_reason: String,
    pub platform: OptimizerExecutionPlatform,
}

impl OptimizationRunDescriptor {
    pub(crate) fn started(
        params: &OptimParams,
        objective_data: &ObjectiveData,
        lower_bounds: &[f64],
        upper_bounds: &[f64],
    ) -> Self {
        debug_assert_eq!(lower_bounds.len(), upper_bounds.len());
        let mut constraints = Vec::new();
        if params.min_spacing_oct > 0.0 {
            constraints.push(format!("minimum_spacing_oct={}", params.min_spacing_oct));
        }
        if objective_data.penalty_w_ceiling > 0.0 {
            constraints.push("ceiling_penalty".into());
        }
        if objective_data.penalty_w_spacing > 0.0 {
            constraints.push("spacing_penalty".into());
        }
        if objective_data.penalty_w_mingain > 0.0 {
            constraints.push("minimum_gain_penalty".into());
        }
        if objective_data.integrality.is_some() {
            constraints.push("integrality".into());
        }
        if let Some((rail, weight)) = objective_data.q_rail_penalty {
            constraints.push(format!(
                "q_rail_penalty_v2:rail={rail}:gain_weight={weight}:exponent=4:families=peak+matched_peak"
            ));
        }
        constraints.extend(super::resolution_guard::guard_identities(objective_data));
        if let Some(multi) = objective_data.multi_objective.as_ref()
            && multi
                .objectives
                .iter()
                .any(|objective| objective.captured_loss_perturbation_bound.is_some())
        {
            constraints.push(format!(
                "{}:lambda={}:weights={:?}:loss_bounds={:?}",
                if multi.strategy == crate::roomeq::MultiMeasurementStrategy::VariancePenalized {
                    "captured_variance_noise_v1"
                } else {
                    "captured_power_average_noise_v1"
                },
                multi.variance_lambda,
                multi.weights,
                multi
                    .objectives
                    .iter()
                    .map(|objective| objective.captured_loss_perturbation_bound)
                    .collect::<Vec<_>>()
            ));
        }
        if objective_data.epa_flatness_regression_weight != 0.0 {
            let epa = objective_data.epa_config.clone().unwrap_or_default();
            constraints.push(format!(
                "epa_spectral_regression_v1:weight={}:erb_mix={}:band_mix={}:bands={:?}:baseline=uncorrected_target_error",
                objective_data.epa_flatness_regression_weight,
                epa.flatness_erb_weight, epa.flatness_band_weight, epa.flatness_band_weights
            ));
        }

        Self {
            schema: "autoeq.optimization-run".into(),
            schema_version: 1,
            objective: format!("{:?}", objective_data.loss_type),
            parameter_bounds: lower_bounds
                .iter()
                .zip(upper_bounds)
                .map(|(&lower, &upper)| ParameterBounds { lower, upper })
                .collect(),
            constraints,
            seed: params.seed,
            backend: params.algo.clone(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            stopping_reason: "not_started".into(),
            platform: OptimizerExecutionPlatform {
                operating_system: std::env::consts::OS.into(),
                architecture: std::env::consts::ARCH.into(),
                compiler: env!("AUTOEQ_RUSTC_VERSION").into(),
            },
        }
    }

    pub(crate) fn finished(&mut self, stopping_reason: impl Into<String>) {
        self.stopping_reason = stopping_reason.into();
    }
}

/// Optimizer output plus the descriptor needed to append a workflow ledger
/// operation without reconstructing run settings from lossy logs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptimizationRunResult {
    pub parameters: Vec<f64>,
    pub objective_value: f64,
    pub descriptor: OptimizationRunDescriptor,
}
