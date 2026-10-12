//! Builds display-only target overlays from loaded target and response data.
//!
//! File lookup and CSV parsing stay at the report boundary. This module receives
//! the loaded target shape and performs the deterministic curve alignment math.
//!
//! # Example
//!
//! ```
//! use autoeq_report_wasm::target_overlay::build_target_overlay_curves;
//! use serde_json::json;
//! let data = json!({"channels": {}, "metadata": {}});
//! let references = json!({"L": {"freq": [100.0], "spl": [70.0]}});
//! let overlays = build_target_overlay_curves(&data, &references, None, None);
//! assert!(overlays.as_object().unwrap().is_empty());
//! ```
//!
//! # Errors
//!
//! Invalid or incomplete curves are omitted, as in the Python report builder.
//! Unsupported crossover kinds are left unchanged only when a route lacks a
//! usable cutoff; valid routed crossovers support the documented RoomEQ types.
//!
//! # Panics
//!
//! This module does not intentionally panic for caller-provided JSON values.

use serde_json::{Map, Value, json};

const DEFAULT_MIN_HZ: f64 = 20.0;
const DEFAULT_MAX_HZ: f64 = 20_000.0;

fn numbers(value: &Value) -> Option<Vec<f64>> {
    value.as_array()?.iter().map(Value::as_f64).collect()
}

fn curve_arrays(curve: &Value) -> Option<(Vec<f64>, Vec<f64>)> {
    let frequencies = numbers(curve.get("freq")?)?;
    let levels = numbers(curve.get("spl")?)?;
    (frequencies.len() == levels.len()).then_some((frequencies, levels))
}

fn frequencies(curve: &Value) -> Option<Vec<f64>> {
    numbers(curve.get("freq")?)
}

fn configured_number(object: &Value, key: &str, default: f64) -> f64 {
    match object.get(key) {
        None => default,
        Some(value) => value.as_f64().unwrap_or(f64::NAN),
    }
}

/// Interpolates a target SPL curve over a requested frequency grid in log-frequency.
///
/// Frequencies below or above the source grid use the nearest endpoint, matching
/// the legacy report behavior. Returns `None` for empty, inconsistent, or invalid
/// source arrays.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::interpolate_log_space;
/// use serde_json::json;
/// let source = json!({"freq":[10.0, 1000.0], "spl":[0.0, 20.0]});
/// let result = interpolate_log_space(&source, &[100.0]).unwrap();
/// assert!((result[0] - 10.0).abs() < 1e-12);
/// ```
///
/// # Errors
///
/// Invalid input returns `None`.
///
/// # Panics
///
/// This function does not panic for caller-provided values.
pub fn interpolate_log_space(target: &Value, frequencies: &[f64]) -> Option<Vec<f64>> {
    let (source_freq, source_spl) = curve_arrays(target)?;
    if source_freq.is_empty()
        || source_freq.iter().any(|f| !f.is_finite() || *f <= 0.0)
        || source_spl.iter().any(|s| !s.is_finite())
        || source_freq.windows(2).any(|pair| pair[0] >= pair[1])
        || frequencies.iter().any(|f| !f.is_finite() || *f <= 0.0)
    {
        return None;
    }
    let source_log: Vec<f64> = source_freq
        .iter()
        .map(|frequency| frequency.log10())
        .collect();
    frequencies
        .iter()
        .map(|frequency| {
            if *frequency <= source_freq[0] {
                return Some(source_spl[0]);
            }
            let last = source_freq.len() - 1;
            if *frequency >= source_freq[last] {
                return Some(source_spl[last]);
            }
            let upper = source_freq.partition_point(|source| source <= frequency);
            let lower = upper - 1;
            let position =
                (frequency.log10() - source_log[lower]) / (source_log[upper] - source_log[lower]);
            let level = source_spl[lower] + position * (source_spl[upper] - source_spl[lower]);
            level.is_finite().then_some(level)
        })
        .collect()
}

