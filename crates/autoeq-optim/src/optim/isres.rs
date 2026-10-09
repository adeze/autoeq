//! Pure-Rust ISRES backend (no NLopt C-FFI).
//!
//! Wraps [`math_audio_optimisation::isres::isres`] in a [`FilterOptimizer`]
//! impl. Like the COBYLA backend, it installs the four autoeq inequality
//! constraints (ceiling, min-gain, spacing, and crossover monotonicity for
//! `DriversFlat`) natively.
//!
//! Registered as `"autoeq:isres"`. The unprefixed legacy alias `"isres"`
//! resolves to `"nlopt:isres"` first (preserves existing behaviour for
//! roomeq config presets); when the `nlopt` feature is OFF, it falls
//! through to this backend.

use super::backend::{
    AlgorithmType, ConstraintCapabilities, ConstraintInstallation, FilterOptimizer,
    FilterOptimizerOutput, NativeConstraint,
};
use super::constraints_install::{build_crossover_monotonicity_constraint, install_constraints};
use super::optimize::OptimizerBackendCompletion;
use super::params::OptimParams;
use super::run_control::OptimizerBudgetProfile;
use super::{ObjectiveData, OptimProgressCallback, PenaltyMode, compute_fitness_penalties_ref};
use math_audio_optimisation::isres::{
    IsresConfig, IsresConstraint, IsresConstraintFn, IsresReport, isres,
};
use ndarray::Array1;
use std::sync::Arc;

/// Pure-Rust ISRES `FilterOptimizer`.
pub struct AutoeqIsresBackend {
    name: &'static str,
}

impl AutoeqIsresBackend {
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }

    fn optimize_with_completion(
        &self,
        x: &mut [f64],
        lower: &[f64],
        upper: &[f64],
        objective: ObjectiveData,
        params: &OptimParams,
    ) -> IsresOptimizationCompletion {
        if lower.len() != x.len() || upper.len() != x.len() {
            return (
                Err((
                    format!(
                        "bounds dimension mismatch: x={}, lower={}, upper={}",
                        x.len(),
                        lower.len(),
                        upper.len(),
                    ),
                    f64::INFINITY,
                )),
                None,
            );
        }

        let mut objective = objective;
        let installation = install_constraints(self.capabilities(), &mut objective);
        let crossover = build_crossover_monotonicity_constraint(&objective);

        let mut constraints: Vec<IsresConstraint> = Vec::new();
        if let ConstraintInstallation::Native(ncs) = installation {
            for c in ncs {
                constraints.push(native_to_isres(c));
            }
        }
        if let Some(c) = crossover {
            constraints.push(native_to_isres(c));
        }

        let obj = Arc::new(objective);
        let obj_for_call = obj.clone();
        let f = move |x: &Array1<f64>| -> f64 {
            compute_fitness_penalties_ref(x.as_slice().unwrap(), &obj_for_call)
        };

        let bounds: Vec<(f64, f64)> = lower
            .iter()
            .zip(upper.iter())
            .map(|(&lo, &hi)| (lo, hi))
            .collect();
        let x0: Vec<f64> = x
            .iter()
            .zip(bounds.iter())
            .map(|(&xi, (lo, hi))| xi.clamp(*lo, *hi))
            .collect();

        // Population sizing: respect `params.population` when sensible
        // (≥ 2), else use ISRES defaults (μ=30, λ=7μ).
        let mu = params.population.max(2);
        let cfg = IsresConfig {
            bounds,
            x0: Some(Array1::from(x0)),
            mu,
            lambda: 0, // 7μ
            maxeval: params.maxeval.max(mu),
            seed: params.seed,
            ..Default::default()
        };

        match isres(&f, &constraints, cfg) {
            Ok(report) => {
                if report.x.len() == x.len() {
                    x.copy_from_slice(report.x.as_slice().unwrap());
                }
                completed_report_result(report)
            }
            Err(e) => (
                Err((format!("ISRES setup failed: {:?}", e), f64::INFINITY)),
                None,
            ),
        }
    }
}

type IsresOptimizationCompletion = (
    Result<(String, f64), (String, f64)>,
    Option<OptimizerBackendCompletion>,
);

fn completed_report_result(report: IsresReport) -> IsresOptimizationCompletion {
    // The solver reports success on stagnation; only budget exhaustion
    // reports success=false. Carry the solver-side convergence verdict
    // through the typed completion so a converged run is not downgraded to
    // best-effort. Budget stops keep the legacy tuple classification
    // downstream.
    let completion = report
        .success
        .then_some(OptimizerBackendCompletion::Converged);
    let label = if report.success {
        format!("AutoEQ ISRES: {}", report.message)
    } else {
        format!("AutoEQ ISRES: {} (not converged)", report.message)
    };
    (Ok((label, report.fun)), completion)
}

