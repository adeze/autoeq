//! Native command-line adapter for the shared report producer APIs.
//!
//! Input is one JSON object on stdin. Section operations return JSON strings,
//! objects, or arrays as documented by their operation; `assemble` returns raw
//! HTML. Errors are printed to stderr and produce a nonzero exit status.

// Rust guideline compliant 2026-02-21

use std::io::{self, Read};
use std::process::ExitCode;

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
    let output = autoeq_report_wasm::producer::produce(&request)?;
    print!("{output}");
    Ok(())
}
