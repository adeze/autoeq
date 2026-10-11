//! Native command-line adapter for the shared report producer APIs.
//!
//! Input is one JSON object on stdin. The `time_of_flight` operation returns
//! a JSON string; `crossover_config_html` returns a JSON string; `assemble`
//! returns raw HTML. Errors are printed to stderr and produce a nonzero exit
//! status.

// Rust guideline compliant 2026-02-21

use std::io::{self, Read};
use std::process::ExitCode;

use autoeq_report_wasm::assemble::{ShellAssets, assemble_html, checked_in_assets};
use autoeq_report_wasm::{crossover_config_html, time_of_flight_html, time_of_flight_rows};
use serde_json::Value;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("report-producer: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("could not read stdin: {error}"))?;
    let request: Value =
        serde_json::from_str(&input).map_err(|error| format!("invalid JSON request: {error}"))?;
    let operation = request
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| "request must contain a string `operation`".to_string())?;

    match operation {
        "time_of_flight" => {
            let metadata = request.get("metadata").unwrap_or(&Value::Null);
            let html = time_of_flight_html(metadata);
            let encoded = serde_json::to_string(&html)
                .map_err(|error| format!("could not encode Time of Flight response: {error}"))?;
            println!("{encoded}");
        }
        "time_of_flight_rows" => {
            let metadata = request.get("metadata").unwrap_or(&Value::Null);
            let rows = serde_json::to_string(&time_of_flight_rows(metadata))
                .map_err(|error| format!("could not encode Time of Flight rows: {error}"))?;
            println!("{rows}");
        }
        "crossover_config_html" => {
            let data = request.get("data").unwrap_or(&Value::Null);
            let html = crossover_config_html(data);
            let encoded = serde_json::to_string(&html).map_err(|error| {
                format!("could not encode crossover configuration response: {error}")
            })?;
            println!("{encoded}");
        }
        "assemble" => {
            let title = request
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| "assemble request must contain a string `title`".to_string())?;
            let payload = request
                .get("payload")
                .ok_or_else(|| "assemble request must contain `payload`".to_string())?;
            let payload_json = serde_json::to_string(payload)
                .map_err(|error| format!("could not encode report payload: {error}"))?;
            let assets = match request.get("shell_assets") {
                Some(value) => serde_json::from_value::<ShellAssets>(value.clone())
                    .map_err(|error| format!("invalid `shell_assets`: {error}"))?,
                None => checked_in_assets()
                    .map_err(|error| format!("could not load checked-in shell assets: {error}"))?,
            };
            print!("{}", assemble_html(title, &payload_json, &assets));
        }
        other => return Err(format!("unsupported operation `{other}`")),
    }
    Ok(())
}
