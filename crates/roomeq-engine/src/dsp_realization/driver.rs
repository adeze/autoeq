//! Per-driver application of a serialized channel DSP chain.

// Rust guideline compliant 2026-02-21

use super::ConvolutionIrProvider;
use super::Result;
use crate::response::{MIN_REALIZATION_RESPONSE_DB, apply_complex_response_with_min_db};
use autoeq_core::serialized_chain::plugin_chain_response;
use num_complex::Complex64;
use roomeq_model::{DriverDspChain, PluginConfigWrapper};

/// Apply one driver's plugins followed by its channel's common plugins.
///
/// The driver's measured curve is authoritative. When `measured_band_hz` is
/// present, only original samples inside that inclusive band are retained;
/// no edge samples are synthesized. An absent band preserves the complete
/// validated curve for legacy data and does not certify that the whole grid
/// was measured. Phase remains absent when the input phase was unavailable.
/// Noise-floor and coherence evidence are retained as supplied and are never
/// inferred from the filtered response.
///
/// # Errors
/// Returns an error if the driver has no measured curve, the curve or band is
/// invalid, the retained band has insufficient measured support, the sample
/// rate is invalid, or any plugin/IR cannot be evaluated.
pub fn apply_driver_dsp_to_curve<P: ConvolutionIrProvider>(
    driver: &DriverDspChain,
    common_plugins: &[PluginConfigWrapper],
    sample_rate: f64,
    convolution: &mut P,
) -> Result<crate::Curve> {
    let source_curve = driver.initial_curve.as_ref().ok_or_else(|| {
        autoeq_core::AutoeqError::InvalidMeasurement {
            message: format!("driver '{}' has no measured initial curve", driver.name),
        }
    })?;
    let source_curve = crate::Curve::from(source_curve.clone());
    source_curve.validate("driver DSP realization curve")?;

    let curve = match driver.measured_band_hz {
        Some(band) => source_curve.select_frequency_band(band)?,
        None => source_curve,
    };
    let sample_rate_u32 = super::checked_sample_rate(sample_rate)?;
    let response = curve
        .freq
        .iter()
        .map(|&frequency_hz| {
            let driver_response = plugin_chain_response(
                &driver.plugins,
                frequency_hz,
                sample_rate,
                sample_rate_u32,
                convolution,
            )?;
            let common_response = plugin_chain_response(
                common_plugins,
                frequency_hz,
                sample_rate,
                sample_rate_u32,
                convolution,
            )?;
            Ok(driver_response * common_response)
        })
        .collect::<Result<Vec<Complex64>>>()?;

    let had_phase = curve.phase.is_some();
    let mut result =
        apply_complex_response_with_min_db(&curve, &response, MIN_REALIZATION_RESPONSE_DB);
    if !had_phase {
        result.phase = None;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoeq_core::serialized_chain::NoConvolutionIr;
    use math_audio_iir_fir::{Biquad, BiquadFilterType};
    use ndarray::Array1;
    use roomeq_model::{CurveData, DriverDspChain, PluginConfigWrapper};

    fn curve(phase: Option<Vec<f64>>) -> CurveData {
        CurveData {
            freq: vec![100.0, 250.0, 500.0, 1_000.0, 2_000.0],
            spl: vec![70.0, 71.0, 72.0, 73.0, 74.0],
            phase,
            ..CurveData::default()
        }
    }

    fn gain(gain_db: f64, invert: bool) -> PluginConfigWrapper {
        PluginConfigWrapper {
            plugin_type: "gain".to_string(),
            parameters: serde_json::json!({"gain_db": gain_db, "invert": invert}),
        }
    }

    fn delay(delay_ms: f64) -> PluginConfigWrapper {
        PluginConfigWrapper {
            plugin_type: "delay".to_string(),
            parameters: serde_json::json!({"delay_ms": delay_ms}),
        }
    }

    fn driver(index: usize, plugins: Vec<PluginConfigWrapper>) -> DriverDspChain {
        DriverDspChain {
            measured_acoustics: None,
            name: format!("driver-{index}"),
            index,
            plugins,
            initial_curve: Some(curve(Some(vec![10.0; 5]))),
            measured_band_hz: None,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn direct_peak_response(filter: &Biquad, frequency_hz: f64, sample_rate: f64) -> Complex64 {
        let (a1, a2, b0, b1, b2) = filter.constants();
        let z_inverse =
            Complex64::from_polar(1.0, -std::f64::consts::TAU * frequency_hz / sample_rate);
        let z_inverse_squared = z_inverse * z_inverse;
        (b0 + b1 * z_inverse + b2 * z_inverse_squared)
            / (1.0 + a1 * z_inverse + a2 * z_inverse_squared)
    }

    #[test]
    fn measured_driver_application_matches_independent_branch_response() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            let low = driver(0, vec![gain(-3.0, false), delay(0.25)]);
            let high = driver(1, vec![gain(-6.0, true), delay(-0.5)]);
            let common_peak = Biquad::new(BiquadFilterType::Peak, 500.0, sample_rate, 1.1, 2.0);
            let common = vec![
                gain(-2.0, false),
                crate::output::create_eq_plugin(std::slice::from_ref(&common_peak)),
            ];
            let mut no_ir = NoConvolutionIr;
            let actual_low =
                apply_driver_dsp_to_curve(&low, &common, sample_rate, &mut no_ir).unwrap();
            let actual_high =
                apply_driver_dsp_to_curve(&high, &common, sample_rate, &mut no_ir).unwrap();

            for (index, &frequency_hz) in
                low.initial_curve.as_ref().unwrap().freq.iter().enumerate()
            {
                let expected_low_response = Complex64::new(10.0_f64.powf(-3.0 / 20.0), 0.0)
                    * Complex64::from_polar(
                        1.0,
                        -std::f64::consts::TAU * frequency_hz * 0.25 / 1_000.0,
                    )
                    * Complex64::new(10.0_f64.powf(-2.0 / 20.0), 0.0)
                    * direct_peak_response(&common_peak, frequency_hz, sample_rate);
                let expected_low = low.initial_curve.as_ref().unwrap().spl[index]
                    + 20.0 * expected_low_response.norm().log10();
                assert_close(actual_low.spl[index], expected_low);
                assert_close(
                    actual_low.phase.as_ref().unwrap()[index],
                    10.0 + expected_low_response.arg().to_degrees(),
                );

                let expected_high_response = Complex64::new(-10.0_f64.powf(-6.0 / 20.0), 0.0)
                    * Complex64::from_polar(
                        1.0,
                        -std::f64::consts::TAU * frequency_hz * -0.5 / 1_000.0,
                    )
                    * Complex64::new(10.0_f64.powf(-2.0 / 20.0), 0.0)
                    * direct_peak_response(&common_peak, frequency_hz, sample_rate);
                let expected_high = high.initial_curve.as_ref().unwrap().spl[index]
                    + 20.0 * expected_high_response.norm().log10();
                assert_close(actual_high.spl[index], expected_high);
                assert_close(
                    actual_high.phase.as_ref().unwrap()[index],
                    10.0 + expected_high_response.arg().to_degrees(),
                );
            }
        }
    }

    #[test]
    fn application_preserves_unknown_phase_and_slices_all_per_bin_evidence() {
        let mut measured = curve(None);
        measured.coherence = Some(vec![0.8, 0.81, 0.82, 0.83, 0.84]);
        measured.noise_floor_db = Some(vec![20.0, 21.0, 22.0, 23.0, 24.0]);
        let mut item = driver(0, vec![gain(0.0, false)]);
        item.initial_curve = Some(measured);
        item.measured_band_hz = Some([250.0, 1_000.0]);

        let mut no_ir = NoConvolutionIr;
        let result = apply_driver_dsp_to_curve(&item, &[], 48_000.0, &mut no_ir).unwrap();
        assert_eq!(result.freq, Array1::from_vec(vec![250.0, 500.0, 1_000.0]));
        assert_eq!(result.spl, Array1::from_vec(vec![71.0, 72.0, 73.0]));
        assert!(result.phase.is_none());
        assert_eq!(
            result.coherence,
            Some(Array1::from_vec(vec![0.81, 0.82, 0.83]))
        );
        assert_eq!(
            result.noise_floor_db,
            Some(Array1::from_vec(vec![21.0, 22.0, 23.0]))
        );
    }

    #[test]
    fn application_rejects_missing_curve_and_malformed_or_empty_band() {
        let mut no_ir = NoConvolutionIr;
        let mut missing = driver(0, vec![]);
        missing.initial_curve = None;
        assert!(apply_driver_dsp_to_curve(&missing, &[], 48_000.0, &mut no_ir).is_err());

        let mut malformed = driver(0, vec![]);
        malformed.measured_band_hz = Some([1_000.0, 500.0]);
        assert!(apply_driver_dsp_to_curve(&malformed, &[], 48_000.0, &mut no_ir).is_err());

        let mut unsupported = driver(0, vec![]);
        unsupported.measured_band_hz = Some([3_000.0, 4_000.0]);
        assert!(apply_driver_dsp_to_curve(&unsupported, &[], 48_000.0, &mut no_ir).is_err());
    }
}