/// Returns the arithmetic mean of finite SPL samples in an inclusive band.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::band_mean_spl;
/// use serde_json::json;
/// let curve = json!({"freq":[50.0, 100.0, 200.0], "spl":[10.0, 20.0, 40.0]});
/// assert_eq!(band_mean_spl(&curve, 100.0, 200.0), Some(30.0));
/// ```
///
/// # Errors
///
/// Invalid arrays or an empty band return `None`.
///
/// # Panics
///
/// This function does not panic for caller-provided values.
pub fn band_mean_spl(curve: &Value, min_hz: f64, max_hz: f64) -> Option<f64> {
    let (frequencies, levels) = curve_arrays(curve)?;
    let values: Vec<f64> = frequencies
        .into_iter()
        .zip(levels)
        .filter(|(frequency, level)| {
            min_hz <= *frequency && *frequency <= max_hz && level.is_finite()
        })
        .map(|(_, level)| level)
        .collect();
    if values.is_empty() {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    mean.is_finite().then_some(mean)
}

/// Computes the single display offset shared by an L/R target overlay.
///
/// The mean is averaged across the two reference channels, then compared with
/// the target shape interpolated on the left grid (or right grid if left is empty).
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::global_target_offset_for_pair;
/// use serde_json::json;
/// let curve = json!({"freq":[100.0], "spl":[70.0]});
/// let shape = json!({"freq":[100.0], "spl":[80.0]});
/// assert_eq!(global_target_offset_for_pair(Some(&curve), Some(&curve), Some(&shape), 100.0, 100.0), Some(-10.0));
/// ```
///
/// # Errors
///
/// Missing channels, shape, or in-band samples return `None`.
///
/// # Panics
///
/// This function does not panic for caller-provided values.
pub fn global_target_offset_for_pair(
    reference_l: Option<&Value>,
    reference_r: Option<&Value>,
    shape: Option<&Value>,
    min_hz: f64,
    max_hz: f64,
) -> Option<f64> {
    let (reference_l, reference_r, shape) = (reference_l?, reference_r?, shape?);
    let (grid_l, _) = curve_arrays(reference_l)?;
    let (grid_r, _) = curve_arrays(reference_r)?;
    let mean_l = band_mean_spl(reference_l, min_hz, max_hz)?;
    let mean_r = band_mean_spl(reference_r, min_hz, max_hz)?;
    let grid = if grid_l.is_empty() { grid_r } else { grid_l };
    if grid.is_empty() {
        return None;
    }
    let levels = interpolate_log_space(shape, &grid)?;
    let shape = json!({"freq": grid, "spl": levels});
    let shape_mean = band_mean_spl(&shape, min_hz, max_hz)?;
    let offset = (mean_l + mean_r) / 2.0 - shape_mean;
    offset.is_finite().then_some(offset)
}

/// Interpolates and aligns a relative target to the reference's passband.
///
/// Samples more than 30 dB below the in-band peak do not influence the offset,
/// which prevents routed low-pass stopbands from pulling the target down.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::align_target_to_curve;
/// use serde_json::json;
/// let target = json!({"freq":[20.0, 20000.0], "spl":[0.0, -10.0]});
/// let reference = json!({"freq":[20.0, 20000.0], "spl":[80.0, 70.0]});
/// assert!(align_target_to_curve(&target, &reference, 20.0, 20000.0).is_some());
/// ```
///
/// # Errors
///
/// Invalid curves or no eligible reference samples return `None`.
///
/// # Panics
///
/// This function does not panic for caller-provided values.
pub fn align_target_to_curve(
    target: &Value,
    reference: &Value,
    min_hz: f64,
    max_hz: f64,
) -> Option<Value> {
    let (frequencies, reference_levels) = curve_arrays(reference)?;
    if frequencies.is_empty() {
        return None;
    }
    let target_levels = interpolate_log_space(target, &frequencies)?;
    let in_band: Vec<f64> = frequencies
        .iter()
        .zip(&reference_levels)
        .filter(|(frequency, level)| {
            min_hz <= **frequency && **frequency <= max_hz && level.is_finite()
        })
        .map(|(_, level)| *level)
        .collect();
    let peak = in_band.iter().copied().max_by(f64::total_cmp)?;
    let floor = peak - 30.0;
    let offsets: Vec<f64> = frequencies
        .iter()
        .zip(&reference_levels)
        .zip(&target_levels)
        .filter(|((frequency, measured), _)| {
            min_hz <= **frequency
                && **frequency <= max_hz
                && measured.is_finite()
                && measured.is_finite()
                && **measured >= floor
        })
        .map(|((_, measured), desired)| *measured - *desired)
        .collect();
    if offsets.is_empty() {
        return None;
    }
    let offset = offsets.iter().sum::<f64>() / offsets.len() as f64;
    if !offset.is_finite() {
        return None;
    }
    let levels: Vec<f64> = target_levels
        .into_iter()
        .map(|level| level + offset)
        .collect();
    levels
        .iter()
        .all(|level| level.is_finite())
        .then(|| json!({"freq":frequencies,"spl":levels}))
}

/// Interpolates and shifts a target so its in-band mean matches the reference.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::shift_target_to_reference_band_mean;
/// use serde_json::json;
/// let target = json!({"freq":[100.0],"spl":[0.0]});
/// let reference = json!({"freq":[100.0],"spl":[70.0]});
/// assert_eq!(shift_target_to_reference_band_mean(&target, &reference, 100.0, 100.0).unwrap()["spl"][0], 70.0);
/// ```
///
/// # Errors
///
/// Invalid curves or empty matching bands return `None`.
///
/// # Panics
///
/// This function does not intentionally panic for caller-provided values.
pub fn shift_target_to_reference_band_mean(
    target: &Value,
    reference: &Value,
    min_hz: f64,
    max_hz: f64,
) -> Option<Value> {
    let (frequencies, reference_levels) = curve_arrays(reference)?;
    if frequencies.is_empty() {
        return None;
    }
    let target_levels = interpolate_log_space(target, &frequencies)?;
    let reference_mean = band_mean_spl(
        &json!({"freq":frequencies,"spl":reference_levels}),
        min_hz,
        max_hz,
    )?;
    let target_mean = band_mean_spl(
        &json!({"freq":frequencies,"spl":target_levels}),
        min_hz,
        max_hz,
    )?;
    let offset = reference_mean - target_mean;
    if !offset.is_finite() {
        return None;
    }
    let target_levels: Vec<f64> = target_levels
        .into_iter()
        .map(|level| level + offset)
        .collect();
    target_levels
        .iter()
        .all(|level| level.is_finite())
        .then(|| json!({"freq":frequencies,"spl":target_levels}))
}

/// Finds the global offset for the serialized L/R target shape.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::global_target_offset;
/// use serde_json::json;
/// let data = json!({"channels":{"L":{"target_curve":{"freq":[100.0],"spl":[80.0]}}}});
/// let refs = json!({"L":{"freq":[100.0],"spl":[70.0]},"R":{"freq":[100.0],"spl":[90.0]}});
/// assert_eq!(global_target_offset(&data, &refs), Some(0.0));
/// ```
///
/// # Errors
///
/// Missing shape, pair, or band samples return `None`.
///
/// # Panics
///
/// This function does not intentionally panic for caller-provided values.
pub fn global_target_offset(data: &Value, reference_curves: &Value) -> Option<f64> {
    let channels = data.get("channels")?;
    let shape = channels
        .get("L")
        .and_then(|channel| channel.get("target_curve"))
        .filter(|shape| {
            shape
                .get("freq")
                .and_then(Value::as_array)
                .is_some_and(|values| !values.is_empty())
        })
        .or_else(|| {
            channels
                .get("R")
                .and_then(|channel| channel.get("target_curve"))
                .filter(|shape| {
                    shape
                        .get("freq")
                        .and_then(Value::as_array)
                        .is_some_and(|values| !values.is_empty())
                })
        });
    global_target_offset_for_pair(
        reference_curves.get("L"),
        reference_curves.get("R"),
        shape,
        100.0,
        10_000.0,
    )
}

fn route_for<'a>(data: &'a Value, channel: &str, kind: &str) -> Option<&'a Value> {
    data.get("metadata")?
        .get("bass_management")?
        .get("routing_graph")?
        .get("routes")?
        .as_array()?
        .iter()
        .find(|route| {
            route.get("source_channel").and_then(Value::as_str) == Some(channel)
                && route.get("route_kind").and_then(Value::as_str) == Some(kind)
        })
}

