//! Portable response evaluation for serialized RoomEQ plugin chains.
//!
//! This module validates bounded, JSON-transportable requests and delegates
//! every filter calculation to `autoeq_core::serialized_chain`. It performs no
//! file access. A native host may provide FIR responses sampled on the exact
//! request grid; this module never interpolates or resolves resource paths.

// Rust guideline compliant 2026-02-21

use autoeq_core::serialized_chain::{
    ConvolutionIrProvider, checked_sample_rate, plugin_chain_response,
};
use autoeq_core::serialized_plugins::PluginConfigWrapper;
use autoeq_core::{AutoeqError, Result as CoreResult};
use rustfft::num_complex::Complex64;
use serde_json::Value;
use std::collections::BTreeMap;

/// Maximum serialized plugins accepted in one portable operation.
///
/// Larger chains require a report-level batching design instead of a single
/// unbounded WASM request.
const MAX_PLUGIN_COUNT: usize = 512;
/// Maximum weighted plugin-frequency work units per operation.
///
/// EQ filters and linear-phase crossover taps count individually so one large
/// nested plugin cannot bypass the budget by staying under the plugin count.
const MAX_PLUGIN_FREQUENCY_WORK: usize = 10_000_000;
/// Conservative work estimate for one IIR crossover response sample.
const IIR_CROSSOVER_WORK_UNITS: usize = 8;
/// Maximum supplied complex FIR response samples across all resources.
///
/// The request contains two floating-point values per sample, so this bound
/// limits both parsing memory and response lookup storage.
const MAX_FIR_RESPONSE_SAMPLES: usize = 1_000_000;
/// Maximum resource identities in one request, including unused evidence.
const MAX_FIR_RESOURCE_COUNT: usize = 1_024;

/// Evaluate one bounded serialized-plugin-chain request.
///
/// The request must contain `operation: "serialized_plugin_chain_response"`,
/// a positive sample rate, a non-empty finite grid within DC through Nyquist,
/// and canonical `PluginConfigWrapper` values. Frequency order and duplicate
/// values are preserved. Optional `fir_responses` maps exact IR identities to
/// complex responses in the same order as `frequencies`. Missing FIR evidence
/// fails when referenced.
///
/// # Errors
/// Returns an error for malformed or oversized requests, unsupported plugins,
/// invalid FIR evidence, or a non-finite evaluated response.
pub fn response_from_request(request: &Value) -> Result<Vec<[f64; 2]>, String> {
    if request.get("operation").and_then(Value::as_str) != Some("serialized_plugin_chain_response")
    {
        return Err("unsupported serialized chain operation".into());
    }

    let sample_rate = request["sample_rate"]
        .as_f64()
        .ok_or("missing or non-numeric sample_rate")?;
    let sample_rate_u32 = checked_sample_rate(sample_rate).map_err(|error| error.to_string())?;
    if sample_rate_u32 == 0 {
        return Err("sample_rate rounds below the supported integer rate".into());
    }

    let frequencies = parse_frequency_grid(&request["frequencies"], sample_rate)?;
    let plugins_value = request["plugins"]
        .as_array()
        .ok_or("missing serialized plugin array")?;
    if plugins_value.len() > MAX_PLUGIN_COUNT {
        return Err(format!("plugin count exceeds limit {MAX_PLUGIN_COUNT}"));
    }
    let work_units = estimated_plugin_work_units(plugins_value)?;
    let work = work_units
        .checked_mul(frequencies.len())
        .ok_or("plugin-frequency work count overflow")?;
    if work > MAX_PLUGIN_FREQUENCY_WORK {
        return Err(format!(
            "plugin-frequency work exceeds limit {MAX_PLUGIN_FREQUENCY_WORK}"
        ));
    }
    let plugins: Vec<PluginConfigWrapper> =
        serde_json::from_value(Value::Array(plugins_value.clone()))
            .map_err(|error| format!("invalid serialized plugin array: {error}"))?;

    let fir_responses = parse_fir_responses(&request["fir_responses"], &frequencies)?;
    let mut provider = GridResponseProvider {
        frequencies: &frequencies,
        grid_index: 0,
        responses: fir_responses,
    };
    let mut response = Vec::with_capacity(frequencies.len());
    for (grid_index, &frequency_hz) in frequencies.iter().enumerate() {
        provider.grid_index = grid_index;
        let value = plugin_chain_response(
            &plugins,
            frequency_hz,
            sample_rate,
            sample_rate_u32,
            &mut provider,
        )
        .map_err(|error| error.to_string())?;
        if !value.re.is_finite() || !value.im.is_finite() {
            return Err(format!(
                "serialized chain response is non-finite at {frequency_hz} Hz"
            ));
        }
        response.push([value.re, value.im]);
    }
    Ok(response)
}

