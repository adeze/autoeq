//! Canonical small-signal response of serialized RoomEQ plugin chains.
//!
//! Playback realization and portable reports share stage order, validation,
//! FIR wet/dry mixing, and mixed-band reconstruction. Callers own immutable
//! convolution evidence through the injected provider; this module performs no I/O.

// Rust guideline compliant 2026-02-21
use crate::fir_response::fir_response_at;
use crate::serialized_plugins::PluginConfigWrapper;
use crate::{AutoeqError, Result};
use num_complex::Complex64;
use std::f64::consts::PI;

/// Resolves convolution sidecars without coupling the DSP engine to file I/O.
pub trait ConvolutionIrProvider {
    /// Resolve immutable FIR taps at the declared playback sample rate.
    ///
    /// # Errors
    /// Returns an error when the resource is unavailable or incompatible.
    fn taps(&mut self, ir_file: &str, sample_rate: u32) -> Result<&[f64]>;

    /// Evaluate the complete FIR at one frequency.
    ///
    /// Providers may cache exact responses for a requested frequency grid.
    ///
    /// # Errors
    /// Returns an error for an invalid sample rate or unavailable sidecar.
    fn response(
        &mut self,
        ir_file: &str,
        frequency_hz: f64,
        sample_rate: f64,
    ) -> Result<Complex64> {
        Ok(fir_response_at(
            self.taps(ir_file, checked_sample_rate(sample_rate)?)?,
            frequency_hz,
            sample_rate,
        ))
    }
}

/// Provider for chains that are required not to contain convolution plugins.
#[derive(Debug, Default)]
pub struct NoConvolutionIr;

impl ConvolutionIrProvider for NoConvolutionIr {
    fn taps(&mut self, ir_file: &str, _sample_rate: u32) -> Result<&[f64]> {
        Err(AutoeqError::InvalidConfiguration {
            message: format!(
                "DSP realization requires convolution IR '{ir_file}', but no sidecar provider was supplied"
            ),
        })
    }
}

/// Validate and round the playback sample-rate contract.
///
/// # Errors
/// Returns an error for non-finite, non-positive, or out-of-range rates.
pub fn checked_sample_rate(sample_rate: f64) -> Result<u32> {
    if !sample_rate.is_finite() || sample_rate <= 0.0 || sample_rate > u32::MAX as f64 {
        return Err(AutoeqError::InvalidConfiguration {
            message: format!("invalid DSP realization sample rate {sample_rate}"),
        });
    }
    Ok(sample_rate.round() as u32)
}

/// Evaluate a plugin chain at one frequency without reopening resources.
///
/// `sample_rate_u32` must be the checked playback rate for `sample_rate`.
/// The caller validates the evaluation frequency; playback permits periodic
/// evaluation above Nyquist. The response includes small-signal limiter latency,
/// not nonlinear limiting or overload protection.
///
/// # Errors
/// Returns configuration errors for unsupported/malformed plugins or bands,
/// and propagates convolution-provider errors.
pub fn plugin_chain_response<P: ConvolutionIrProvider>(
    plugins: &[PluginConfigWrapper],
    frequency_hz: f64,
    sample_rate: f64,
    sample_rate_u32: u32,
    convolution: &mut P,
) -> Result<Complex64> {
    let mut response = Complex64::new(1.0, 0.0);
    let mut index = 0usize;
    // A serialized chain can advance at most once per plugin. Keep traversal
    // bounded so malformed control flow fails deterministically instead of
    // hanging DSP validation.
    for _ in 0..plugins.len() {
        if index >= plugins.len() {
            break;
        }
        let plugin = &plugins[index];
        if plugin.plugin_type == "band_split" {
            let merge_offset = plugins[index + 1..]
                .iter()
                .position(|candidate| candidate.plugin_type == "band_merge")
                .ok_or_else(|| AutoeqError::InvalidConfiguration {
                    message: "serialized band_split has no matching band_merge".to_string(),
                })?;
            let merge_index = index + 1 + merge_offset;
            let merge = &plugins[merge_index];
            let bands = merge
                .parameters
                .get("bands")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| malformed("band_merge plugin", "bands", &merge.parameters))?;
            if bands != 2 {
                return Err(AutoeqError::InvalidConfiguration {
                    message: format!(
                        "unsupported band_merge plugin with {bands} bands; canonical realization supports 2"
                    ),
                });
            }
            response *= mixed_band_response(
                plugin,
                &plugins[index + 1..merge_index],
                frequency_hz,
                sample_rate,
                sample_rate_u32,
                convolution,
            )?;
            index = merge_index + 1;
            continue;
        }
        if plugin.plugin_type == "band_merge" {
            return Err(AutoeqError::InvalidConfiguration {
                message: "serialized band_merge has no preceding band_split".to_string(),
            });
        }
        response *= plugin_response(
            plugin,
            frequency_hz,
            sample_rate,
            sample_rate_u32,
            convolution,
        )?;
        index += 1;
    }
    Ok(response)
}