/// Reports whether the saved topology owns low-pass shaping for a source.
pub(crate) fn has_lfe_lowpass_route(data: &Value, channel: &str) -> bool {
    route_for(data, channel, "lfe_lowpass_to_sub").is_some()
}

/// Applies a caller-supplied low-pass response to an LFE target when routed.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::target_shape_for_channel;
/// use serde_json::json;
/// let target = json!({"freq":[20.0],"spl":[0.0]});
/// let reference = json!({"freq":[100.0],"spl":[70.0]});
/// assert!(target_shape_for_channel(&json!({}), "LFE", &target, &reference, None).is_some());
/// ```
///
/// # Errors
///
/// Invalid target or response arrays return `None`.
///
/// # Panics
///
/// This function does not intentionally panic for caller-provided values.
pub fn target_shape_for_channel(
    data: &Value,
    channel: &str,
    target: &Value,
    reference: &Value,
    route_crossover_db: Option<&Value>,
) -> Option<Value> {
    let Some(route) = route_for(data, channel, "lfe_lowpass_to_sub") else {
        return Some(target.clone());
    };
    let Some(cutoff) = route.get("low_pass_hz").and_then(Value::as_f64) else {
        return Some(target.clone());
    };
    let frequencies = frequencies(reference)?;
    if !cutoff.is_finite() || cutoff <= 0.0 || frequencies.is_empty() {
        return Some(target.clone());
    }
    let response_db = route_crossover_db?.get(channel).and_then(Value::as_array)?;
    if response_db.len() != frequencies.len() {
        return None;
    }
    let levels = interpolate_log_space(target, &frequencies)?;
    let filtered: Option<Vec<f64>> = levels
        .into_iter()
        .zip(response_db)
        .map(|(level, response)| {
            let response = response.as_f64()?;
            let result = level + response;
            result.is_finite().then_some(result)
        })
        .collect();
    Some(json!({"freq":frequencies,"spl":filtered?}))
}

