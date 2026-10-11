//! Parses file-backed target-curve CSV data for report consumers.
//!
//! The parser mirrors the legacy Python loader's aliases, permissive row widths,
//! numeric filtering, duplicate-frequency replacement, and sorted output. It
//! limits input to 8 MiB and fields to 131,072 characters before conversion.
//!
//! # Example
//!
//! ```
//! use autoeq_report_wasm::target_csv::parse_target_csv;
//! let target = parse_target_csv("frequency,spl\n100,80\n").unwrap();
//! assert_eq!(target["freq"][0], 100.0);
//! ```
//!
//! # Errors
//!
//! Missing columns, invalid numbers, invalid CSV, and empty usable data return
//! a structured error that the Python boundary maps to legacy warning behavior.
//!
//! # Panics
//!
//! This module does not intentionally panic for caller-provided CSV data.

// Rust guideline compliant 2026-02-21

use std::fmt;

use serde_json::{Value, json};

const FREQUENCY_COLUMNS: [&str; 4] = ["frequency", "freq", "frequency_hz", "freq_hz"];
const SPL_COLUMNS: [&str; 4] = ["spl", "spl_db", "level", "level_db"];
// The parser runs in both native and WASM report processes; bound input size
// before CSV allocation and JSON transport.
pub const MAX_TARGET_CSV_BYTES: usize = 8 * 1024 * 1024;
/// Maximum decoded characters in one CSV field, matching Python's default.
pub const MAX_TARGET_CSV_FIELD_CHARS: usize = 131_072;

/// Describes why target CSV data could not be converted into a target curve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetCsvError {
    /// The header or a row lacks one of the required selected columns.
    MissingColumns,
    /// A selected frequency or SPL cell is not a valid Python-style float.
    InvalidNumber(String),
    /// No finite SPL sample has a positive finite frequency.
    NoFinitePoints,
    /// CSV input exceeds the report parser's memory bound.
    TooLarge,
    /// A decoded header or row field exceeds Python's default field limit.
    FieldTooLarge,
    /// The CSV syntax or encoding is invalid.
    Csv(String),
}

impl TargetCsvError {
    /// Returns the stable error category used by the Python boundary.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Csv(_) | Self::FieldTooLarge => "csv",
            Self::MissingColumns
            | Self::InvalidNumber(_)
            | Self::NoFinitePoints
            | Self::TooLarge => "value",
        }
    }
}