fn plugin_response<P: ConvolutionIrProvider>(
    plugin: &PluginConfigWrapper,
    frequency_hz: f64,
    sample_rate: f64,
    sample_rate_u32: u32,
    convolution: &mut P,
) -> Result<Complex64> {
    match plugin.plugin_type.as_str() {
        "limiter" => {
            // Small-signal response only; electrical acceptance must separately
            // verify this mandatory nonlinear processor at the physical output.
            crate::runtime_limiter::ceiling(plugin)?;
            let samples = crate::runtime_limiter::latency_samples(sample_rate);
            Ok(Complex64::from_polar(
                1.0,
                -2.0 * PI * frequency_hz * samples as f64 / sample_rate,
            ))
        }
        "gain" => {
            let gain_db = required_finite_number(&plugin.parameters, "gain_db", "gain plugin")?;
            let sign = if optional_bool(&plugin.parameters, "invert", false, "gain plugin")? {
                -1.0
            } else {
                1.0
            };
            Ok(Complex64::new(sign * 10.0_f64.powf(gain_db / 20.0), 0.0))
        }
        "convolution" => convolution_response(
            plugin,
            frequency_hz,
            sample_rate,
            sample_rate_u32,
            convolution,
        ),
        "crossover" => {
            let crossover_hz = required_frequency(
                &plugin.parameters,
                "frequency",
                "crossover plugin",
                sample_rate,
            )?;
            let output = required_string(&plugin.parameters, "output", "crossover plugin")?;
            let is_lowpass = match output.to_ascii_lowercase().as_str() {
                "low" | "lowpass" | "lp" => true,
                "high" | "highpass" | "hp" => false,
                "both" => return Ok(Complex64::new(1.0, 0.0)),
                other => {
                    return Err(AutoeqError::InvalidConfiguration {
                        message: format!("unsupported crossover output mode '{other}'"),
                    });
                }
            };
            let crossover_type = required_crossover_type(plugin, "crossover plugin")?;
            Ok(crate::crossover::compute_crossover_complex_response(
                crossover_type,
                crossover_hz,
                sample_rate,
                is_lowpass,
                &ndarray::array![frequency_hz],
            )[0])
        }
        "delay" => {
            let delay_ms = required_finite_number(&plugin.parameters, "delay_ms", "delay plugin")?;
            Ok(Complex64::from_polar(
                1.0,
                -2.0 * PI * frequency_hz * delay_ms / 1_000.0,
            ))
        }
        "eq" => eq_response(plugin, frequency_hz, sample_rate),
        other => Err(AutoeqError::InvalidConfiguration {
            message: format!("unsupported serialized RoomEQ plugin '{other}' in DSP realization"),
        }),
    }
}

fn eq_response(
    plugin: &PluginConfigWrapper,
    frequency_hz: f64,
    sample_rate: f64,
) -> Result<Complex64> {
    let mut response = Complex64::new(1.0, 0.0);
    let filters = plugin
        .parameters
        .get("filters")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| malformed("EQ plugin", "filters", &plugin.parameters))?;
    for filter in filters {
        response *= serialized_filter_response(filter, frequency_hz, sample_rate)?;
    }
    Ok(response)
}

fn serialized_filter_response(
    filter: &serde_json::Value,
    frequency_hz: f64,
    sample_rate: f64,
) -> Result<Complex64> {
    crate::serialized_dsp::serialized_filter_response(filter, frequency_hz, sample_rate)
}

fn mixed_band_response<P: ConvolutionIrProvider>(
    split: &PluginConfigWrapper,
    plugins: &[PluginConfigWrapper],
    frequency_hz: f64,
    sample_rate: f64,
    sample_rate_u32: u32,
    convolution: &mut P,
) -> Result<Complex64> {
    let crossover_hz = required_frequency(
        &split.parameters,
        "frequency",
        "band_split plugin",
        sample_rate,
    )?;
    let crossover_type = required_crossover_type(split, "band_split plugin")?;
    let grid = ndarray::array![frequency_hz];
    let mut low = crate::crossover::compute_crossover_complex_response(
        crossover_type,
        crossover_hz,
        sample_rate,
        true,
        &grid,
    )[0];
    let mut high = crate::crossover::compute_crossover_complex_response(
        crossover_type,
        crossover_hz,
        sample_rate,
        false,
        &grid,
    )[0];

    for plugin in plugins {
        let branch_response = plugin_response(
            plugin,
            frequency_hz,
            sample_rate,
            sample_rate_u32,
            convolution,
        )?;
        if plugin_affects_mixed_band(plugin, true)? {
            low *= branch_response;
        }
        if plugin_affects_mixed_band(plugin, false)? {
            high *= branch_response;
        }
    }
    Ok(low + high)
}

