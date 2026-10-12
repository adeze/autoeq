//! Canonical IIR and linear-phase crossover design and complex response.
//!
//! These primitives preserve the existing topology design, aliases, and
//! unknown-family fallback. Serialized plugin validation rejects unknown
//! families before calling them.

use crate::response;
use math_audio_iir_fir::Biquad;

#[doc(inline)]
pub use math_audio_iir_fir::DEFAULT_FIR_CROSSOVER_TAPS;

/// Evaluate the canonical crossover on an explicit frequency grid.
///
/// The caller supplies a valid positive sample rate and crossover frequency.
/// Linear-phase aliases use the canonical FIR design; other families use IIR.
/// Unknown families preserve the topology's legacy LR24 fallback.
pub fn compute_crossover_complex_response(
    type_str: &str,
    freq: f64,
    sample_rate: f64,
    is_lowpass: bool,
    freqs: &ndarray::Array1<f64>,
) -> Vec<num_complex::Complex64> {
    if is_linear_phase_crossover_type(type_str) {
        let coeffs = linear_phase_crossover_coefficients(freq, sample_rate, is_lowpass);
        response::compute_fir_complex_response(&coeffs, freqs, sample_rate)
    } else {
        let filters = create_crossover_filters(type_str, freq, sample_rate, is_lowpass);
        response::compute_peq_complex_response(&filters, freqs, sample_rate)
    }
}

/// Build canonical IIR crossover filters, preserving legacy family aliases.
///
/// The caller supplies valid design frequencies and sample rates. Linear-phase
/// and disabled crossovers return no IIR filters; unknown families use LR24.
pub fn create_crossover_filters(
    type_str: &str,
    freq: f64,
    sample_rate: f64,
    is_lowpass: bool,
) -> Vec<Biquad> {
    use math_audio_iir_fir::{
        peq_butterworth_highpass, peq_butterworth_lowpass, peq_linkwitzriley_highpass,
        peq_linkwitzriley_lowpass,
    };
    if is_linear_phase_crossover_type(type_str) {
        return Vec::new();
    }
    let crossover_type = type_str.parse::<crate::serialized_plugins::CrossoverType>();
    let peq = match crossover_type {
        Ok(crate::serialized_plugins::CrossoverType::LinkwitzRiley2) => {
            if is_lowpass {
                peq_linkwitzriley_lowpass(2, freq, sample_rate)
            } else {
                peq_linkwitzriley_highpass(2, freq, sample_rate)
            }
        }
        Ok(crate::serialized_plugins::CrossoverType::LinkwitzRiley4) => {
            if is_lowpass {
                peq_linkwitzriley_lowpass(4, freq, sample_rate)
            } else {
                peq_linkwitzriley_highpass(4, freq, sample_rate)
            }
        }
        Ok(crate::serialized_plugins::CrossoverType::LinkwitzRiley8) => {
            if is_lowpass {
                peq_linkwitzriley_lowpass(8, freq, sample_rate)
            } else {
                peq_linkwitzriley_highpass(8, freq, sample_rate)
            }
        }
        Ok(crate::serialized_plugins::CrossoverType::Butterworth2) => {
            if is_lowpass {
                peq_butterworth_lowpass(2, freq, sample_rate)
            } else {
                peq_butterworth_highpass(2, freq, sample_rate)
            }
        }
        Ok(crate::serialized_plugins::CrossoverType::Butterworth4) => {
            if is_lowpass {
                peq_butterworth_lowpass(4, freq, sample_rate)
            } else {
                peq_butterworth_highpass(4, freq, sample_rate)
            }
        }
        Ok(
            crate::serialized_plugins::CrossoverType::LinearPhase
            | crate::serialized_plugins::CrossoverType::None,
        ) => Vec::new(),
        Err(_) => {
            log::warn!("Unknown crossover type '{}', defaulting to LR24", type_str);
            if is_lowpass {
                peq_linkwitzriley_lowpass(4, freq, sample_rate)
            } else {
                peq_linkwitzriley_highpass(4, freq, sample_rate)
            }
        }
    };
    peq.into_iter().map(|(_, b)| b).collect()
}

/// Recognize the canonical case-insensitive linear-phase crossover aliases.
pub fn is_linear_phase_crossover_type(type_str: &str) -> bool {
    matches!(
        type_str.to_ascii_lowercase().as_str(),
        "linearphase" | "linear_phase" | "linear-phase" | "linearphasefir" | "fir" | "lpfir"
    )
}

/// Build the canonical FIR crossover at the default crossover tap count.
///
/// The caller supplies a valid positive sample rate and crossover frequency.
pub fn linear_phase_crossover_coefficients(
    freq: f64,
    sample_rate: f64,
    is_lowpass: bool,
) -> Vec<f64> {
    let crossover =
        math_audio_iir_fir::FirCrossover::new(freq, sample_rate, 1, DEFAULT_FIR_CROSSOVER_TAPS);
    if is_lowpass {
        crossover.lowpass_coefficients().to_vec()
    } else {
        crossover.highpass_coefficients()
    }
}