impl fmt::Display for TargetCsvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingColumns => {
                formatter.write_str("target CSV needs frequency/freq and spl/spl_db columns")
            }
            Self::InvalidNumber(value) => write!(
                formatter,
                "could not convert string to float: {}",
                python_string_repr(value)
            ),
            Self::NoFinitePoints => {
                formatter.write_str("target CSV has no finite positive-frequency points")
            }
            Self::TooLarge => formatter.write_str("target CSV exceeds the 8 MiB size limit"),
            Self::FieldTooLarge => write!(
                formatter,
                "field larger than field limit ({MAX_TARGET_CSV_FIELD_CHARS})"
            ),
            Self::Csv(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for TargetCsvError {}

/// Parses UTF-8 CSV text into a sorted, finite target curve.
///
/// A leading UTF-8 BOM is ignored. Header matching trims surrounding
/// whitespace and ignores case; aliases are selected in the same order
/// as the legacy loader. Duplicate normalized headers and frequencies keep
/// their final occurrence. Inputs larger than [`MAX_TARGET_CSV_BYTES`] are
/// rejected to bound native/WASM memory and transport use. Each decoded
/// field is also limited to [`MAX_TARGET_CSV_FIELD_CHARS`] characters, matching
/// Python's CSV parser default.
///
/// # Errors
/// Returns [`TargetCsvError`] when CSV syntax is invalid, selected columns or
/// values are missing, numeric conversion fails, or no usable points remain.
///
/// # Panics
/// This function does not panic for caller-provided input.
pub fn parse_target_csv(csv_text: &str) -> Result<Value, TargetCsvError> {
    if csv_text.len() > MAX_TARGET_CSV_BYTES {
        return Err(TargetCsvError::TooLarge);
    }
    let csv_text = csv_text.strip_prefix('\u{feff}').unwrap_or(csv_text);

    // Python's DictReader consumes the first physical blank record as its
    // header instead of skipping it. Subsequent nonempty rows consequently
    // lack our selected columns; preserve that distinction before the Rust
    // CSV reader's empty-line handling.
    if let Some(rest) = after_initial_blank_record(csv_text) {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_reader(rest.as_bytes());
        for record in reader.records() {
            let record = record.map_err(|error| TargetCsvError::Csv(error.to_string()))?;
            validate_field_sizes(&record)?;
            if !record.is_empty() {
                return Err(TargetCsvError::MissingColumns);
            }
        }
        return Err(TargetCsvError::NoFinitePoints);
    }

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|error| TargetCsvError::Csv(error.to_string()))?
        .clone();
    validate_field_sizes(&headers)?;
    let frequency_index = selected_column(&headers, &FREQUENCY_COLUMNS);
    let spl_index = selected_column(&headers, &SPL_COLUMNS);
    let mut points = Vec::<(f64, f64)>::new();

    for record in reader.records() {
        let record = record.map_err(|error| TargetCsvError::Csv(error.to_string()))?;
        validate_field_sizes(&record)?;
        let (Some(frequency_index), Some(spl_index)) = (frequency_index, spl_index) else {
            return Err(TargetCsvError::MissingColumns);
        };
        let (Some(frequency_text), Some(spl_text)) =
            (record.get(frequency_index), record.get(spl_index))
        else {
            return Err(TargetCsvError::MissingColumns);
        };
        let frequency = parse_python_float(frequency_text)
            .ok_or_else(|| TargetCsvError::InvalidNumber(frequency_text.to_owned()))?;
        let spl = parse_python_float(spl_text)
            .ok_or_else(|| TargetCsvError::InvalidNumber(spl_text.to_owned()))?;
        if frequency.is_finite() && frequency > 0.0 && spl.is_finite() {
            points.push((frequency, spl));
        }
    }

    if points.is_empty() {
        return Err(TargetCsvError::NoFinitePoints);
    }
    // `sort_by` is stable, so the final sample for a repeated frequency stays
    // last and can replace earlier values while making this O(n log n).
    points.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut unique_points = Vec::with_capacity(points.len());
    for (frequency, spl) in points {
        if let Some((last_frequency, last_spl)) = unique_points.last_mut()
            && *last_frequency == frequency
        {
            *last_spl = spl;
        } else {
            unique_points.push((frequency, spl));
        }
    }
    let (frequencies, levels): (Vec<f64>, Vec<f64>) = unique_points.into_iter().unzip();
    Ok(json!({"freq": frequencies, "spl": levels}))
}

fn after_initial_blank_record(csv_text: &str) -> Option<&str> {
    csv_text
        .strip_prefix("\r\n")
        .or_else(|| csv_text.strip_prefix('\n'))
        .or_else(|| csv_text.strip_prefix('\r'))
}

fn validate_field_sizes(record: &csv::StringRecord) -> Result<(), TargetCsvError> {
    if record
        .iter()
        .any(|field| field.chars().count() > MAX_TARGET_CSV_FIELD_CHARS)
    {
        return Err(TargetCsvError::FieldTooLarge);
    }
    Ok(())
}

fn selected_column(headers: &csv::StringRecord, aliases: &[&str]) -> Option<usize> {
    // DictReader builds a mapping in input order, so duplicate normalized
    // headers resolve to the last column. Alias priority then comes from the
    // ordered alias list rather than whichever matching column appears first.
    let normalized: Vec<String> = headers
        .iter()
        .map(|header| header.trim().to_lowercase())
        .collect();
    aliases
        .iter()
        .find_map(|alias| normalized.iter().rposition(|header| header == alias))
}

fn parse_python_float(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut cleaned = String::with_capacity(value.len());
    let characters: Vec<char> = value
        .chars()
        .map(|character| {
            decimal_digit(character)
                .map(|digit| char::from(b'0' + digit))
                .unwrap_or(character)
        })
        .collect();
    for (index, character) in characters.iter().copied().enumerate() {
        if character == '_'
            && (index == 0
                || index + 1 == characters.len()
                || !characters[index - 1].is_ascii_digit()
                || !characters[index + 1].is_ascii_digit())
        {
            return None;
        }
        if character != '_' {
            cleaned.push(character);
        }
    }
    cleaned.parse::<f64>().ok()
}

