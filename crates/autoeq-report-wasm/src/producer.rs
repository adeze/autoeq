//! Dispatches canonical report requests for native, CLI, and WASM consumers.

#[cfg(not(target_arch = "wasm32"))]
use crate::assemble::checked_in_assets;
use crate::assemble::{ShellAssets, assemble_html};
use serde_json::Value;

fn encode(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("could not encode report result: {error}"))
}

/// Produces a report fragment or assembled document from one JSON request.
///
/// Section responses are serialized JSON; `assemble` returns raw HTML.
///
/// # Errors
/// Returns an error for malformed requests, unsupported operations, or missing assets.
pub fn produce(request: &Value) -> Result<String, String> {
    let operation = request["operation"]
        .as_str()
        .ok_or("request must contain a string `operation`")?;
    match operation {
        "dashboard_payload" => encode(&crate::dashboard_payload::dashboard_payload(request)?),
        "eq_section_html" => encode(
            &crate::eq_sections::eq_section_html(
                request["builder"]
                    .as_str()
                    .ok_or("missing EQ section builder")?,
                &request["prepared"],
            )
            .map_err(|error| error.to_string())?,
        ),
        "combined_figures" => encode(&crate::combined_figures::combined_figures(
            &request["prepared"],
            request.get("tab"),
        )),
        "eq_figure" => encode(&crate::eq_figures::build_eq_figure(
            request["builder"]
                .as_str()
                .ok_or("missing EQ figure builder")?,
            request,
        )?),
        "evidence_section" => encode(&crate::evidence_sections::evidence_section(request)?),
        "temporal_figure" => encode(&crate::temporal_figures::build_temporal_figure(
            request["builder"]
                .as_str()
                .ok_or("missing temporal figure builder")?,
            request,
        )?),
        "response_figures" => encode(&crate::response_figures::response_figure_report(
            &request["data"],
        )),
        "add_response_overlays" => encode(&crate::response_figures::add_channel_response_overlays(
            request["figure"].clone(),
            request["channel_name"]
                .as_str()
                .ok_or("missing channel name")?,
            request.get("target_curve").filter(|value| !value.is_null()),
            request
                .get("lfe_plus_channel_curve")
                .filter(|value| !value.is_null()),
        )),
        "resample_spl_onto_grid" => encode(&crate::response_figures::resample_spl_onto_grid(
            &request["source_freq"],
            &request["source_spl"],
            &request["target_freq"],
        )),
        "bass_figure" => match request["builder"].as_str() {
            Some("routing") => encode(&crate::bass_figures::routing_figure(
                &request["data"],
                request["tab"].as_str(),
            )),
            Some("headroom") => encode(&crate::bass_figures::headroom_figure(
                &request["data"],
                request["tab"].as_str(),
            )),
            _ => Err("unsupported bass figure builder".into()),
        },
        "report_data" => encode(&crate::report_data::helper_response(request)?),
        "target_overlay_curves" => encode(&crate::target_overlay::build_target_overlay_curves(
            &request["data"],
            &request["reference_curves"],
            request
                .get("loaded_target")
                .filter(|value| !value.is_null()),
            request
                .get("route_crossover_db")
                .filter(|value| !value.is_null()),
        )),
        "target_overlay_helper" => encode(&crate::target_overlay::helper_response(request)?),
        "ir_group_delay" => {
            let samples = |key: &str| -> Vec<f64> {
                request[key]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| value.as_f64().unwrap_or(f64::NAN))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let point_count = request["point_count"].as_i64().unwrap_or(512).max(8);
            // Report display grids are bounded independently of waveform length.
            if point_count > 65_536 {
                return Err("IR group-delay point count exceeds 65536".into());
            }
            let setting = |key: &str, default: f64| {
                request
                    .get(key)
                    .map(|value| value.as_f64().unwrap_or(f64::NAN))
                    .unwrap_or(default)
            };
            encode(&crate::ir_group_delay::compute_group_delay_from_ir(
                &samples("time_ms"),
                &samples("amplitude"),
                setting("min_freq", 20.0),
                setting("max_freq", 20_000.0),
                point_count as usize,
            ))
        }
        "acoustic_summary_report" => encode(&crate::acoustic_summary::operational_summary_report(
            &request["data"],
        )),
        "operational_summaries_by_channel" => encode(
            &crate::acoustic_summary::operational_summaries_by_channel(&request["data"]),
        ),
        "operational_response_cell" => encode(&crate::acoustic_summary::operational_response_cell(
            &request["summaries_by_channel"],
            request["name"].as_str().unwrap_or_default(),
        )),
        "summary_table_html" => encode(&crate::acoustic_summary::summary_table_html(
            &request["data"],
        )),
        "pending_cell" => encode(&crate::acoustic_summary::pending_cell(
            request["field"].as_str().unwrap_or_default(),
        )),
        "symmetric_groups" => {
            let names: Vec<String> = serde_json::from_value(request["names"].clone())
                .map_err(|error| format!("invalid channel names: {error}"))?;
            encode(&crate::symmetric_pairs::symmetric_groups(&names))
        }
        "pair_sum_difference" => encode(&crate::symmetric_pairs::pair_sum_difference(
            &request["a"],
            &request["b"],
        )),
        "acoustic_reflections" => encode(&crate::acoustic_reflections::reflection_report(
            &request["channel"],
            request["label"].as_str().unwrap_or_default(),
            request["tab"].as_str(),
        )),
        "t60_report" => encode(&crate::acoustic_t60::t60_report(&request["data"])),
        "t60_rows" => encode(&crate::acoustic_t60::t60_rows(&request["channel"])),
        "t60_itu_note" => encode(&crate::acoustic_t60::t60_itu_note(&request["reference"])),
        "acoustic_curves" => encode(&crate::acoustic_curves::curve_report(&request["data"])),
        "acoustic_band_mean" => encode(&crate::acoustic_curves::band_mean(
            &request["curve"],
            request["low_hz"].as_f64().unwrap_or(f64::NAN),
            request["high_hz"].as_f64().unwrap_or(f64::NAN),
        )),
        "acoustic_smooth_curve" => encode(&crate::acoustic_curves::smooth_curve(
            &request["curve"],
            request["octaves"].as_f64().unwrap_or(f64::NAN),
        )),
        "acoustic_notch_cell" => encode(&crate::acoustic_curves::notch_cell(
            request["notch"].as_f64(),
        )),
        "time_of_flight" => encode(&crate::time_of_flight_html(&request["metadata"])),
        "time_of_flight_rows" => encode(&crate::time_of_flight_rows(&request["metadata"])),
        "crossover_config_html" => encode(&crate::crossover_config_html(&request["data"])),
        "gain_summary_html" => encode(&crate::gain_summary_html(&request["data"])),
        "bass_management_sections" => encode(&crate::bass_management_sections(&request["report"])),
        "signal_flow_sections" => encode(&crate::signal_flow_sections(&request["data"])),
        "shared_sections" => encode(&crate::build_shared_sections(&request["data"])),
        "assemble" => {
            let title = request["title"]
                .as_str()
                .ok_or("assemble request must contain a string `title`")?;
            let payload = request
                .get("payload")
                .ok_or("assemble request must contain `payload`")?;
            let assets = match request.get("shell_assets") {
                Some(value) => serde_json::from_value::<ShellAssets>(value.clone())
                    .map_err(|error| format!("invalid `shell_assets`: {error}"))?,
                #[cfg(not(target_arch = "wasm32"))]
                None => checked_in_assets()
                    .map_err(|error| format!("could not load checked-in shell assets: {error}"))?,
                // The web producer must not embed its own checked-in WASM
                // binary (or the separate GPUI renderer) inside itself.
                #[cfg(target_arch = "wasm32")]
                None => return Err("WebAssembly assembly requires explicit `shell_assets`".into()),
            };
            Ok(assemble_html(title, &encode(payload)?, &assets))
        }
        other => Err(format!("unsupported operation `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn invalid_operations_fail_and_known_operation_matches_direct_api() {
        assert!(produce(&json!({})).is_err());
        assert!(produce(&json!({"operation":"unknown"})).is_err());
        let data = json!({"channels":{}});
        assert_eq!(
            produce(&json!({"operation":"shared_sections","data":data})).unwrap(),
            encode(&crate::build_shared_sections(&data)).unwrap()
        );
    }

    #[test]
    fn ir_dispatch_preserves_unavailable_bounds_and_clamps_signed_counts() {
        let mut request = json!({"operation":"ir_group_delay","time_ms":[0.0,0.125,0.25,0.375,0.5,0.625,0.75,0.875],"amplitude":[1,0,0,0,0,0,0,0],"point_count":-1});
        let value: Value = serde_json::from_str(&produce(&request).unwrap()).unwrap();
        assert_eq!(value["frequency_hz"].as_array().unwrap().len(), 8);
        request["min_freq"] = Value::Null;
        let value: Value = serde_json::from_str(&produce(&request).unwrap()).unwrap();
        assert!(value["frequency_hz"].as_array().unwrap().is_empty());
        request["point_count"] = json!(65_537);
        assert!(produce(&request).is_err());
    }
}
