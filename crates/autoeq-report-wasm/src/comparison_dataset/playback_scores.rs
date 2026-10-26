//! Read complete native configured-playback receipts without inventing scores.
use serde_json::Value;
use std::collections::BTreeSet;

fn sha256(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

pub(super) fn configured_mean(metadata: &Value) -> Option<(f64, f64, usize)> {
    let training = &metadata["correction_acceptance"]["acoustic_quality"]["training"];
    let configured = &training["configured_baseline"];
    if configured["method"].as_str()? != "configured_routing_unprocessed_captures_v1"
        || !configured["unavailable"].as_array()?.is_empty()
        || !sha256(&configured["playback_graph_sha256"])
    {
        return None;
    }
    let rows = configured["residuals"].as_array()?;
    if rows.is_empty() || training["curve_count"].as_u64()? != rows.len() as u64 {
        return None;
    }
    let mut indices = BTreeSet::new();
    let mut seats = BTreeSet::new();
    let mut pre = 0.0;
    let mut post = 0.0;
    for row in rows {
        let score = &row["score"];
        let index = score["curve_index"].as_u64()?;
        let input = score["logical_input"].as_str()?;
        let outputs = row["physical_outputs"].as_array()?;
        let mut physical = BTreeSet::new();
        if input.is_empty()
            || outputs.is_empty()
            || !indices.insert(index)
            || index >= rows.len() as u64
            || !seats.insert((input, row["seat_index"].as_u64()?))
            || outputs.iter().any(|output| {
                output
                    .as_str()
                    .is_none_or(|name| name.is_empty() || !physical.insert(name))
            })
            || score["sample_count"].as_u64()? < 2
            || !score["post_covers_baseline_grid"].as_bool()?
            || score["normalized_to_pre"].as_bool().is_none()
            || [
                "frequency_grid_sha256",
                "baseline_samples_sha256",
                "target_samples_sha256",
            ]
            .iter()
            .any(|key| !sha256(&score[key]))
        {
            return None;
        }
        let band = score["evaluated_band_hz"].as_array()?;
        if band.len() != 2 {
            return None;
        }
        let low = band[0].as_f64()?;
        let high = band[1].as_f64()?;
        let before = score["pre_weighted_rms_db"].as_f64()?;
        let after = score["post_weighted_rms_db"].as_f64()?;
        let improvement = score["improvement_db"].as_f64()?;
        if [
            low,
            high,
            before,
            after,
            improvement,
            score["level_offset_db"].as_f64()?,
            score["post_reference_gain_db"].as_f64()?,
        ]
        .iter()
        .any(|value| !value.is_finite())
            || low <= 0.0
            || high <= low
            || before < 0.0
            || after < 0.0
            || (before - after - improvement).abs() > 1e-9
        {
            return None;
        }
        pre += before / rows.len() as f64;
        post += after / rows.len() as f64;
    }
    (pre.is_finite() && post.is_finite()).then_some((pre, post, rows.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(super) fn metadata() -> Value {
        let rows: Vec<_> = [1.0, 2.0, 9.0].into_iter().enumerate().map(|(index, pre)| json!({
            "seat_index":index,"physical_outputs":["L"],"score":{
                "curve_index":index,"logical_input":"L","sample_count":128,
                "frequency_grid_sha256":"a".repeat(64),"baseline_samples_sha256":"b".repeat(64),"target_samples_sha256":"c".repeat(64),
                "evaluated_band_hz":[20.0,20000.0],"normalized_to_pre":true,
                "level_offset_db":80.0,"post_reference_gain_db":0.0,"post_covers_baseline_grid":true,
                "pre_weighted_rms_db":pre,"post_weighted_rms_db":1.0,"improvement_db":pre-1.0
            }
        })).collect();
        json!({"pre_score":5.8,"post_score":3.2,"correction_acceptance":{
            "outcome":"safe_fallback","acoustic_quality":{"training":{
                "curve_count":3,"configured_baseline":{
                    "method":"configured_routing_unprocessed_captures_v1","playback_graph_sha256":"d".repeat(64),
                    "unavailable":[],"residuals":rows
                }
            }}
        }})
    }

    #[test]
    fn complete_playback_receipts_use_mean_without_training_or_native_score_substitution() {
        assert_eq!(configured_mean(&metadata()), Some((4.0, 1.0, 3)));
    }

    #[test]
    fn incomplete_playback_receipts_never_become_zero_or_partial_scores() {
        for (key, value) in [
            ("post_covers_baseline_grid", json!(false)),
            ("post_weighted_rms_db", Value::Null),
            ("improvement_db", json!(99.0)),
            ("sample_count", json!(true)),
            ("baseline_samples_sha256", json!("invalid")),
            ("curve_index", json!(0)),
        ] {
            let mut data = metadata();
            data["correction_acceptance"]["acoustic_quality"]["training"]["configured_baseline"]
                ["residuals"][1]["score"][key] = value;
            assert_eq!(configured_mean(&data), None, "{key}");
        }
        let mut data = metadata();
        data["correction_acceptance"]["acoustic_quality"]["training"]["configured_baseline"]["unavailable"] =
            json!(["missing physical response"]);
        assert_eq!(configured_mean(&data), None);
    }
}
