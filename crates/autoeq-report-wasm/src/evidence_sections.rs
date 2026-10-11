//! Renders saved playback, psychoacoustic, and FIR timing evidence.
//!
//! These sections display serialized evidence without repeating measurement or
//! payload verification. Playback status requires the caller's binding result
//! from the authoritative bundle verifier.

// Rust guideline compliant 2026-02-21

use crate::report_helpers::{
    channel_sort_key, escape_html, format_ms, numeric_value, python_string,
};
use serde_json::{Value, json};

const OK: &str = "#2ecc71";
const WARN: &str = "#f1c40f";
const BAD: &str = "#e74c3c";
const EMPTY_CELL: &str = "<td style=\"color:#999\">-</td>";

fn populated(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn number(value: &Value) -> Option<f64> {
    numeric_value(value).filter(|value| value.is_finite())
}

fn status_box(title: &str, color: &str, paragraphs: &[String]) -> String {
    let mut html = format!(
        "<section class=\"playback-status-box\" role=\"note\" style=\"flex:1;min-width:260px;border:2px solid {color};padding:16px\"><h2>{}</h2>",
        escape_html(title)
    );
    for detail in paragraphs {
        html.push_str(&format!("<p>{}</p>", escape_html(detail)));
    }
    html.push_str("</section>\n");
    html
}

fn playback_status(request: &Value) -> String {
    let metadata = &request["metadata"];
    let acceptance = &metadata["correction_acceptance"];
    let outcome = acceptance["outcome"].as_str();
    let eligible = outcome == Some("unchanged")
        || (outcome == Some("accepted")
            && acceptance["accepted"] == true
            && acceptance["decision"] == "accepted");
    let verified = request["binding_verified"] == true;
    let approved = eligible && verified;
    let label = request["label"].as_str().unwrap_or_default();
    let prefix = if label.is_empty() {
        String::new()
    } else {
        format!("{label} — ")
    };
    let title = format!(
        "{prefix}Saved DSP playback eligibility: {}",
        outcome
            .filter(|value| !value.is_empty())
            .unwrap_or("unverified")
    );
    let (eligibility_color, detail) = match outcome {
        Some("accepted" | "unchanged") if eligible => (
            OK,
            format!("Recorded verdict: {}.", outcome.unwrap_or_default()),
        ),
        Some("accepted" | "unchanged") => (
            BAD,
            format!(
                "Recorded verdict is contradictory: outcome '{}' without matching accepted/decision flags.",
                outcome.unwrap_or_default()
            ),
        ),
        Some("rejected") => (BAD, "Recorded verdict: rejected.".into()),
        None => (
            WARN,
            "No recorded acceptance verdict (legacy or unverified output).".into(),
        ),
        Some(value) => (
            WARN,
            format!("Recorded verdict: {value} — no playback approval."),
        ),
    };
    let mut eligibility = vec![detail];
    let violations: Vec<_> = acceptance["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .collect();
    if !violations.is_empty() {
        eligibility.push(format!("Recorded violations: {}.", violations.join("; ")));
    }
    let binding_title = format!(
        "{prefix}Delivered payload: {}",
        if verified { "verified" } else { "not verified" }
    );
    let mut playback = vec![if approved {
        "Approved for playback under the declared contract.".to_string()
    } else {
        "Not approved for playback. These curves are diagnostic only.".to_string()
    }];
    if approved {
        playback.push("Recorded validation is evidence for the saved graph and its declared playback contract, not an independent replay by this report.".into());
    }
    let policy = &metadata["effective_config"]["optimizer"]["finalization"];
    let mut peaks = std::collections::BTreeMap::new();
    peaks.insert(
        "default",
        policy
            .get("default_input_peak")
            .and_then(number)
            .unwrap_or(1.0),
    );
    if let Some(limits) = policy["input_peak_limits"].as_object() {
        for (name, value) in limits {
            if let Some(value) = number(value) {
                peaks.insert(name.as_str(), value);
            } else {
                peaks.remove(name.as_str());
            }
        }
    }
    let reduced: Vec<_> = peaks
        .into_iter()
        .filter(|(_, value)| *value > 0.0 && *value < 1.0)
        .collect();
    if !reduced.is_empty() {
        let values = reduced
            .iter()
            .map(|(name, peak)| format!("{name}: {:.2} dBFS", 20.0 * peak.log10()))
            .collect::<Vec<_>>()
            .join(", ");
        playback.push(format!("Conditional on enforced input-peak ceilings ({values}). This report does not enforce them; full-scale input safety is not established."));
    }
    let limited: Vec<_> = metadata["stage_outcomes"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|stage| stage["checks"].as_array().into_iter().flatten())
        .filter_map(|check| {
            check["id"]
                .as_str()?
                .strip_prefix("runtime_limiter_physical_output:")
        })
        .collect();
    if !limited.is_empty() {
        playback.push(format!("Runtime limiter required on physical outputs: {}. Response curves describe small-signal playback below limiting. Loud bass peaks may be reduced dynamically. Protection is sample-peak, not a true-peak or loudspeaker-excursion guarantee. Do not bypass the limiter or add downstream gain.", limited.join(", ")));
    }
    let playback_color = if !approved {
        BAD
    } else if !reduced.is_empty() || !limited.is_empty() {
        WARN
    } else {
        OK
    };
    let mut html = "<section class=\"playback-status\" role=\"group\" aria-label=\"Playback verdict\" style=\"display:flex;gap:12px;flex-wrap:wrap;margin:16px 0;border:0;padding:0\">".to_string();
    html.push_str(&status_box(&title, eligibility_color, &eligibility));
    html.push_str(&status_box(
        &binding_title,
        if verified { OK } else { WARN },
        &[request["binding_reason"]
            .as_str()
            .unwrap_or("Binding evidence unavailable.")
            .to_string()],
    ));
    html.push_str(&status_box(
        &format!("{prefix}Playback approval"),
        playback_color,
        &playback,
    ));
    html.push_str("</section>\n");
    html
}

fn group_delay(metadata: &Value) -> String {
    let gd = &metadata["group_delay"];
    if !populated(gd) {
        return EMPTY_CELL.repeat(5);
    }
    let advisory = gd
        .get("advisory")
        .map(python_string)
        .unwrap_or_else(|| "-".into());
    let advisory_color = if advisory == "success" { OK } else { "#b9770e" };
    let applied = gd["applied"].as_bool();
    let applied_str = match applied {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    };
    let applied_color = if applied == Some(true) { OK } else { "#999" };
    let improvement = number(&gd["improvement_db"]);
    let improvement_str = improvement
        .map(|value| format!("{value:+.2} dB"))
        .unwrap_or_else(|| "-".into());
    let improvement_color = if improvement.is_some_and(|value| value >= 0.0) {
        OK
    } else {
        BAD
    };
    // Python counts integer entries, including booleans, but excludes float entries.
    let ap_total: i128 = gd["per_channel_ap_count"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            value
                .as_i64()
                .map(i128::from)
                .or_else(|| value.as_u64().map(i128::from))
                .or_else(|| value.as_bool().map(i128::from))
        })
        .sum();
    let coherence = number(&gd["mean_coherence"])
        .map(|value| format!(", coh={value:.2}"))
        .unwrap_or_default();
    format!(
        "<td style=\"color:{advisory_color};font-weight:600\">{}</td><td style=\"color:{applied_color};font-weight:600\">{applied_str}</td><td>{} → {}</td><td style=\"color:{improvement_color};font-weight:600\">{improvement_str}</td><td>{ap_total}{coherence}</td>",
        escape_html(&advisory),
        format_ms(gd.get("sum_gd_pre_rms_ms")),
        format_ms(gd.get("sum_gd_post_rms_ms"))
    )
}

fn ringing_pair(masking: &Value, peak: &str, audible: &str) -> String {
    match (number(&masking[peak]), number(&masking[audible])) {
        (Some(peak), Some(audible)) => format!("{peak:+.2} / {audible:+.2} dB"),
        (Some(peak), None) => format!("{peak:+.2} / - dB"),
        (None, Some(audible)) => format!("- / {audible:+.2} dB"),
        _ => "-".into(),
    }
}

fn mixed_phase(metadata: &Value, channels: &Value) -> String {
    let phase = &metadata["mixed_phase_per_channel"];
    let mut masking = std::collections::BTreeMap::new();
    if let Some(channels) = channels.as_object() {
        for (name, channel) in channels {
            if populated(&channel["fir_temporal_masking"]) {
                masking.insert(name.clone(), &channel["fir_temporal_masking"]);
            }
        }
    } else if let Some(channels) = channels.as_array() {
        for channel in channels {
            if populated(&channel["channel"]) && populated(&channel["fir_temporal_masking"]) {
                masking.insert(
                    python_string(&channel["channel"]),
                    &channel["fir_temporal_masking"],
                );
            }
        }
    }
    let perceptual = &metadata["perceptual_metrics"];
    let pre = number(&perceptual["fir_pre_ringing_audible_db"]);
    let post = number(&perceptual["fir_post_ringing_audible_db"]);
    let penalty = number(&perceptual["fir_temporal_masking_penalty"]);
    if !populated(phase)
        && masking.is_empty()
        && pre.is_none()
        && post.is_none()
        && penalty.is_none()
    {
        return String::new();
    }
    let mut names: Vec<_> = phase
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name.clone())
        .chain(masking.keys().cloned())
        .collect();
    names.sort_by(|left, right| channel_sort_key(left).cmp(&channel_sort_key(right)));
    names.dedup();
    let mut rows = String::new();
    for name in names {
        let report = &phase[&name];
        let empty = Value::Null;
        let mask = masking.get(&name).copied().unwrap_or(&empty);
        let taps = if report["fir_taps"].is_i64()
            || report["fir_taps"].is_u64()
            || report["fir_taps"].is_boolean()
        {
            python_string(&report["fir_taps"])
        } else {
            "-".into()
        };
        let range = match (
            number(&report["residual_excess_phase_min_deg"]),
            number(&report["residual_excess_phase_max_deg"]),
        ) {
            (Some(low), Some(high)) => format!("{low:+.2}° to {high:+.2}°"),
            _ => "-".into(),
        };
        let rms = number(&report["residual_excess_phase_rms_deg"])
            .map(|value| format!("{value:.2}°"))
            .unwrap_or_else(|| "-".into());
        let penalty = number(&mask["penalty"])
            .map(|value| format!("{value:.3}"))
            .unwrap_or_else(|| "-".into());
        rows.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{taps}</td><td>{range}</td><td>{rms}</td><td>{}</td><td>{}</td><td>{}</td><td>{penalty}</td></tr>\n", escape_html(&name), format_ms(report.get("estimated_delay_ms")), format_ms(mask.get("main_time_ms")), ringing_pair(mask,"pre_ringing_peak_db","pre_ringing_audible_db"), ringing_pair(mask,"post_ringing_peak_db","post_ringing_audible_db")));
    }
    let mut parts = Vec::new();
    if pre.is_some() || post.is_some() {
        parts.push(format!(
            "Worst audible pre/post ringing: {} / {}",
            pre.map(|value| format!("{value:+.2} dB"))
                .unwrap_or_else(|| "-".into()),
            post.map(|value| format!("{value:+.2} dB"))
                .unwrap_or_else(|| "-".into())
        ));
    }
    if let Some(penalty) = penalty {
        parts.push(format!("worst temporal masking penalty: {penalty:.3}"));
    }
    let mut html = "<div class=\"filters-section mixed-phase-section\">\n<h3>Mixed-Phase and FIR Timing</h3>\n".to_string();
    if !rows.is_empty() {
        html.push_str("<table class=\"bm-table\">\n<thead><tr><th>Channel</th><th>Estimated delay</th><th>FIR taps</th><th>Residual phase range</th><th>Residual RMS</th><th>Main impulse</th><th>Pre peak / audible</th><th>Post peak / audible</th><th>Penalty</th></tr></thead>\n<tbody>\n");
        html.push_str(&rows);
        html.push_str("</tbody>\n</table>\n");
    }
    if !parts.is_empty() {
        html.push_str(&format!(
            "<p class=\"epa-footer\">{}.</p>\n",
            escape_html(&parts.join("; "))
        ));
    }
    html.push_str("</div>\n");
    html
}

