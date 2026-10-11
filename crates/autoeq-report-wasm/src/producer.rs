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
}