// Unicode 17.0 Decimal_Number digit block starts. Each block contains exactly
// ten digits; this preserves Python 3.15 float parsing for Unicode decimal
// digits without adding another Unicode-table dependency.
const DECIMAL_DIGIT_BLOCK_STARTS: [u32; 77] = [
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66,
    0xDE6, 0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90,
    0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
    0x104A0, 0x10D30, 0x10D40, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0,
    0x11650, 0x116C0, 0x116D0, 0x116DA, 0x11730, 0x118E0, 0x11950, 0x11BF0, 0x11C50, 0x11D50,
    0x11DA0, 0x11DE0, 0x11F50, 0x16130, 0x16A60, 0x16AC0, 0x16B50, 0x16D70, 0x1CCF0, 0x1D7CE,
    0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E5F1, 0x1E950, 0x1FBF0,
];

fn decimal_digit(character: char) -> Option<u8> {
    let codepoint = u32::from(character);
    let block = DECIMAL_DIGIT_BLOCK_STARTS.partition_point(|start| *start <= codepoint);
    let start = *DECIMAL_DIGIT_BLOCK_STARTS.get(block.checked_sub(1)?)?;
    let digit = codepoint.checked_sub(start)?;
    if digit < 10 {
        u8::try_from(digit).ok()
    } else {
        None
    }
}

fn python_string_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::with_capacity(value.len() + 2);
    result.push(quote);
    for character in value.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            character if character == quote => {
                result.push('\\');
                result.push(character);
            }
            character => result.push(character),
        }
    }
    result.push(quote);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn matches_captured_python_target_csv_goldens() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../tests/fixtures/target_csv_python_goldens.json"
        ))
        .expect("golden fixture should be valid JSON");

        for case in cases {
            let name = case["name"].as_str().unwrap_or("unnamed");
            let csv_text = if let Some(csv) = case["csv"].as_str() {
                csv.to_owned()
            } else {
                let prefix = case["csv_prefix"].as_str().unwrap_or_default();
                let suffix = case["csv_suffix"].as_str().unwrap_or_default();
                let repeated = case["repeat_char"].as_str().unwrap_or_default();
                let repeat_count = case["repeat_count"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                format!("{prefix}{}{suffix}", repeated.repeat(repeat_count))
            };
            let result = parse_target_csv(&csv_text);
            if let Some(expected) = case.get("target") {
                assert_eq!(result.unwrap(), *expected, "case {name}");
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), case["error"]["kind"], "case {name}");
                assert_eq!(error.to_string(), case["error"]["message"], "case {name}");
            }
        }
    }

    #[test]
    fn accepts_python_float_whitespace_underscores_and_nonfinite_filtering() {
        assert_eq!(
            parse_target_csv("freq,spl\n 1_000 , 8_0 \n2000,NaN\n").unwrap(),
            json!({"freq": [1000.0], "spl": [80.0]})
        );
        assert_eq!(
            parse_target_csv("frequency,spl\nInfinity,80\n").unwrap_err(),
            TargetCsvError::NoFinitePoints
        );
        assert_eq!(
            parse_target_csv("frequency,spl\n100,1__0\n").unwrap_err(),
            TargetCsvError::InvalidNumber("1__0".to_owned())
        );
        assert_eq!(
            parse_target_csv("frequency,spl\n١٢.٣,8٠\n").unwrap(),
            json!({"freq": [12.3], "spl": [80.0]})
        );
        assert_eq!(
            parse_target_csv("frequency,spl\n１２.３,８０\n").unwrap(),
            json!({"freq": [12.3], "spl": [80.0]})
        );
    }

    #[test]
    fn rejects_input_above_the_documented_size_limit() {
        let oversized = "x".repeat(MAX_TARGET_CSV_BYTES + 1);
        assert_eq!(parse_target_csv(&oversized), Err(TargetCsvError::TooLarge));
    }

    #[test]
    fn enforces_field_limit_on_headers_and_rows_before_numeric_conversion() {
        let long_field = "é".repeat(MAX_TARGET_CSV_FIELD_CHARS + 1);
        let row_csv = format!("frequency,spl,note\ninvalid,70,{long_field}\n");
        assert_eq!(
            parse_target_csv(&row_csv),
            Err(TargetCsvError::FieldTooLarge)
        );
        let header_csv = format!("frequency,spl,{long_field}\n100,80,note\n");
        assert_eq!(
            parse_target_csv(&header_csv),
            Err(TargetCsvError::FieldTooLarge)
        );
        assert_eq!(
            TargetCsvError::FieldTooLarge.to_string(),
            "field larger than field limit (131072)"
        );
    }
}