/// Returns the routed optimizer alignment band, falling back to input bounds.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::target_alignment_band;
/// use serde_json::json;
/// assert_eq!(target_alignment_band(&json!({}), "L", 20.0, 20000.0), (20.0, 20000.0));
/// ```
///
/// # Errors
///
/// Unusable route data returns the input bounds.
///
/// # Panics
///
/// This function does not intentionally panic for caller-provided values.
pub fn target_alignment_band(data: &Value, channel: &str, min_hz: f64, max_hz: f64) -> (f64, f64) {
    let Some(route) = route_for(data, channel, "redirected_bass_lowpass_to_sub") else {
        return (min_hz, max_hz);
    };
    let Some(crossover) = route.get("low_pass_hz").and_then(Value::as_f64) else {
        return (min_hz, max_hz);
    };
    let low = min_hz.max(crossover * 2.0);
    let high = max_hz.min(crossover * 8.0).min(2_000.0);
    if low >= high {
        (min_hz, max_hz)
    } else {
        (low, high)
    }
}

/// Builds all display target overlays using already-loaded target and response data.
///
/// The returned object maps each reference channel to its aligned `{freq, spl}`
/// curve. `loaded_target` should be `None` when file lookup or parsing failed.
/// `route_crossover_db` maps routed channels to their authoritative low-pass
/// response in dB on each channel's reference grid. This temporary boundary
/// keeps crossover coefficients owned by the existing DSP implementation.
///
/// # Example
///
/// ```
/// use autoeq_report_wasm::target_overlay::build_target_overlay_curves;
/// use serde_json::json;
/// let result = build_target_overlay_curves(&json!({}), &json!({}), None, None);
/// assert!(result.as_object().unwrap().is_empty());
/// ```
///
/// # Errors
///
/// Invalid individual channel curves are skipped; valid channels remain present.
///
/// # Panics
///
/// This function does not intentionally panic for caller-provided values.
pub fn build_target_overlay_curves(
    data: &Value,
    reference_curves: &Value,
    loaded_target: Option<&Value>,
    route_crossover_db: Option<&Value>,
) -> Value {
    build_target_overlay_curves_with_status(
        data,
        reference_curves,
        loaded_target,
        route_crossover_db,
        None,
    )
}

