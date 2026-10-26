//! Groups symmetric speakers and compares curves on matching frequency grids.

// Rust guideline compliant 2026-02-21
use serde_json::{Value, json};

/// Returns recognized symmetric groups and remaining channels in input order.
pub fn symmetric_groups(names: &[String]) -> Value {
    let mut buckets: Vec<(&str, Vec<&String>)> = Vec::new();
    for name in names {
        let key = match name.trim().to_uppercase().as_str() {
            "L" | "R" => "L+R",
            "SL" | "SR" => "SL+SR",
            "TFL" | "TFR" => "TFL+TFR",
            "TBL" | "TBR" => "TBL+TBR",
            "SSL" | "SSR" => "SSL+SSR",
            "LBL" | "LBR" => "LBL+LBR",
            _ => continue,
        };
        if let Some((_, members)) = buckets.iter_mut().find(|(label, _)| *label == key) {
            members.push(name);
        } else {
            buckets.push((key, vec![name]));
        }
    }
    let groups: Vec<_> = buckets
        .iter()
        .filter(|(_, members)| members.len() == 2)
        .collect();
    let unpaired: Vec<_> = names
        .iter()
        .filter(|name| !groups.iter().any(|(_, members)| members.contains(name)))
        .collect();
    json!([groups, unpaired])
}

fn points(curve: &Value) -> Option<Vec<(f64, f64)>> {
    let frequencies = curve["freq"].as_array()?;
    let spl = curve["spl"].as_array()?;
    if frequencies.is_empty() || frequencies.len() != spl.len() {
        return None;
    }
    let mut previous = 0.0;
    frequencies
        .iter()
        .zip(spl)
        .map(|(frequency, level)| {
            let frequency = frequency.as_f64()?;
            let level = level.as_f64()?;
            if !frequency.is_finite() || frequency <= previous || !level.is_finite() {
                return None;
            }
            previous = frequency;
            Some((frequency, level))
        })
        .collect()
}

/// Computes magnitude sum and difference, returning null for invalid or mismatched curves.
///
/// This display arithmetic assumes equal phase for the sum; it is not a measured
/// coherent response. Frequencies must agree within the existing relative 1e-9 tolerance.
pub fn pair_sum_difference(a: &Value, b: &Value) -> Value {
    let Some(a_points) = points(a) else {
        return Value::Null;
    };
    let Some(b_points) = points(b) else {
        return Value::Null;
    };
    if a_points.len() != b_points.len() {
        return Value::Null;
    }
    let mut sum = Vec::with_capacity(a_points.len());
    let mut difference = Vec::with_capacity(a_points.len());
    for ((fa, sa), (fb, sb)) in a_points.iter().zip(&b_points) {
        if (fa - fb).abs() > 1e-9 * fa.abs().max(fb.abs()).max(1.0) {
            return Value::Null;
        }
        let pa = 10.0_f64.powf(sa / 20.0);
        let pb = 10.0_f64.powf(sb / 20.0);
        if !pa.is_finite() || !pb.is_finite() || !(pa + pb).is_finite() {
            return Value::Null;
        }
        // Preserve the display floor used by the Python viewer for exact cancellation.
        let to_spl = |amplitude: f64| {
            if amplitude > 1e-12 {
                20.0 * amplitude.log10()
            } else {
                -120.0
            }
        };
        sum.push(to_spl(pa + pb));
        difference.push(to_spl((pa - pb).abs()));
    }
    json!({"freq": a["freq"], "sum_spl": sum, "diff_spl": difference})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_python_grouping_and_pair_arithmetic_match() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/symmetric_pairs_python.json"
        ))
        .unwrap();
        for case in fixture["groups"].as_array().unwrap() {
            let names: Vec<String> = serde_json::from_value(case["names"].clone()).unwrap();
            assert_eq!(symmetric_groups(&names), case["expected"]);
        }
        for case in fixture["pairs"].as_array().unwrap() {
            let actual = pair_sum_difference(&case["a"], &case["b"]);
            if case["expected"].is_null() {
                assert!(actual.is_null());
            } else {
                assert_eq!(actual["freq"], case["expected"]["freq"]);
                for field in ["sum_spl", "diff_spl"] {
                    for (actual, expected) in actual[field]
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(case["expected"][field].as_array().unwrap())
                    {
                        assert!(
                            (actual.as_f64().unwrap() - expected.as_f64().unwrap()).abs() < 1e-10
                        );
                    }
                }
            }
        }
    }
}