// Labels and precision are the existing EPA display contract, not scoring rules.
pub(crate) const EPA_FIELDS: &[(&str, &str, usize)] = &[
    ("preference", "Preference", 2),
    ("evaluation", "Evaluation", 2),
    ("potency", "Potency", 2),
    ("activity", "Activity", 2),
    ("sharpness_acum", "Sharpness (acum)", 2),
    ("roughness", "Roughness", 3),
    ("total_loudness_sone", "Total loudness (sone)", 2),
    ("loudness_balance", "Loudness balance", 3),
];

fn delta(field: &str, pre: &Value, post: &Value) -> String {
    let (Some(pre), Some(post)) = (number(pre), number(post)) else {
        return "<span style=\"color:#999\">-</span>".into();
    };
    let delta = post - pre;
    if delta.abs() < 1e-9 {
        return "<span style=\"color:#999\">=</span>".into();
    }
    let higher_is_better = matches!(
        field,
        "preference" | "evaluation" | "total_loudness_sone" | "loudness_balance"
    );
    let color = if (delta > 0.0) == higher_is_better {
        OK
    } else {
        BAD
    };
    format!("<span style=\"color:{color};font-weight:600\">{delta:+.3}</span>")
}

fn epa_channel(epa: &Value) -> String {
    let (pre, post) = (&epa["pre"], &epa["post"]);
    if !populated(pre) && !populated(post) {
        return String::new();
    }
    let mut rows = String::new();
    for (field, label, precision) in EPA_FIELDS {
        let formatted = |value: &Value| {
            number(value)
                .map(|value| format!("{value:+.precision$}"))
                .unwrap_or_else(|| "-".into())
        };
        rows.push_str(&format!(
            "                        <tr><td>{label}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            formatted(&pre[*field]),
            formatted(&post[*field]),
            delta(field, &pre[*field], &post[*field])
        ));
    }
    format!(
        "                <div class=\"epa-section\">\n                    <h3>EPA Psychoacoustic Scores</h3>\n                    <table class=\"epa-table\">\n                        <thead><tr><th>Metric</th><th>Before EQ</th><th>After EQ</th><th>Δ</th></tr></thead>\n                        <tbody>\n{rows}                        </tbody>\n                    </table>\n                    <p class=\"epa-footer\">Higher is better for Preference / Evaluation / Total loudness / Loudness balance; lower is better for Activity / Sharpness deviation / Roughness. Δ cells are green when the change improves the metric.</p>\n                </div>\n"
    )
}