/// Builds target overlays while requiring explicit availability for routed shaping.
pub fn build_target_overlay_curves_with_status(
    data: &Value,
    reference_curves: &Value,
    loaded_target: Option<&Value>,
    route_crossover_db: Option<&Value>,
    route_crossover_status: Option<&Value>,
) -> Value {
    let metadata = data.get("metadata").unwrap_or(&Value::Null);
    let config = metadata.get("effective_config").unwrap_or(&Value::Null);
    let optimizer = config.get("optimizer").unwrap_or(&Value::Null);
    let min_hz = configured_number(optimizer, "min_freq", DEFAULT_MIN_HZ);
    let max_hz = configured_number(optimizer, "max_freq", DEFAULT_MAX_HZ);
    let channels = data.get("channels").unwrap_or(&Value::Null);
    let shape = channels
        .get("L")
        .and_then(|channel| channel.get("target_curve"))
        .filter(|shape| {
            shape
                .get("freq")
                .and_then(Value::as_array)
                .is_some_and(|array| !array.is_empty())
        })
        .or_else(|| {
            channels
                .get("R")
                .and_then(|channel| channel.get("target_curve"))
                .filter(|shape| {
                    shape
                        .get("freq")
                        .and_then(Value::as_array)
                        .is_some_and(|array| !array.is_empty())
                })
        });
    let reference_l = reference_curves.get("L");
    let reference_r = reference_curves.get("R");
    let global_offset =
        global_target_offset_for_pair(reference_l, reference_r, shape, 100.0, 10_000.0);
    let mut result = Map::new();
    let Some(references) = reference_curves.as_object() else {
        return Value::Object(result);
    };
    for (channel, reference) in references {
        let absolute_target = channels
            .get(channel)
            .and_then(|value| value.get("target_curve"));
        if let Some(target) = absolute_target.filter(|target| {
            target
                .get("freq")
                .and_then(Value::as_array)
                .is_some_and(|values| !values.is_empty())
                && target
                    .get("spl")
                    .and_then(Value::as_array)
                    .is_some_and(|values| !values.is_empty())
        }) {
            let Some(frequencies) = frequencies(reference) else {
                continue;
            };
            if frequencies.is_empty() {
                continue;
            }
            let Some(mut levels) = interpolate_log_space(target, &frequencies) else {
                continue;
            };
            if let Some(offset) = global_offset {
                levels.iter_mut().for_each(|level| *level += offset);
            }
            if levels.iter().all(|level| level.is_finite()) {
                result.insert(channel.clone(), json!({"freq":frequencies,"spl":levels}));
            }
            continue;
        }
        let Some(target) = loaded_target else {
            continue;
        };
        if route_target_unavailable_reason(
            data,
            channel,
            target,
            route_crossover_db,
            route_crossover_status,
            Some(reference),
        )
        .is_some()
        {
            continue;
        }
        let Some(channel_target) =
            target_shape_for_channel(data, channel, target, reference, route_crossover_db)
        else {
            continue;
        };
        let (alignment_min, alignment_max) = target_alignment_band(data, channel, min_hz, max_hz);
        if let Some(aligned) =
            align_target_to_curve(&channel_target, reference, alignment_min, alignment_max)
        {
            result.insert(channel.clone(), aligned);
        }
    }
    Value::Object(result)
}

/// Returns why a loaded target cannot be shaped for one routed source.
pub(crate) fn route_target_unavailable_reason(
    data: &Value,
    channel: &str,
    loaded_target: &Value,
    route_crossover_db: Option<&Value>,
    route_crossover_status: Option<&Value>,
    reference: Option<&Value>,
) -> Option<String> {
    if !loaded_target.is_object() {
        return None;
    }
    let absolute = data
        .get("channels")
        .and_then(|channels| channels.get(channel))
        .and_then(|value| value.get("target_curve"))
        .filter(|target| {
            target["freq"]
                .as_array()
                .is_some_and(|values| !values.is_empty())
                && target["spl"]
                    .as_array()
                    .is_some_and(|values| !values.is_empty())
        });
    if absolute.is_some() || route_for(data, channel, "lfe_lowpass_to_sub").is_none() {
        return None;
    }
    let statuses = route_crossover_status?;
    let status = statuses.get(channel);
    if status.is_some_and(|value| value["status"] == "available") {
        let response = route_crossover_db
            .and_then(|responses| responses.get(channel))
            .and_then(Value::as_array);
        let expected_len = reference.and_then(frequencies).map(|values| values.len());
        if expected_len.is_none() {
            return Some("routed reference response grid is unavailable".to_owned());
        }
        let response_is_valid = response.is_some_and(|values| {
            Some(values.len()) == expected_len
                && !values.is_empty()
                && values
                    .iter()
                    .all(|value| value.as_f64().is_some_and(f64::is_finite))
        });
        return (!response_is_valid).then(|| {
            "canonical routed crossover response is missing, invalid, or off-grid".to_owned()
        });
    }
    Some(
        status
            .and_then(|value| value["reason"].as_str())
            .unwrap_or("canonical routed crossover response is unavailable")
            .to_owned(),
    )
}

