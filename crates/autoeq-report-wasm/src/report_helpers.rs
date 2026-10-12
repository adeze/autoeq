//! Small formatting helpers shared by canonical report sections.

// Rust guideline compliant 2026-02-21

/// Returns the report's established classical channel sort key.
pub(crate) fn channel_sort_key(name: &str) -> (u16, &str) {
    const CHANNELS: &[(&str, u16)] = &[
        ("L", 10),
        ("LEFT", 10),
        ("R", 20),
        ("RIGHT", 20),
        ("C", 30),
        ("CENTER", 30),
        ("LFE", 40),
        ("SUB", 40),
        ("SUBWOOFER", 40),
        ("LFE1", 41),
        ("LFE2", 42),
        ("SL", 50),
        ("SURROUND LEFT", 50),
        ("LS", 50),
        ("SR", 60),
        ("SURROUND RIGHT", 60),
        ("RS", 60),
        ("SBL", 70),
        ("SURROUND BACK LEFT", 70),
        ("LBS", 70),
        ("LB", 70),
        ("SBR", 80),
        ("SURROUND BACK RIGHT", 80),
        ("RBS", 80),
        ("RB", 80),
        ("FHL", 90),
        ("FRONT HEIGHT LEFT", 90),
        ("FHR", 100),
        ("FRONT HEIGHT RIGHT", 100),
        ("BHL", 110),
        ("BACK HEIGHT LEFT", 110),
        ("BHR", 120),
        ("BACK HEIGHT RIGHT", 120),
    ];
    let uppercase = name.to_uppercase();
    let order = CHANNELS
        .iter()
        .find_map(|(alias, order)| {
            if uppercase == *alias
                || (uppercase.starts_with(alias)
                    && uppercase[alias.len()..]
                        .chars()
                        .next()
                        .is_some_and(|character| !character.is_alphanumeric()))
            {
                Some(*order)
            } else {
                None
            }
        })
        .unwrap_or(1000);
    (order, name)
}

/// Escapes text with the same quote handling as Python's `html.escape`.
pub(crate) fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

pub(crate) fn python_string(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_string(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

pub(crate) fn numeric_value(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| {
        value
            .as_bool()
            .map(|boolean| if boolean { 1.0 } else { 0.0 })
    })
}

pub(crate) fn format_db(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(numeric_value)
        .map(|number| format!("{number:+.2} dB"))
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn format_hz(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(numeric_value)
        .map(|number| format!("{number:.1} Hz"))
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn format_ms(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(numeric_value)
        .map(|number| format!("{number:.3} ms"))
        .unwrap_or_else(|| "-".to_string())
}