fn parse_frequency_grid(value: &Value, sample_rate: f64) -> Result<Vec<f64>, String> {
    let values = value.as_array().ok_or("missing frequency grid")?;
    if values.is_empty() {
        return Err("frequency grid must not be empty".into());
    }
    if values.len() > crate::serialized_dsp::MAX_RESPONSE_FREQUENCIES {
        return Err(format!(
            "frequency grid exceeds limit {}",
            crate::serialized_dsp::MAX_RESPONSE_FREQUENCIES
        ));
    }
    let frequencies = values
        .iter()
        .map(|value| value.as_f64().ok_or("frequency must be numeric"))
        .collect::<Result<Vec<_>, _>>()?;
    if frequencies.iter().any(|frequency| {
        !frequency.is_finite() || *frequency < 0.0 || *frequency > sample_rate / 2.0
    }) {
        return Err("frequency grid must be finite and within DC through Nyquist".into());
    }
    Ok(frequencies)
}

fn estimated_plugin_work_units(plugins: &[Value]) -> Result<usize, String> {
    plugins.iter().try_fold(0usize, |total, plugin| {
        let plugin_type = plugin
            .get("plugin_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let cost = match plugin_type {
            "eq" => plugin
                .get("parameters")
                .and_then(|parameters| parameters.get("filters"))
                .and_then(Value::as_array)
                .map_or(1, |filters| filters.len().max(1)),
            "crossover" => crossover_work_units(plugin, 1),
            "band_split" => crossover_work_units(plugin, 2),
            _ => 1,
        };
        total
            .checked_add(cost)
            .ok_or_else(|| "plugin work-unit count overflow".to_string())
    })
}

fn crossover_work_units(plugin: &Value, branch_count: usize) -> usize {
    let crossover_type = plugin
        .get("parameters")
        .and_then(|parameters| parameters.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let per_branch = if autoeq_core::crossover::is_linear_phase_crossover_type(crossover_type) {
        autoeq_core::crossover::DEFAULT_FIR_CROSSOVER_TAPS
    } else {
        IIR_CROSSOVER_WORK_UNITS
    };
    per_branch.saturating_mul(branch_count)
}

fn parse_fir_responses(
    value: &Value,
    frequencies: &[f64],
) -> Result<BTreeMap<String, Vec<Complex64>>, String> {
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    let resources = value.as_object().ok_or("fir_responses must be an object")?;
    if resources.len() > MAX_FIR_RESOURCE_COUNT {
        return Err(format!(
            "FIR resource count exceeds limit {MAX_FIR_RESOURCE_COUNT}"
        ));
    }

    let mut total_samples = 0usize;
    let mut parsed = BTreeMap::new();
    for (ir_file, value) in resources {
        if ir_file.is_empty() {
            return Err("FIR response identity must not be empty".into());
        }
        let samples = value
            .as_array()
            .ok_or_else(|| format!("FIR response for '{ir_file}' must be an array"))?;
        if samples.len() != frequencies.len() {
            return Err(format!(
                "FIR response for '{ir_file}' has {} samples for a {}-point frequency grid",
                samples.len(),
                frequencies.len()
            ));
        }
        total_samples = total_samples
            .checked_add(samples.len())
            .ok_or("FIR response sample count overflow")?;
        if total_samples > MAX_FIR_RESPONSE_SAMPLES {
            return Err(format!(
                "FIR response samples exceed limit {MAX_FIR_RESPONSE_SAMPLES}"
            ));
        }
        let response = samples
            .iter()
            .enumerate()
            .map(|(index, sample)| {
                let pair = sample
                    .as_array()
                    .filter(|pair| pair.len() == 2)
                    .ok_or_else(|| {
                        format!("FIR response sample {index} for '{ir_file}' must be [re, im]")
                    })?;
                let re = pair[0]
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        format!("FIR response real sample {index} for '{ir_file}' is invalid")
                    })?;
                let im = pair[1]
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        format!("FIR response imaginary sample {index} for '{ir_file}' is invalid")
                    })?;
                Ok(Complex64::new(re, im))
            })
            .collect::<Result<Vec<_>, String>>()?;
        parsed.insert(ir_file.clone(), response);
    }
    Ok(parsed)
}