/// Lists unavailable loaded-target shaping results for saved LFE routes.
pub(crate) fn unavailable_route_target_statuses(
    data: &Value,
    loaded_target: Option<&Value>,
    route_crossover_db: Option<&Value>,
    route_crossover_status: Option<&Value>,
    reference_curves: Option<&Value>,
) -> Vec<Value> {
    let Some(target) = loaded_target else {
        return Vec::new();
    };
    let Some(routes) = data["metadata"]["bass_management"]["routing_graph"]["routes"].as_array()
    else {
        return Vec::new();
    };
    let mut sources = std::collections::BTreeSet::new();
    let mut results = Vec::new();
    for route in routes {
        if route["route_kind"] != "lfe_lowpass_to_sub" {
            continue;
        }
        let Some(channel) = route["source_channel"].as_str() else {
            continue;
        };
        if !sources.insert(channel) {
            continue;
        }
        if let Some(reason) = route_target_unavailable_reason(
            data,
            channel,
            target,
            route_crossover_db,
            route_crossover_status,
            reference_curves.and_then(|curves| curves.get(channel)),
        ) {
            results.push(json!({
                "section":"target_overlay",
                "channel":channel,
                "status":"unavailable",
                "reason":reason
            }));
        }
    }
    results
}

/// Executes a small named target-overlay helper request from native adapters.
pub(crate) fn helper_response(request: &Value) -> Result<Value, String> {
    let helper = request
        .get("helper")
        .and_then(Value::as_str)
        .ok_or("target overlay helper request needs a string `helper`")?;
    let number = |key: &str, default: f64| match request.get(key) {
        None => default,
        Some(value) => value.as_f64().unwrap_or(f64::NAN),
    };
    let result = match helper {
        "interpolate_log_space" => numbers(&request["frequencies"])
            .and_then(|frequencies| interpolate_log_space(&request["target"], &frequencies))
            .map_or(Value::Null, |values| json!(values)),
        "band_mean_spl" => band_mean_spl(
            &request["curve"],
            number("min_hz", f64::NAN),
            number("max_hz", f64::NAN),
        )
        .map_or(Value::Null, |value| json!(value)),
        "shift_target_to_reference_band_mean" => shift_target_to_reference_band_mean(
            &request["target"],
            &request["reference"],
            number("min_hz", 100.0),
            number("max_hz", 10_000.0),
        )
        .unwrap_or(Value::Null),
        "global_target_offset_for_pair" => global_target_offset_for_pair(
            request.get("reference_l").filter(|value| !value.is_null()),
            request.get("reference_r").filter(|value| !value.is_null()),
            request.get("shape").filter(|value| !value.is_null()),
            number("min_hz", 100.0),
            number("max_hz", 10_000.0),
        )
        .map_or(Value::Null, |value| json!(value)),
        "align_target_to_curve" => align_target_to_curve(
            &request["target"],
            &request["reference"],
            number("min_hz", DEFAULT_MIN_HZ),
            number("max_hz", DEFAULT_MAX_HZ),
        )
        .unwrap_or(Value::Null),
        "global_target_offset" => {
            global_target_offset(&request["data"], &request["reference_curves"])
                .map_or(Value::Null, |value| json!(value))
        }
        "target_shape_for_channel" => target_shape_for_channel(
            &request["data"],
            request["channel"].as_str().unwrap_or_default(),
            &request["target"],
            &request["reference"],
            request
                .get("route_crossover_db")
                .filter(|value| !value.is_null()),
        )
        .unwrap_or(Value::Null),
        "target_alignment_band_for_channel" => {
            let (low, high) = target_alignment_band(
                &request["data"],
                request["channel"].as_str().unwrap_or_default(),
                number("min_hz", DEFAULT_MIN_HZ),
                number("max_hz", DEFAULT_MAX_HZ),
            );
            json!([low, high])
        }
        _ => return Err(format!("unsupported target overlay helper `{helper}`")),
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_is_logarithmic_and_clamps_endpoints() {
        let target = json!({"freq":[10.0,1000.0],"spl":[0.0,20.0]});
        assert_eq!(
            interpolate_log_space(&target, &[1.0, 100.0, 2000.0]),
            Some(vec![0.0, 10.0, 20.0])
        );
    }

    #[test]
    fn alignment_ignores_more_than_thirty_decibels_below_peak() {
        let target = json!({"freq":[20.0,20000.0],"spl":[0.0,-10.0]});
        let reference = json!({"freq":[20.0,40.0,80.0,160.0,1000.0,20000.0],"spl":[80.0,79.0,77.0,60.0,-40.0,-120.0]});
        let aligned = align_target_to_curve(&target, &reference, 20.0, 20000.0).unwrap();
        assert!(aligned["spl"][0].as_f64().unwrap() > 75.0);
    }

    #[test]
    fn helper_preserves_malformed_explicit_band_bounds_as_unavailable() {
        let request = json!({
            "helper": "band_mean_spl",
            "curve": {"freq": [100.0], "spl": [70.0]},
            "min_hz": null,
            "max_hz": 1000.0
        });
        assert_eq!(helper_response(&request).unwrap(), Value::Null);
    }

    #[test]
    fn unavailable_lfe_crossover_suppresses_loaded_target_but_baseline_target_remains() {
        let graph = json!({
            "channels": {},
            "metadata": {"bass_management": {"routing_graph": {"routes": [{
                "source_channel":"LFE", "route_kind":"lfe_lowpass_to_sub", "low_pass_hz":80.0
            }]}}}
        });
        let references = json!({"LFE":{"freq":[40.0,80.0,160.0],"spl":[70.0,70.0,70.0]}});
        let target = json!({"freq":[20.0,20000.0],"spl":[75.0,75.0]});
        let unavailable = json!({"LFE":{"status":"unavailable","reason":"playback rate missing"}});
        let routed = build_target_overlay_curves_with_status(
            &graph,
            &references,
            Some(&target),
            Some(&json!({})),
            Some(&unavailable),
        );
        assert!(routed.get("LFE").is_none());
        assert_eq!(
            route_target_unavailable_reason(
                &graph,
                "LFE",
                &target,
                Some(&json!({})),
                Some(&unavailable),
                references.get("LFE"),
            )
            .as_deref(),
            Some("playback rate missing")
        );

        let malformed_available = json!({"LFE":{"status":"available"}});
        let off_grid_response = json!({"LFE":[-1.0,-2.0]});
        let reason = route_target_unavailable_reason(
            &graph,
            "LFE",
            &target,
            Some(&off_grid_response),
            Some(&malformed_available),
            references.get("LFE"),
        );
        assert!(
            reason
                .as_deref()
                .is_some_and(|reason| reason.contains("off-grid"))
        );
        let malformed = build_target_overlay_curves_with_status(
            &graph,
            &references,
            Some(&target),
            Some(&off_grid_response),
            Some(&malformed_available),
        );
        assert!(malformed.get("LFE").is_none());

        let baseline_graph = json!({"channels":{},"metadata":{"bass_management":{"routing_graph":{"routes":[{
            "source_channel":"LFE","route_kind":"direct","low_pass_hz":80.0
        }]}}}});
        let baseline = build_target_overlay_curves_with_status(
            &baseline_graph,
            &references,
            Some(&target),
            None,
            Some(&json!({})),
        );
        assert!(baseline.get("LFE").is_some());
        assert_eq!(
            baseline,
            build_target_overlay_curves(&baseline_graph, &references, Some(&target), None,)
        );
    }
}
