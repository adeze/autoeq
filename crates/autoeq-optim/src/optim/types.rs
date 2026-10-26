use super::objective_data::ObjectiveData;

/// Data for multi-objective optimization across multiple measurements
#[derive(Debug, Clone)]
pub struct MultiObjectiveData {
    /// One ObjectiveData per measurement curve. For
    /// `MultiMeasurementStrategy::MinimaxUncertainty`, this holds B
    /// bootstrap-resampled objectives generated once at setup time.
    pub objectives: Vec<ObjectiveData>,
    /// Strategy for combining per-measurement losses
    pub strategy: crate::roomeq::MultiMeasurementStrategy,
    /// Normalized weights (len == objectives.len()), used by WeightedSum
    pub weights: Vec<f64>,
    /// Lambda for VariancePenalized strategy
    pub variance_lambda: f64,
    /// CVaR tail fraction in (0, 1], used by `MinimaxUncertainty` when its
    /// scalarisation is `Cvar`. `None` means pure worst-case (max).
    pub uncertainty_cvar_alpha: Option<f64>,
}

/// Read-only prediction on the prepared objective grid; not measured validation.
#[derive(Debug, Clone)]
pub struct OptimizerResponseSnapshot {
    pub frequency_hz: Vec<f64>,
    pub prepared_measurement_db: Vec<f64>,
    pub target_db: Vec<f64>,
    pub filter_db: Vec<f64>,
    pub predicted_db: Vec<f64>,
}

/// Progress callback: (iteration, best_loss, epa_preference, response) -> continue/stop
///
/// Used to thread per-iteration optimizer progress through the room EQ call chain.
/// `epa_preference` is `Some` when EPA is computed (every N iterations), `None` otherwise.
pub type OptimProgressCallback = Box<
    dyn FnMut(
            usize,
            f64,
            Option<f64>,
            Option<std::sync::Arc<OptimizerResponseSnapshot>>,
        ) -> crate::de::CallbackAction
        + Send,
>;