struct GridResponseProvider<'a> {
    frequencies: &'a [f64],
    grid_index: usize,
    responses: BTreeMap<String, Vec<Complex64>>,
}

impl ConvolutionIrProvider for GridResponseProvider<'_> {
    fn taps(&mut self, ir_file: &str, _sample_rate: u32) -> CoreResult<&[f64]> {
        Err(AutoeqError::InvalidConfiguration {
            message: format!("FIR taps are unavailable for portable response resource '{ir_file}'"),
        })
    }

    fn response(
        &mut self,
        ir_file: &str,
        frequency_hz: f64,
        _sample_rate: f64,
    ) -> CoreResult<Complex64> {
        if self.frequencies.get(self.grid_index).copied() != Some(frequency_hz) {
            return Err(AutoeqError::InvalidConfiguration {
                message: format!(
                    "FIR response grid sample {} does not match {frequency_hz} Hz",
                    self.grid_index
                ),
            });
        }
        self.responses
            .get(ir_file)
            .and_then(|response| response.get(self.grid_index))
            .copied()
            .ok_or_else(|| AutoeqError::InvalidConfiguration {
                message: format!("missing FIR response for exact resource '{ir_file}'"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::response_from_request;
    use rustfft::num_complex::Complex64;
    use serde_json::{Value, json};
    use std::f64::consts::TAU;

    fn request(sample_rate: f64, frequencies: &[f64], plugins: Value) -> Value {
        json!({
            "operation": "serialized_plugin_chain_response",
            "sample_rate": sample_rate,
            "frequencies": frequencies,
            "plugins": plugins,
        })
    }

    fn assert_complex(actual: [f64; 2], expected: Complex64) {
        assert!((actual[0] - expected.re).abs() < 1e-10);
        assert!((actual[1] - expected.im).abs() < 1e-10);
    }

    #[test]
    fn evaluates_gain_polarity_and_delay_with_canonical_chain() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            let frequencies = [0.0, sample_rate / 4.0, sample_rate / 2.0];
            let input = request(
                sample_rate,
                &frequencies,
                json!([
                    {"plugin_type":"gain", "parameters":{"gain_db":-6.0,"invert":true}},
                    {"plugin_type":"delay", "parameters":{"delay_ms":0.75}}
                ]),
            );
            let actual = response_from_request(&input).unwrap();
            for (actual, &frequency_hz) in actual.iter().zip(&frequencies) {
                let expected = Complex64::from_polar(
                    -10.0_f64.powf(-6.0 / 20.0),
                    -TAU * frequency_hz * 0.75 / 1_000.0,
                );
                assert_complex(*actual, expected);
            }
        }
    }

    #[test]
    fn applies_exact_grid_fir_response_with_canonical_gain_and_mix() {
        let frequencies = [0.0, 12_000.0, 24_000.0];
        let input = json!({
            "operation":"serialized_plugin_chain_response",
            "sample_rate":48_000.0,
            "frequencies":frequencies,
            "plugins":[{"plugin_type":"convolution", "parameters":{
                "ir_file":"room/ir.wav", "mix":0.4, "gain_db":-6.0
            }}],
            "fir_responses":{"room/ir.wav":[[0.5,0.0],[0.0,0.5],[-0.5,0.0]]}
        });
        let actual = response_from_request(&input).unwrap();
        let wet_gain = 0.4 * 10.0_f64.powf(-6.0 / 20.0);
        let expected_wet = [
            Complex64::new(0.5, 0.0),
            Complex64::new(0.0, 0.5),
            Complex64::new(-0.5, 0.0),
        ];
        for (actual, wet) in actual.iter().zip(expected_wet) {
            assert_complex(*actual, Complex64::new(0.6, 0.0) + wet * wet_gain);
        }
    }

    #[test]
    fn reconstructs_band_split_merge_through_the_shared_evaluator() {
        let frequencies = [0.0];
        let input = request(
            48_000.0,
            &frequencies,
            json!([
                {"plugin_type":"band_split", "parameters":{"frequency":1_000.0,"type":"LR24"}},
                {"plugin_type":"band_merge", "parameters":{"bands":2}}
            ]),
        );
        let actual = response_from_request(&input).unwrap();
        assert_complex(actual[0], Complex64::new(1.0, 0.0));
    }

    #[test]
    fn rejects_invalid_grids_plugins_and_fir_evidence() {
        let valid_grid = [0.0, 24_000.0];
        for invalid_grid in [vec![], vec![0.0, 24_001.0], vec![-1.0, 20.0]] {
            let input = request(48_000.0, &invalid_grid, json!([]));
            assert!(response_from_request(&input).is_err());
        }

        let malformed_plugin = request(
            48_000.0,
            &valid_grid,
            json!([{"plugin_type":"gain", "parameters":{}}]),
        );
        assert!(response_from_request(&malformed_plugin).is_err());

        let missing_fir = request(
            48_000.0,
            &valid_grid,
            json!([{"plugin_type":"convolution", "parameters":{"ir_file":"missing.wav"}}]),
        );
        assert!(response_from_request(&missing_fir).is_err());

        let mismatched_grid = json!({
            "operation":"serialized_plugin_chain_response",
            "sample_rate":48_000.0,
            "frequencies":valid_grid,
            "plugins":[],
            "fir_responses":{"ir.wav":[[1.0,0.0]]}
        });
        assert!(response_from_request(&mismatched_grid).is_err());

        let nonfinite_fir = json!({
            "operation":"serialized_plugin_chain_response",
            "sample_rate":48_000.0,
            "frequencies":valid_grid,
            "plugins":[],
            "fir_responses":{"ir.wav":[[1.0,0.0],["NaN",0.0]]}
        });
        assert!(response_from_request(&nonfinite_fir).is_err());

        let unsupported_merge = request(
            48_000.0,
            &valid_grid,
            json!([{"plugin_type":"band_merge", "parameters":{"bands":2}}]),
        );
        assert!(response_from_request(&unsupported_merge).is_err());

        let invalid_rate = request(0.1, &[0.0], json!([]));
        assert!(response_from_request(&invalid_rate).is_err());
    }

    #[test]
    fn bounds_plugin_count_and_frequency_work() {
        let too_many_plugins = request(
            48_000.0,
            &[0.0],
            Value::Array(
                (0..=super::MAX_PLUGIN_COUNT)
                    .map(|_| json!({"plugin_type":"gain", "parameters":{"gain_db":0.0}}))
                    .collect(),
            ),
        );
        assert!(
            response_from_request(&too_many_plugins)
                .unwrap_err()
                .contains("plugin count")
        );

        let frequencies: Vec<_> = (0..100_000).map(|index| index as f64 * 0.24).collect();
        let plugins = Value::Array(
            (0..101)
                .map(|_| json!({"plugin_type":"gain", "parameters":{"gain_db":0.0}}))
                .collect(),
        );
        let work_limited = request(48_000.0, &frequencies, plugins);
        assert!(
            response_from_request(&work_limited)
                .unwrap_err()
                .contains("plugin-frequency work")
        );
    }

    #[test]
    fn preserves_partial_and_reordered_frequency_grid_by_exact_index() {
        for frequencies in [vec![20.0, 100.0, 1_000.0], vec![1_000.0, 20.0, 1_000.0]] {
            let fir_values = [[0.25, 0.0], [0.5, -0.25], [0.75, 0.125]];
            let input = json!({
                "operation":"serialized_plugin_chain_response",
                "sample_rate":48_000.0,
                "frequencies":frequencies,
                "plugins":[{"plugin_type":"convolution", "parameters":{"ir_file":"exact.wav"}}],
                "fir_responses":{"exact.wav":fir_values}
            });
            let actual = response_from_request(&input).unwrap();
            assert_eq!(actual, fir_values);
        }
    }

    #[test]
    fn weights_nested_eq_filters_and_linear_phase_taps_in_work_limit() {
        let frequencies: Vec<_> = (0..10_000).map(|index| index as f64 * 2.4).collect();
        let filters = vec![json!({"type":"peak"}); 1_100];
        let nested_eq = json!([{
            "plugin_type":"eq",
            "parameters":{"filters":filters}
        }]);
        let input = request(48_000.0, &frequencies, nested_eq);
        assert!(
            response_from_request(&input)
                .unwrap_err()
                .contains("plugin-frequency work")
        );

        let linear_phase = request(
            48_000.0,
            &[0.0, 1_000.0],
            json!([{
                "plugin_type":"crossover",
                "parameters":{"frequency":1_000.0,"type":"LinearPhase","output":"low"}
            }]),
        );
        let measured_cost =
            super::estimated_plugin_work_units(linear_phase["plugins"].as_array().unwrap())
                .unwrap();
        assert_eq!(
            measured_cost,
            autoeq_core::crossover::DEFAULT_FIR_CROSSOVER_TAPS
        );
    }
}