impl FilterOptimizer for AutoeqIsresBackend {
    fn name(&self) -> &'static str {
        self.name
    }

    fn supports_initial_candidate(&self) -> bool {
        true
    }

    fn evaluation_budget_profile(
        &self,
        lower_bounds: &[f64],
        upper_bounds: &[f64],
        params: &OptimParams,
    ) -> Option<OptimizerBudgetProfile> {
        if lower_bounds.len() != upper_bounds.len() {
            return None;
        }
        let mu = params.population.max(2);
        let solver_limit = params.maxeval.max(mu);
        Some(OptimizerBudgetProfile::new(
            params.maxeval,
            Some(solver_limit),
            mu,
            mu,
            Some(mu.saturating_mul(7)),
            Some(mu),
            Some(solver_limit / mu),
        ))
    }

    fn library(&self) -> &'static str {
        "AutoEQ"
    }
    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::Global
    }
    fn capabilities(&self) -> ConstraintCapabilities {
        ConstraintCapabilities {
            nonlinear_ineq: true,
            // Equality constraints not exposed; ISRES could support them
            // (R&Y 2005 §III.D) but autoeq has no use case.
            nonlinear_eq: false,
            linear: true,
            iteration_callback: false,
            fallback_penalty_mode: PenaltyMode::Disabled,
        }
    }
    fn optimize(
        &self,
        x: &mut [f64],
        lower: &[f64],
        upper: &[f64],
        objective: ObjectiveData,
        params: &OptimParams,
        _callback: Option<OptimProgressCallback>,
    ) -> Result<(String, f64), (String, f64)> {
        self.optimize_with_completion(x, lower, upper, objective, params)
            .0
    }

    fn optimize_with_typed_completion(
        &self,
        x: &mut [f64],
        lower: &[f64],
        upper: &[f64],
        objective: ObjectiveData,
        params: &OptimParams,
        _callback: Option<OptimProgressCallback>,
    ) -> (
        FilterOptimizerOutput,
        Option<super::optimize::OptimizerBackendCompletion>,
    ) {
        let (result, completion) =
            self.optimize_with_completion(x, lower, upper, objective, params);
        (
            FilterOptimizerOutput::Completed {
                result,
                pareto_report: None,
            },
            completion,
        )
    }
}

fn native_to_isres(c: NativeConstraint) -> IsresConstraint {
    let f = c.fun;
    let wrapped: IsresConstraintFn = Arc::new(move |x: &Array1<f64>| f(x.as_slice().unwrap()));
    IsresConstraint { fun: wrapped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array1;

    fn report(success: bool, message: &str) -> IsresReport {
        IsresReport {
            x: Array1::from(vec![0.5]),
            fun: 0.25,
            max_violation: 0.0,
            feasible: true,
            success,
            message: message.to_string(),
            nfev: 4210,
            nit: 60,
        }
    }

    #[test]
    fn completed_stagnation_report_carries_typed_convergence() {
        let (result, completion) =
            completed_report_result(report(true, "stagnated for 60 generations"));
        assert!(result.is_ok(), "completed solver report: {result:?}");
        let (status, objective) = result.unwrap();
        assert!(!status.contains("not converged"));
        assert_eq!(objective, 0.25);

        let mut evidence = super::super::optimize::OptimizerRunEvidence::from_backend_result(
            "autoeq:isres",
            Ok((status, objective)),
            &[0.5],
            &[0.0],
            &[1.0],
            20000,
            None,
        );
        evidence.apply_backend_completion(completion.unwrap());
        assert_eq!(
            evidence.termination,
            super::super::optimize::OptimizerTermination::Converged
        );
        assert!(evidence.converged);
        assert!(!evidence.best_effort);
        assert_eq!(
            evidence.confidence,
            super::super::optimize::OptimizerConfidence::High
        );
        assert_eq!(evidence.objective, Some(0.25));
    }

    #[test]
    fn budget_stop_keeps_legacy_evaluation_limit_classification() {
        let (result, completion) =
            completed_report_result(report(false, "Maximum evaluations reached: 20000"));
        assert!(result.is_ok(), "completed solver report: {result:?}");
        assert!(
            completion.is_none(),
            "budget stop carries no typed completion"
        );
        let (status, objective) = result.unwrap();

        let evidence = super::super::optimize::OptimizerRunEvidence::from_backend_result(
            "autoeq:isres",
            Ok((status, objective)),
            &[0.5],
            &[0.0],
            &[1.0],
            20000,
            None,
        );
        assert_eq!(
            evidence.termination,
            super::super::optimize::OptimizerTermination::EvaluationLimit
        );
        assert!(!evidence.converged);
        assert!(evidence.best_effort);
    }
}