fn plugin_affects_mixed_band(plugin: &PluginConfigWrapper, low_band: bool) -> Result<bool> {
    let Some(channels_value) = plugin.parameters.get("channels") else {
        return Ok(true);
    };
    let channels = channels_value.as_array().ok_or_else(|| {
        malformed(
            &format!("{} plugin", plugin.plugin_type),
            "channels",
            channels_value,
        )
    })?;
    let mut affects = false;
    for value in channels {
        let channel = value
            .as_u64()
            .filter(|channel| *channel <= 3)
            .ok_or_else(|| {
                malformed(
                    &format!("{} plugin", plugin.plugin_type),
                    "channels entry (expected 0..=3)",
                    value,
                )
            })?;
        affects |= if low_band {
            channel == 0 || channel == 1
        } else {
            channel == 2 || channel == 3
        };
    }
    Ok(affects)
}

fn convolution_response<P: ConvolutionIrProvider>(
    plugin: &PluginConfigWrapper,
    frequency_hz: f64,
    sample_rate: f64,
    _sample_rate_u32: u32,
    convolution: &mut P,
) -> Result<Complex64> {
    let ir_file = plugin
        .parameters
        .get("ir_file")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AutoeqError::InvalidConfiguration {
            message: "serialized convolution plugin has no ir_file".to_string(),
        })?;
    let wet = convolution.response(ir_file, frequency_hz, sample_rate)?;
    if ir_file.is_empty() {
        return Err(AutoeqError::InvalidConfiguration {
            message: "serialized convolution plugin has an empty ir_file".to_string(),
        });
    }
    let mix = optional_finite_number(&plugin.parameters, "mix", 1.0, "convolution plugin")?
        .clamp(0.0, 1.0);
    let gain_db = optional_finite_number(&plugin.parameters, "gain_db", 0.0, "convolution plugin")?;
    let gain = 10.0_f64.powf(gain_db / 20.0);
    Ok(Complex64::new(1.0 - mix, 0.0) + wet * (mix * gain))
}

fn malformed(owner: &str, field: &str, value: &serde_json::Value) -> AutoeqError {
    AutoeqError::InvalidConfiguration {
        message: format!("malformed {owner} field '{field}': {value}"),
    }
}

fn required_finite_number(object: &serde_json::Value, field: &str, owner: &str) -> Result<f64> {
    object
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            malformed(
                owner,
                field,
                object.get(field).unwrap_or(&serde_json::Value::Null),
            )
        })
}

fn optional_finite_number(
    object: &serde_json::Value,
    field: &str,
    default: f64,
    owner: &str,
) -> Result<f64> {
    match object.get(field) {
        None => Ok(default),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| malformed(owner, field, value)),
    }
}

fn required_positive_number(object: &serde_json::Value, field: &str, owner: &str) -> Result<f64> {
    let value = required_finite_number(object, field, owner)?;
    if value <= 0.0 {
        return Err(malformed(owner, field, object.get(field).unwrap()));
    }
    Ok(value)
}

fn required_frequency(
    object: &serde_json::Value,
    field: &str,
    owner: &str,
    sample_rate: f64,
) -> Result<f64> {
    let value = required_positive_number(object, field, owner)?;
    if value >= sample_rate / 2.0 {
        return Err(AutoeqError::InvalidConfiguration {
            message: format!(
                "malformed {owner} field '{field}': {value} must be below Nyquist {}",
                sample_rate / 2.0
            ),
        });
    }
    Ok(value)
}

fn required_string<'a>(object: &'a serde_json::Value, field: &str, owner: &str) -> Result<&'a str> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            malformed(
                owner,
                field,
                object.get(field).unwrap_or(&serde_json::Value::Null),
            )
        })
}

fn optional_bool(
    object: &serde_json::Value,
    field: &str,
    default: bool,
    owner: &str,
) -> Result<bool> {
    match object.get(field) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| malformed(owner, field, value)),
    }
}

fn required_crossover_type<'a>(plugin: &'a PluginConfigWrapper, owner: &str) -> Result<&'a str> {
    let crossover_type = required_string(&plugin.parameters, "type", owner)?;
    crossover_type
        .parse::<crate::serialized_plugins::CrossoverType>()
        .map_err(|message| AutoeqError::InvalidConfiguration { message })?;
    Ok(crossover_type)
}