fn epa_summary(metadata: &Value) -> Value {
    let mean = |side: &str| {
        let values: Vec<_> = metadata["epa_per_channel"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, entry)| number(&entry[side]["preference"]))
            .collect();
        if values.is_empty() {
            Value::Null
        } else {
            json!(values.iter().sum::<f64>() / values.len() as f64)
        }
    };
    json!([mean("pre"), mean("post")])
}

/// Produces a display section from saved evidence and verified binding status.
///
/// # Errors
/// Returns an error for an unsupported builder or missing playback binding data.
pub fn evidence_section(request: &Value) -> Result<Value, String> {
    Ok(match request["builder"].as_str().unwrap_or_default() {
        "playback_status" => {
            if !request["binding_verified"].is_boolean() || !request["binding_reason"].is_string() {
                return Err("playback status requires a verified binding result and reason".into());
            }
            json!(playback_status(request))
        }
        "group_delay" => json!(group_delay(&request["metadata"])),
        "mixed_phase" => json!(mixed_phase(&request["metadata"], &request["channels"])),
        "epa_channel" => json!(epa_channel(&request["epa"])),
        "epa_summary" => epa_summary(&request["metadata"]),
        "delta" => json!(delta(
            request["field"].as_str().unwrap_or_default(),
            &request["pre"],
            &request["post"]
        )),
        other => return Err(format!("unsupported evidence section builder `{other}`")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_evidence_sections_match_pre_port_python() {
        let cases: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/evidence_sections_python.json"
        ))
        .unwrap();
        for (index, case) in cases.as_array().unwrap().iter().enumerate() {
            assert_eq!(
                evidence_section(&case["request"]).unwrap(),
                case["expected"],
                "case {index}"
            );
        }
    }

    #[test]
    fn playback_does_not_infer_binding_from_acceptance_flags() {
        assert!(evidence_section(&json!({"builder":"playback_status","metadata":{"correction_acceptance":{"outcome":"accepted","accepted":true,"decision":"accepted"}}})).is_err());
        let html = evidence_section(&json!({"builder":"playback_status","metadata":{"correction_acceptance":{"outcome":"accepted","accepted":true,"decision":"accepted"}},"binding_verified":false,"binding_reason":"member hash mismatch"})).unwrap();
        assert!(
            html.as_str()
                .unwrap()
                .contains("Not approved for playback.")
        );
    }
}
