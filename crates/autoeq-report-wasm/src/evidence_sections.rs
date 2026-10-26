//! Renders saved playback, psychoacoustic, and FIR timing evidence.
//!
//! These sections display serialized evidence without repeating measurement or
//! payload verification. Playback status requires the caller's binding result
//! from the authoritative bundle verifier.

// Rust guideline compliant 2026-02-21

use crate::report_helpers::{
    channel_sort_key, escape_html, format_ms, numeric_value, python_string,
};
use serde_json::{Map, Value, json};

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

fn labelled_heading(title: &str, label: &str) -> String {
    if label.is_empty() {
        title.to_string()
    } else {
        format!("{title} — {label}")
    }
}

fn waveform_status_html(data: &Value, label: &str) -> String {
    let title = labelled_heading("Waveform-view availability", label);
    let start = format!(
        "<section class=\"waveform-status\"><h2>{}</h2>",
        escape_html(&title)
    );
    let unavailable = |reason: &str| {
        format!(
            "{start}<p>Unavailable: {}.</p></section>",
            escape_html(reason)
        )
    };
    if !data.is_object() {
        return unavailable("malformed report data");
    }
    let metadata = &data["metadata"];
    if metadata.is_null() {
        return String::new();
    }
    if !metadata.is_object() {
        return unavailable("malformed waveform metadata");
    }
    // The legacy Python renderer treats an absent field as an old report with
    // no waveform-view stage. An explicit null remains malformed evidence.
    let Some(outcomes) = metadata.get("stage_outcomes") else {
        return String::new();
    };
    if !outcomes.is_array()
        || outcomes
            .as_array()
            .is_some_and(|items| items.iter().any(|item| !item.is_object()))
    {
        return unavailable("malformed stage outcomes");
    }
    let stages: Vec<_> = outcomes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|stage| stage["stage"] == "waveform_views")
        .collect();
    if stages.is_empty() {
        return String::new();
    }
    if stages.len() != 1 {
        return unavailable("conflicting waveform diagnostics");
    }
    let channels = &data["channels"];
    let Some(channel_map) = channels.as_object() else {
        return unavailable("malformed waveform channel inventory");
    };
    if channel_map.values().any(|channel| !channel.is_object()) {
        return unavailable("malformed waveform channel inventory");
    }
    let checks = &stages[0]["checks"];
    let Some(checks) = checks.as_array() else {
        return unavailable("malformed waveform checks");
    };
    if checks.iter().any(|check| !check.is_object()) {
        return unavailable("malformed waveform checks");
    }
    let expected: std::collections::BTreeSet<_> = channel_map
        .keys()
        .flat_map(|channel| [format!("pre_ir:{channel}"), format!("post_ir:{channel}")])
        .collect();
    let ids: Vec<_> = checks
        .iter()
        .filter_map(|check| check["id"].as_str())
        .collect();
    let unique: std::collections::BTreeSet<_> = ids.iter().copied().collect();
    if ids.len() != checks.len()
        || ids.len() != expected.len()
        || unique.len() != ids.len()
        || unique
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            != expected.iter().map(String::as_str).collect()
    {
        return unavailable("waveform checks do not cover each channel and view exactly once");
    }
    let mut sorted = checks.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|check| check["id"].as_str().unwrap_or_default());
    let mut rows = String::new();
    for check in sorted {
        let id = check["id"].as_str().unwrap_or_default();
        let Some((view, channel)) = id.split_once(':') else {
            return unavailable("waveform checks do not cover each channel and view exactly once");
        };
        let available = !channel_map[channel][view].is_null();
        let passed = check["passed"].as_bool();
        let state = match passed {
            Some(value) if available != value => {
                "Unavailable: recorded status and saved waveform disagree".to_string()
            }
            Some(true) => {
                let measured = view == "pre_ir"
                    && channel_map[channel]["t60_octaves"]["basis"] == "measured_room_ir";
                if measured {
                    "Measured room impulse response imported".to_string()
                } else if view == "post_ir" {
                    "Calculated impulse response saved — prediction after DSP".to_string()
                } else {
                    "Impulse response reconstructed from the input frequency response".to_string()
                }
            }
            Some(false) => format!(
                "Unavailable: {}",
                check["diagnostic"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .unwrap_or("no reason recorded")
            ),
            None => "Unavailable: recorded status and saved waveform disagree".to_string(),
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(channel),
            escape_html(view),
            escape_html(&state)
        ));
    }
    format!(
        "{start}<p>An impulse response shows amplitude over time. Before DSP uses the imported room IR when supplied; otherwise it is reconstructed from the input response. After DSP is a calculated prediction, not a new microphone measurement. Available means the waveform was saved and can be plotted.</p><table><thead><tr><th>Channel</th><th>View</th><th>Status / reason</th></tr></thead><tbody>{rows}</tbody></table></section>"
    )
}

fn finite(value: &Value) -> Option<f64> {
    value.as_f64().filter(|value| value.is_finite())
}

fn known(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| !text.trim().is_empty() && !text.trim().eq_ignore_ascii_case("unknown"))
}

fn clock_config(data: &Value) -> &Value {
    if data["speakers"].is_object() {
        data
    } else {
        data.get("metadata")
            .and_then(|value| value.get("effective_config"))
            .unwrap_or(&Value::Null)
    }
}

fn capture_reason(source: &Value) -> Option<String> {
    if !source.is_object() {
        return Some("capture provenance is unavailable".into());
    }
    let provenance = &source["provenance"];
    let capture = &provenance["capture"];
    if !capture.is_object() {
        return Some("per-microphone clock bounds are unavailable".into());
    }
    let measurements = source["measurements"].as_array();
    let takes = capture["takes"].as_array();
    let (Some(measurements), Some(takes)) = (measurements, takes) else {
        return Some("clock evidence does not identify every microphone".into());
    };
    if !(2..=4).contains(&takes.len()) || takes.len() != measurements.len() {
        return Some("clock evidence does not identify every microphone".into());
    }
    let geometry = capture["geometry"].as_str();
    if !matches!(geometry, Some("spread" | "compact")) {
        return Some("session geometry is unavailable".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut reference: Option<&str> = None;
    for take in takes {
        if !take.is_object() {
            return Some("invalid microphone clock record".into());
        }
        let mic = take["microphone_id"].as_str().unwrap_or("unknown");
        let prefix = format!("{mic}: ");
        if !known(&take["microphone_id"])
            || !ids.insert(mic.to_string())
            || !known(&take["device_id"])
            || !known(&take["calibration_id"])
        {
            return Some(format!(
                "{prefix}microphone, device, or calibration identity is invalid"
            ));
        }
        let position = take["position_m"].as_array();
        let uncertainty = finite(&take["position_uncertainty_mm"]);
        if position.is_none_or(|position| {
            position.len() != 3 || position.iter().any(|v| finite(v).is_none())
        }) || uncertainty
            .is_none_or(|value| value < 0.0 || (geometry == Some("compact") && value > 1.0))
        {
            return Some(format!("{prefix}microphone geometry is invalid"));
        }
        if finite(&take["gain_db"]).is_none()
            || !matches!(
                take["calibration_orientation"].as_str(),
                Some("on_axis" | "ninety_degrees")
            )
        {
            return Some(format!(
                "{prefix}gain or calibration orientation is invalid"
            ));
        }
        if take["correction_applied"] != "resampled" || take["preserves_acoustic_delay"] != true {
            return Some(format!(
                "{prefix}common-clock acoustic-delay evidence is unavailable"
            ));
        }
        if take["quality_passed"] != true {
            return Some(format!("{prefix}measurement quality is pending or failed"));
        }
        let skew = finite(&take["skew_ppm"]);
        if finite(&take["offset_samples"]).is_none()
            || skew.is_none_or(|value| value.abs() > 5000.0)
        {
            return Some(format!("{prefix}clock fit is invalid"));
        }
        let bound = finite(&take["residual_uncertainty_us"]);
        if bound.is_none_or(|value| value < 0.0 || value >= 50.0) {
            return Some(format!(
                "{prefix}residual bound is unavailable or not below 50 µs"
            ));
        }
        let declared = take["timing_reference_id"].as_str();
        if declared.is_none_or(|value| {
            !known(&take["timing_reference_id"]) || reference.is_some_and(|old| old != value)
        }) {
            return Some(format!(
                "{prefix}microphones do not share a timing reference"
            ));
        }
        reference = declared;
    }
    if reference != provenance["timing_reference_id"].as_str() {
        return Some("source and microphone timing references disagree".into());
    }
    if !matches!(
        provenance["capture_kind"].as_str(),
        Some("stationary_ir" | "direct_sound")
    ) {
        return Some("source contains magnitude-only acquisition evidence".into());
    }
    None
}

/// Returns the first persisted clock-gate failure for a requested output pair.
///
/// Channel-to-source resolution uses the same saved `system.speakers` mapping as
/// the capture-view renderer. A missing source, duplicate channel/source, or a
/// failed per-source clock check returns a reason; callers must not sum phase
/// coherently when this returns `Some`.
#[must_use]
pub fn coherent_capture_clock_reason(data: &Value, channels: [&str; 2]) -> Option<String> {
    if channels[0].is_empty() || channels[1].is_empty() || channels[0] == channels[1] {
        return Some("two distinct output channels are required".into());
    }
    let config = clock_config(data);
    let Some(sources) = config["speakers"].as_object() else {
        return Some("capture source inventory is unavailable".into());
    };
    let mappings = config["system"]["speakers"].as_object();
    let mut source_ids = std::collections::BTreeSet::new();
    for channel in channels {
        let source_id = mappings
            .and_then(|items| items.get(channel))
            .and_then(Value::as_str)
            .unwrap_or(channel);
        if !source_ids.insert(source_id) {
            return Some(format!(
                "output channels resolve to the same capture source: {source_id}"
            ));
        }
        let Some(source) = sources.get(source_id) else {
            return Some(format!("capture provenance is unavailable for {source_id}"));
        };
        if let Some(reason) = capture_reason(source) {
            return Some(format!("{source_id}: {reason}"));
        }
    }
    None
}

fn capture_clock_qa_html(data: &Value) -> String {
    let config = clock_config(data);
    let Some(speakers) = config["speakers"].as_object() else {
        return String::new();
    };
    let mut rows = String::new();
    for (source_name, source) in speakers {
        if !source.is_object() {
            continue;
        }
        let capture = &source["provenance"]["capture"];
        if !capture.is_object() {
            continue;
        }
        let reason = capture_reason(source);
        let takes = capture["takes"].as_array();
        let fallback = vec![Value::Object(Map::new())];
        for take in takes.filter(|items| !items.is_empty()).unwrap_or(&fallback) {
            if !take.is_object() {
                continue;
            }
            let number_text = |name: &str| {
                finite(&take[name])
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "unavailable".into())
            };
            let geometry = capture["geometry"].as_str().unwrap_or("unknown");
            let microphone = take["microphone_id"].as_str().unwrap_or("unknown");
            let device = take["device_id"].as_str().unwrap_or("unknown");
            let correction = take["correction_applied"].as_str().unwrap_or("unknown");
            let status = reason
                .as_deref()
                .unwrap_or("Clock gate passed; phase validity still required");
            let cells = [
                source_name.to_string(),
                geometry.into(),
                microphone.into(),
                device.into(),
                number_text("offset_samples"),
                number_text("skew_ppm"),
                number_text("residual_uncertainty_us"),
                correction.into(),
                status.into(),
            ];
            rows.push_str("<tr>");
            for cell in cells {
                rows.push_str(&format!("<td>{}</td>", escape_html(&cell)));
            }
            rows.push_str("</tr>");
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    "<section class=\"capture-clock-qa\"><h2>Capture clock QA</h2><p>Every participating microphone needs a finite residual bound below 50 µs, a common reference, and passed measurement quality. Missing evidence uses magnitude only. Bounds assume linear clocks and correctly isolated timing markers.</p><table class=\"summary-table\"><thead><tr><th>Source</th><th>Geometry</th><th>Microphone</th><th>Device</th><th>Offset (samples)</th><th>Skew (ppm)</th><th>Residual (µs)</th><th>Correction</th><th>Coherent-use status</th></tr></thead><tbody>".to_string()
        + &rows
        + "</tbody></table></section>"
}

/// Renders serialized captured-arrival evidence without estimating new directions.
pub fn capture_reflections_html(data: &Value) -> String {
    let config = clock_config(data);
    let channels = data
        .get("channels")
        .and_then(Value::as_object)
        .or_else(|| config.get("speakers").and_then(Value::as_object));
    let Some(channels) = channels else {
        return String::new();
    };
    let system_speakers = config["system"]["speakers"].as_object();
    let sources = config["speakers"].as_object();
    let mut sections = String::new();
    for channel in channels.keys() {
        let source_id = system_speakers
            .and_then(|speakers| speakers.get(channel))
            .and_then(Value::as_str)
            .unwrap_or(channel);
        let source = sources
            .and_then(|items| items.get(source_id))
            .unwrap_or(&Value::Null);
        let report = &source["provenance"]["capture"]["reflection_report"];
        if !report.is_object() {
            continue;
        }
        let mut events = Vec::new();
        let direct = report.get("direct_sound").filter(|event| event.is_object());
        if let Some(direct) = direct {
            events.push((direct, true));
        }
        if let Some(reflections) = report.get("early_reflections").and_then(Value::as_array) {
            events.extend(
                reflections
                    .iter()
                    .take(64)
                    .filter(|event| event.is_object())
                    .map(|event| (event, false)),
            );
        }
        let mut rows = String::new();
        let mut plot_series = Vec::new();
        for (index, (event, is_direct)) in events.into_iter().enumerate() {
            let relative = finite(&event["relative_ms"]);
            let level = finite(&event["level_db"]);
            let mut reason = capture_reflection_direction_reason(source, event);
            if report["source_id"] != source_id {
                reason = Some("arrival source identity does not match this channel".into());
            }
            if relative.is_none_or(|value| !(0.0..=80.0).contains(&value)) || level.is_none() {
                reason = Some("finite early-arrival time/level unavailable".into());
            }
            let label = if is_direct {
                "Direct candidate".to_string()
            } else {
                format!(
                    "Reflection candidate {}",
                    index + 1 - usize::from(direct.is_some())
                )
            };
            let has_direction = reason.is_none();
            let mut status = reason
                .clone()
                .unwrap_or_else(|| "Conditional measured direction".into());
            if has_direction {
                let direction = &event["direction"];
                let x = finite(&direction[0]).unwrap_or(0.0);
                let y = finite(&direction[1]).unwrap_or(0.0);
                let z = finite(&direction[2]).unwrap_or(0.0);
                let horizontal = x.hypot(y);
                let azimuth = y.atan2(x).to_degrees();
                let elevation = z.clamp(-1.0, 1.0).asin().to_degrees();
                if horizontal <= 1e-6 {
                    status = format!(
                        "Conditional vertical arrival; azimuth undefined, elevation {elevation:.1}°"
                    );
                } else {
                    status.push_str(&format!(
                        "; azimuth {azimuth:.1}°, elevation {elevation:.1}°"
                    ));
                    if !is_direct {
                        let time = relative.unwrap_or_default();
                        let level = level.unwrap_or_default();
                        plot_series.push(json!({
                            "name":format!("{label}: {level:.1} dB"),
                            "x":[azimuth,azimuth],
                            "y":[0.0,time],
                            "color":"#56b4e9",
                            "width":2.0,
                            "dash":"solid",
                            "visible":true,
                            "y_axis":0
                        }));
                    }
                }
            }
            if let Some(issues) = event.get("issues").and_then(Value::as_array) {
                let issues = issues
                    .iter()
                    .map(python_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                if !issues.is_empty() {
                    status.push_str("; ");
                    status.push_str(&issues);
                }
            }
            let time_text = relative
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "—".into());
            let level_text = level
                .map(|value| format!("{value:.1}"))
                .unwrap_or_else(|| "—".into());
            rows.push_str(&format!(
                "<tr><td>{}</td><td>{time_text}</td><td>{level_text}</td><td>{}</td></tr>",
                escape_html(&label),
                escape_html(&status)
            ));
        }
        let plot = if plot_series.is_empty() {
            String::new()
        } else {
            let figure = json!({
                "kind":"figure",
                "figure":{
                    "title":"Conditional arrival azimuth and delay",
                    "x":{"label":"Azimuth (degrees)","scale":"linear","min":-180,"max":180},
                    "y":{"label":"Delay from direct candidate (ms)","scale":"linear","min":0,"max":80},
                    "y2":null,"series":plot_series,"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true
                },
                "tab":null
            });
            format!(
                "{}<p>Azimuth uses the recorded session axes. Mirror-ambiguous arrivals are omitted.</p>",
                embedded_report_figure(&figure)
            )
        };
        let notes = report
            .get("issues")
            .and_then(Value::as_array)
            .map(|issues| {
                issues
                    .iter()
                    .map(python_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        sections.push_str(&format!(
            "<section class=\"capture-reflections\"><h3>{}: early-arrival candidates</h3><p>{}</p>{plot}<table class=\"summary-table\"><thead><tr><th>Event</th><th>Relative ms</th><th>Level dB</th><th>Evidence</th></tr></thead><tbody>{rows}</tbody></table></section>",
            escape_html(channel), escape_html(&notes)
        ));
    }
    if sections.is_empty() {
        return String::new();
    }
    format!(
        "<section><h2>Measured arrival evidence</h2><p>Energy-based arrival candidates and conditional plane-wave directions. Microphone phase and source distance are not independently verified; fit residuals are not angular uncertainty bounds.</p>{sections}</section>"
    )
}

fn capture_reflection_direction_reason(source: &Value, event: &Value) -> Option<String> {
    if let Some(reason) = capture_reason(source) {
        return Some(reason);
    }
    let capture = &source["provenance"]["capture"];
    if capture["geometry"] != "compact" {
        return Some("array directions require compact geometry".into());
    }
    if event["mirror_ambiguous"] != false {
        return Some("planar mirror ambiguity is unresolved".into());
    }
    let direction = event["direction"].as_array();
    let Some(direction) = direction.filter(|values| values.len() == 3) else {
        return Some("finite unit direction unavailable".into());
    };
    let Some(vector) = direction.iter().map(finite).collect::<Option<Vec<_>>>() else {
        return Some("finite unit direction unavailable".into());
    };
    let norm = vector.iter().map(|value| value * value).sum::<f64>();
    if (norm - 1.0).abs() > 1e-3 {
        return Some("finite unit direction unavailable".into());
    }
    if finite(&event["residual_samples"]).is_none_or(|value| !(0.0..=0.5).contains(&value)) {
        return Some("plane-wave fit residual unavailable or too large".into());
    }
    let band = event["band_hz"].as_array();
    if band.is_none_or(|values| {
        values.len() != 2
            || finite(&values[0]).is_none_or(|value| value <= 0.0)
            || finite(&values[1]).is_none_or(|value| value <= finite(&values[0]).unwrap_or(0.0))
            || finite(&values[1]).is_some_and(|value| value > 3000.0)
    }) {
        return Some("supported direction-analysis band unavailable".into());
    }
    let mut uncertainties = capture["takes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|take| finite(&take["residual_uncertainty_us"]))
        .collect::<Vec<_>>();
    uncertainties.sort_by(f64::total_cmp);
    uncertainties.reverse();
    let relative_us = uncertainties
        .first()
        .zip(uncertainties.get(1))
        .map(|(a, b)| a + b);
    if relative_us.is_none_or(|value| value <= 0.0)
        || finite(&band.unwrap()[1])
            .is_some_and(|value| value > 25_000.0 / relative_us.unwrap_or(1.0))
    {
        return Some(
            "relative timing uncertainty exceeds the direction-analysis band limit".into(),
        );
    }
    None
}

/// Renders the all-speaker resonance table from already validated saved grids.
pub fn resonance_summary_html(data: &Value) -> String {
    let Some(channels) = data.get("channels").and_then(Value::as_object) else {
        return String::new();
    };
    let mut entries = Vec::<(String, Option<Vec<Value>>)>::new();
    for (name, channel) in channels {
        let valid_drivers = channel["drivers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|driver| driver.is_object() && driver["measured_acoustics"].is_object())
            .collect::<Vec<_>>();
        if valid_drivers.is_empty() {
            entries.push((name.clone(), decay_modes(channel)));
        } else {
            for driver in valid_drivers {
                let driver_name = driver
                    .get("name")
                    .map(python_string)
                    .unwrap_or_else(|| "driver".into());
                let acoustic = &driver["measured_acoustics"];
                entries.push((format!("{name} / {driver_name}"), decay_modes(acoustic)));
            }
        }
    }
    if !entries.iter().any(|(_, modes)| modes.is_some()) {
        return String::new();
    }
    let count = entries
        .iter()
        .map(|(_, modes)| modes.as_ref().map_or(0, Vec::len))
        .max()
        .unwrap_or(0)
        .max(1);
    let mut html = String::from(
        "<h2>Room resonance summary — all speakers</h2><p>Pre-correction detections. Each cell gives frequency, 60 ms level relative to that speaker’s full-grid peak, and fitted decay time. Colours identify frequency, not severity. These candidates are not a geometric identification of room modes.</p><table><tr><th>Speaker</th>",
    );
    for index in 0..count {
        html.push_str(&format!("<th>Mode {}</th>", index + 1));
    }
    html.push_str("</tr>");
    for (name, modes) in entries {
        html.push_str(&format!("<tr><th>{}</th>", escape_html(&name)));
        match modes {
            None => html.push_str(&format!("<td colspan=\"{count}\">Unavailable</td>")),
            Some(modes) if modes.is_empty() => {
                html.push_str(&format!(
                    "<td colspan=\"{count}\">No candidates detected</td>"
                ));
            }
            Some(mut modes) => {
                modes.sort_by(|left, right| {
                    finite(&left["freq_hz"])
                        .unwrap_or_default()
                        .total_cmp(&finite(&right["freq_hz"]).unwrap_or_default())
                });
                for mode in &modes {
                    let frequency = finite(&mode["freq_hz"]).unwrap_or_default();
                    let level = finite(&mode["level_db"]).unwrap_or_default();
                    let decay = finite(&mode["decay_time_s"])
                        .map(|value| format!("{value:.3} s"))
                        .unwrap_or_else(|| "fit unavailable".into());
                    let color = resonance_color(frequency);
                    html.push_str(&format!(
                        "<td style=\"background:{color}33;border-top:4px solid {color}\">{frequency:.1} Hz<br>{level:.2} dB<br>{decay}</td>"
                    ));
                }
                for _ in 0..count.saturating_sub(modes.len()) {
                    html.push_str("<td>—</td>");
                }
            }
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    html
}

fn decay_modes(channel: &Value) -> Option<Vec<Value>> {
    let _ = crate::channel_evidence::optimization_waterfall_html(
        &channel["waterfall"],
        &channel["resonance_decays"],
    )?;
    channel["resonance_decays"]["decays"].as_array().cloned()
}

fn resonance_color(frequency: f64) -> String {
    let ratio = ((frequency.max(10.0) / 10.0).ln() / 30.0_f64.ln()).clamp(0.0, 1.0);
    let h = 0.5 + ratio / 3.0;
    let s = 0.65;
    let v = 0.8;
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f64;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8
    )
}

fn embedded_report_figure(section: &Value) -> String {
    let encoded = serde_json::to_string(section).unwrap_or_else(|_| "{}".into());
    format!(
        "<div class=\"report-figure\" data-section=\"{}\"></div>",
        escape_html(&encoded)
    )
}

/// Renders capture verification views only after the native host confirms the
/// frozen comparison payload bindings supplied in `verification`.
pub fn capture_views_html(report: &Value, verification: &Value) -> String {
    let unavailable = |reason: &str| {
        format!(
            "<section class=\"capture-views\"><h1>Capture acceptance diagnostics</h1><p>Unavailable: {}</p></section>",
            escape_html(reason)
        )
    };
    if !report.is_object() {
        return unavailable("'Value' object has no attribute 'get'");
    }
    let mut parts = vec![
        "<section class=\"capture-views\"><h1>Capture acceptance diagnostics</h1>".to_string(),
        "<p>These plots do not establish safety, improved room damping, or listener benefit. Hashes check artifact consistency, not operator declarations or hardware authenticity.</p>".to_string(),
    ];
    let status = report.get("status").filter(|value| !value.is_null());
    if let Some(status) = status {
        let status = status.as_str().unwrap_or_default();
        if !matches!(
            status,
            "accepted" | "unchanged" | "rejected" | "insufficient_evidence"
        ) {
            return unavailable("unsupported verification status");
        }
        let count = report["verified_captures"].as_u64();
        let exit_code = report["exit_code"].as_i64();
        let (Some(count), Some(exit_code)) = (count, exit_code) else {
            return unavailable("invalid verification capture count or exit code");
        };
        parts.push(format!(
            "<p>Recorded capture verification status: <strong>{}</strong>; {count} pre-result capture check(s); process exit code {exit_code}. This is the report’s recorded verdict, not a new validation by the viewer.</p>",
            escape_html(status)
        ));
        if !report["detail"].is_null() {
            let Some(detail) = report["detail"].as_str() else {
                return unavailable("invalid verification detail");
            };
            parts.push(format!("<p>Recorded detail: {}</p>", escape_html(detail)));
        }
    } else {
        parts.push(
            "<p>Recorded capture verification status unavailable in this legacy report.</p>".into(),
        );
    }
    let graph = report["graph_id"].as_str();
    let comparisons = report["comparisons"].as_array();
    let (Some(graph), Some(comparisons)) = (graph, comparisons) else {
        return unavailable("missing comparison identity or entries");
    };
    if graph.is_empty() || comparisons.is_empty() {
        return unavailable("missing comparison identity or entries");
    }
    if verification["graph_id"] != graph {
        return unavailable(
            "verification candidate graph does not match the saved optimization graph",
        );
    }
    let binding_rows = verification["comparisons"].as_array();
    let Some(binding_rows) = binding_rows else {
        return unavailable("capture-view payload verification is unavailable");
    };
    if binding_rows.len() != comparisons.len() {
        return unavailable(
            "capture-view payload verification does not cover each comparison exactly once",
        );
    }
    let mut t60_groups = Vec::<(String, String, Value)>::new();
    for (index, comparison) in comparisons.iter().enumerate() {
        let views = &comparison["capture_views"];
        if views.is_null() {
            parts.push("<p>Unavailable: legacy comparison has no capture views.</p>".into());
            continue;
        }
        let binding = binding_rows.get(index);
        let stored_binding = &comparison["capture_views_binding"];
        if binding.is_none_or(|binding| {
            binding["index"].as_u64() != Some(index as u64)
                || binding["verified"] != true
                || binding["source"] != comparison["source"]
                || binding["seat"] != comparison["seat"]
                || binding["payload_sha256"] != stored_binding["sha256"]
                || binding["payload_sha256"].as_str().is_none_or(str::is_empty)
        }) || stored_binding["algorithm"] != "sha256-json-typed-v1"
            || stored_binding["graph_identity"] != graph
            || views["candidate_graph_id"] != graph
            || views["source"] != comparison["source"]
            || views["seat"] != comparison["seat"]
        {
            return unavailable("capture view payload or graph/source/seat binding changed");
        }
        let Some(source) = views["source"].as_str() else {
            return unavailable("invalid capture identity or scope");
        };
        let Some(seat) = views["seat"].as_str() else {
            return unavailable("invalid capture identity or scope");
        };
        let Some(baseline) = views["baseline_graph_id"].as_str() else {
            return unavailable("invalid capture identity or scope");
        };
        let Some(scope) = views["scope"].as_str() else {
            return unavailable("invalid capture identity or scope");
        };
        let kind = views["evidence_kind"].as_str().unwrap_or_default();
        if !matches!(
            kind,
            "unavailable" | "synthetic_capture_pair" | "operator_declared_recorded_capture_pair"
        ) {
            return unavailable("unsupported capture evidence kind");
        }
        parts.push(format!(
            "<h2>{} / {}</h2>",
            escape_html(source),
            escape_html(seat)
        ));
        parts.push(format!(
            "<p>Matched IR evidence: {}; baseline {}; candidate {}</p><p>{}</p>",
            escape_html(kind),
            escape_html(baseline),
            escape_html(graph),
            escape_html(scope)
        ));
        if !views["ir_step"].is_null() {
            let view = &views["ir_step"];
            if view["provenance"]["graph_identity"] != graph {
                return unavailable("IR graph identity mismatch");
            }
            let Some(reference) = view["common_reference"].as_str() else {
                return unavailable("missing IR sample-zero reference");
            };
            parts.push(format!(
                "<p>Common sample-zero reference: {}</p>",
                escape_html(reference)
            ));
            match capture_trace_figure(view, "ir", "raw sample units", "IR") {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
            match capture_trace_figure(view, "step", "raw sample units", "STEP") {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
        }
        if !views["etc"].is_null() {
            let etc = &views["etc"];
            if etc["provenance"]["graph_identity"] != graph
                || etc["method"] != "hann_analytic_octave_linear_v1"
            {
                return unavailable("ETC graph identity or analysis method mismatch");
            }
            if etc["window_ms"] != json!([0.0, 40.0])
                || etc["bands"].as_array().is_none_or(|bands| bands.len() != 4)
                || etc["bands"].as_array().is_some_and(|bands| {
                    bands
                        .iter()
                        .map(|band| band["center_hz"].as_f64())
                        .collect::<Vec<_>>()
                        != [Some(500.0), Some(1000.0), Some(2000.0), Some(4000.0)]
                })
            {
                return unavailable("ETC band/window contract mismatch");
            }
            let Some(etc_scope) = etc["scope"].as_str() else {
                return unavailable("missing ETC scope");
            };
            parts.push(format!(
                "<h3>Matched octave ETC</h3><p>{}</p>",
                escape_html(etc_scope)
            ));
            parts.push(format!(
                "<p>Analysis half-support by band (ms): {}; display floor: {} dB.</p>",
                escape_html(&python_json(&etc["filter_half_support_ms"])),
                escape_html(&python_json(&etc["display_floor_db"]))
            ));
            for band in etc["bands"].as_array().unwrap() {
                let trace = json!({"times_ms":band["times_ms"],"pre_etc":band["pre_db"],"post_etc":band["post_db"]});
                let label = format!("ETC {} Hz", python_number(&band["center_hz"]));
                match capture_trace_figure(
                    &trace,
                    "etc",
                    "dB relative to the same baseline-band peak",
                    &label,
                ) {
                    Ok(html) => parts.push(html),
                    Err(reason) => return unavailable(&reason),
                }
            }
        }
        if !views["decay"].is_null() {
            match capture_decay_html(&views["decay"], graph) {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
        }
        if !views["octave_t60"].is_null() {
            match capture_octave_t60_html(&views["octave_t60"]) {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
            let band = &views["valid_band_hz"];
            let sample_rate = finite(&views["sample_rate_hz"]);
            let stimulus = views["stimulus_hash"].as_str();
            let settings = &views["settings"];
            if kind != "unavailable"
                && settings.is_object()
                && band.as_array().is_some_and(|values| {
                    values.len() == 2
                        && finite(&values[0]).is_some_and(|value| value > 0.0)
                        && finite(&values[1])
                            .is_some_and(|value| value > finite(&values[0]).unwrap_or(0.0))
                        && views["octave_t60"]["valid_band_hz"]
                            .as_array()
                            .is_some_and(|other| values == other)
                })
                && stimulus.is_some_and(|value| !value.is_empty())
                && sample_rate.is_some_and(|value| value > 0.0)
            {
                let key = format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    views["seat"],
                    kind,
                    views["baseline_graph_id"],
                    stimulus.unwrap_or_default(),
                    sample_rate.unwrap_or_default(),
                    json_python_separators(settings),
                    band
                );
                t60_groups.push((key, source.to_string(), views["octave_t60"].clone()));
            }
        }
        if !views["early_reflections"].is_null() {
            match capture_early_reflections_html(&views["early_reflections"]) {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
        }
        if !views["early_late_curves"].is_null() {
            match capture_early_late_html(&views["early_late_curves"]) {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
        }
        if !views["waterfall"].is_null() {
            match capture_waterfall_html(&views["waterfall"]) {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
        }
        if !views["wavelet"].is_null() {
            match capture_wavelet_html(&views["wavelet"]) {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
        }
        if !views["ambient_noise"].is_null() {
            match capture_noise_html(&views["ambient_noise"], graph) {
                Ok(html) => parts.push(html),
                Err(reason) => return unavailable(&reason),
            }
        }
        let unavailable_rows = views["unavailable"].as_object();
        let Some(unavailable_rows) = unavailable_rows else {
            return unavailable("'unavailable'");
        };
        parts.push("<ul>".into());
        for (key, reason) in unavailable_rows {
            let Some(reason) = reason.as_str() else {
                return unavailable("invalid capture unavailability reason");
            };
            parts.push(format!(
                "<li>{} unavailable: {}</li>",
                escape_html(key),
                escape_html(reason)
            ));
        }
        parts.push("</ul>".into());
    }
    parts.push(capture_room_mean_t60_html(&t60_groups));
    parts.push("</section>".into());
    parts.concat()
}

fn capture_trace_figure(
    view: &Value,
    prefix: &str,
    units: &str,
    title: &str,
) -> Result<String, String> {
    let times = view["times_ms"]
        .as_array()
        .ok_or("missing raw trace arrays")?;
    let before = view[format!("pre_{prefix}")]
        .as_array()
        .ok_or("missing raw trace arrays")?;
    let after = view[format!("post_{prefix}")]
        .as_array()
        .ok_or("missing raw trace arrays")?;
    if times.len() < 2
        || times.len() != before.len()
        || times.len() != after.len()
        || times.len() > 65_536
    {
        return Err("unaligned or oversized raw traces".into());
    }
    let Some(x) = finite_values(times) else {
        return Err("invalid raw trace value".into());
    };
    let Some(y0) = finite_values(before) else {
        return Err("invalid raw trace value".into());
    };
    let Some(y1) = finite_values(after) else {
        return Err("invalid raw trace value".into());
    };
    if x[0] != 0.0 || x.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("invalid common time axis".into());
    }
    let title_json = json!({"kind":"figure","figure":{
        "title":title,
        "x":{"label":"Time (ms)","scale":"linear","min":0,"max":times[times.len()-1]},
        "y":{"label":units,"scale":"linear","min":null,"max":null},
        "y2":null,
        "series":[
            {"name":"Baseline","x":x,"y":y0,"color":"#2864b4","width":2.0,"dash":"solid","visible":true,"y_axis":0},
            {"name":"Candidate","x":x,"y":y1,"color":"#c56816","width":2.0,"dash":"solid","visible":true,"y_axis":0}
        ],"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true
    },"tab":null});
    Ok(embedded_report_figure(&title_json))
}

fn capture_early_reflections_html(view: &Value) -> Result<String, String> {
    if view["method"] != "bandlimited_early_reflection_table_v1"
        || !numeric_array_equals(&view["band_hz"], &[1000.0, 8000.0])
        || finite(&view["threshold_dbfs"]) != Some(-15.0)
        || !["pre_direct_sample", "post_direct_sample"]
            .into_iter()
            .all(|key| view[key].as_u64().is_some())
    {
        return Err("invalid early reflection analysis contract".into());
    }
    let scope = view["scope"]
        .as_str()
        .ok_or("invalid early reflection analysis contract")?;
    let direct_reference = view["direct_reference"]
        .as_str()
        .ok_or("invalid early reflection analysis contract")?;
    let mut parts = vec![
        "<h3>Early reflection candidates (1–8 kHz)</h3>".to_string(),
        format!("<p>{}</p>", escape_html(scope)),
        format!(
            "<p>{}; reflections below −15 dB relative to direct are omitted. Filtered direct samples: baseline {}, candidate {}; table times are relative to each direct peak.</p>",
            escape_html(direct_reference),
            view["pre_direct_sample"].as_u64().unwrap_or_default(),
            view["post_direct_sample"].as_u64().unwrap_or_default()
        ),
    ];
    for (key, label, color) in [
        ("pre", "Baseline", "#2864b4"),
        ("post", "Candidate", "#c56816"),
    ] {
        let events = view[key]
            .as_array()
            .filter(|events| events.len() <= 64)
            .ok_or("invalid early reflection event list")?;
        let mut last_time = 0.0;
        let mut x = Vec::new();
        let mut y = Vec::new();
        for event in events {
            if !event.is_object() {
                return Err("invalid early reflection event".into());
            }
            let gain = finite(&event["gain_dbfs"]).ok_or("invalid early reflection values")?;
            let time = finite(&event["time_ms"]).ok_or("invalid early reflection values")?;
            let distance =
                finite(&event["distance_cm"]).ok_or("invalid early reflection values")?;
            let first_dip =
                finite(&event["first_dip_hz"]).ok_or("invalid early reflection values")?;
            let ripple = if event["ripple_db"].is_null() {
                None
            } else {
                Some(finite(&event["ripple_db"]).ok_or("invalid early reflection values")?)
            };
            if time <= last_time
                || time > 15.0
                || gain < -15.0
                || distance <= 0.0
                || first_dip <= 0.0
                || ripple.is_some_and(|value| value < 0.0)
            {
                return Err("invalid early reflection values".into());
            }
            last_time = time;
            x.extend([time, time, time]);
            y.extend([json!(-40.0), json!(gain), Value::Null]);
        }
        parts.push(format!("<h4>{label} ({} candidates)</h4>", events.len()));
        if !events.is_empty() {
            let figure = json!({
                "kind":"figure","figure":{
                    "title":format!("{label}: early-reflection candidates"),
                    "x":{"label":"Time after direct peak (ms)","scale":"linear","min":0,"max":15},
                    "y":{"label":"Level relative to direct (dB)","scale":"linear","min":-40,"max":3},
                    "y2":null,
                    "series":[{"name":label,"x":x,"y":y,"color":color,"width":2.0,"dash":"solid","visible":true,"y_axis":0}],
                    "hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true
                },"tab":null
            });
            parts.push(embedded_report_figure(&figure));
        }
        parts.push(
            "<table><tr><th>Reflection</th><th>Gain (dB re direct)</th><th>Time (ms)</th><th>Extra path (cm)</th><th>First dip (Hz)</th><th>Comb ripple (dB p-p)</th></tr>".into(),
        );
        for (index, event) in events.iter().enumerate() {
            let ripple = finite(&event["ripple_db"])
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "unbounded".into());
            let gain = finite(&event["gain_dbfs"]).unwrap_or_default();
            let time = finite(&event["time_ms"]).unwrap_or_default();
            let distance = finite(&event["distance_cm"]).unwrap_or_default();
            let dip = finite(&event["first_dip_hz"]).unwrap_or_default();
            parts.push(format!(
                "<tr><td>{}</td><td>{gain:.2}</td><td>{time:.2}</td><td>{distance:.1}</td><td>{dip:.1}</td><td>{ripple}</td></tr>",
                index + 1
            ));
        }
        parts.push("</table>".into());
    }
    Ok(parts.concat())
}

fn capture_early_late_html(view: &Value) -> Result<String, String> {
    let band = view["valid_band_hz"].as_array();
    if view["method"] != "incoherent_band_energy"
        || view["reference"] != "full_peak_band"
        || view["smoothing"] != "third_octave"
        || finite(&view["split_ms"]) != Some(20.0)
        || !matches!(
            view["direct_reference"].as_str(),
            Some("broadband envelope peak" | "120 Hz lowpass envelope peak")
        )
        || band.is_none_or(|band| {
            band.len() != 2
                || finite(&band[0]).is_none_or(|value| value <= 0.0)
                || finite(&band[1]).is_none_or(|value| value <= finite(&band[0]).unwrap_or(0.0))
        })
        || view["scope"].as_str().is_none_or(str::is_empty)
    {
        return Err("invalid capture early/late analysis contract".into());
    }
    let band = band.unwrap();
    let scope = view["scope"].as_str().unwrap_or_default();
    let direct_reference = view["direct_reference"].as_str().unwrap_or_default();
    let mut common_freq: Option<Vec<f64>> = None;
    let mut parts = vec![
        "<h3>Captured IR early and late energy</h3>".to_string(),
        format!("<p>{}</p>", escape_html(scope)),
        format!(
            "<p>20 ms split from {}; full, early, and late use one peak-band reference within each capture. Baseline and candidate levels have separate references.</p>",
            escape_html(direct_reference)
        ),
    ];
    for (side, label) in [("pre", "Baseline"), ("post", "Candidate")] {
        let record = &view[side];
        if record["direct_sample"].as_u64().is_none() {
            return Err("invalid capture early/late direct sample".into());
        }
        let curves = ["full", "early", "late"].map(|key| &record[key]);
        if curves.iter().any(|curve| !curve.is_object()) {
            return Err("missing capture early/late curves".into());
        }
        let frequencies = curves[0]["freq"]
            .as_array()
            .ok_or("invalid capture early/late band support")?;
        if frequencies.len() < 2 || frequencies.len() > 21 {
            return Err("invalid capture early/late band support".into());
        }
        let freq = finite_values(frequencies).ok_or("invalid capture early/late band support")?;
        if freq.iter().any(|value| *value <= 0.0)
            || freq.windows(2).any(|pair| pair[0] >= pair[1])
            || freq.iter().any(|value| {
                value / 2.0_f64.powf(1.0 / 6.0) < finite(&band[0]).unwrap_or_default() - 1e-6
                    || value * 2.0_f64.powf(1.0 / 6.0) > finite(&band[1]).unwrap_or_default() + 1e-6
            })
            || common_freq.as_ref().is_some_and(|old| old != &freq)
        {
            return Err("invalid capture early/late band support".into());
        }
        common_freq = Some(freq.clone());
        let mut levels = Vec::new();
        for curve in curves {
            if curve["freq"] != Value::Array(frequencies.clone()) {
                return Err("invalid capture early/late levels".into());
            }
            let values = curve["spl"]
                .as_array()
                .ok_or("invalid capture early/late levels")?;
            let values = finite_values(values).ok_or("invalid capture early/late levels")?;
            if values.len() != freq.len()
                || values.iter().any(|value| !(-120.01..=0.01).contains(value))
            {
                return Err("invalid capture early/late levels".into());
            }
            levels.push(values);
        }
        if levels[0]
            .iter()
            .zip(&levels[1])
            .any(|(full, early)| *full + 1e-6 < *early)
            || levels[0]
                .iter()
                .zip(&levels[2])
                .any(|(full, late)| *full + 1e-6 < *late)
        {
            return Err(
                "capture early/late full level below a part: incoherent energy sum violated".into(),
            );
        }
        parts.push(format!(
            "<h4>{label}</h4><p>Direct sample: {}.</p>",
            record["direct_sample"].as_u64().unwrap_or_default()
        ));
        let colors = ["#202020", "#2864b4", "#c56816"];
        let names = ["Full", "Early", "Late"];
        let series = names.iter().enumerate().map(|(index, name)| json!({
            "name":name,"x":freq,"y":levels[index],"color":colors[index],"width":2.0,"dash":"solid","visible":true,"y_axis":0
        })).collect::<Vec<_>>();
        let figure = json!({"kind":"figure","figure":{
            "title":format!("{label}: early vs late energy"),
            "x":{"label":"Frequency (Hz)","scale":"log","min":null,"max":null},
            "y":{"label":"Level vs full peak band (dB)","scale":"linear","min":null,"max":null},"y2":null,
            "series":series,"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true
        },"tab":null});
        parts.push(embedded_report_figure(&figure));
        let early_late = freq
            .iter()
            .zip(&levels[1])
            .zip(&levels[2])
            .filter_map(|((frequency, early), late)| {
                (1000.0..=8000.0)
                    .contains(frequency)
                    .then_some(early - late)
            })
            .collect::<Vec<_>>();
        if freq[0] <= 1000.0 && *freq.last().unwrap_or(&0.0) >= 8000.0 && early_late.len() >= 2 {
            let ratio = early_late.iter().sum::<f64>() / early_late.len() as f64;
            if ratio.is_finite() {
                parts.push(format!("<p>1–8 kHz mean early minus late: {ratio:+.2} dB (arithmetic mean of third-octave differences).</p>"));
            }
        } else {
            parts.push(
                "<p>1–8 kHz early/late ratio unavailable: full band support is missing.</p>".into(),
            );
        }
    }
    Ok(parts.concat())
}

fn capture_octave_t60_html(view: &Value) -> Result<String, String> {
    let centers = [63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    let band = view["valid_band_hz"]
        .as_array()
        .filter(|values| values.len() == 2)
        .ok_or("invalid octave T60 analysis contract")?;
    let lo = finite(&band[0]).ok_or("invalid octave T60 analysis contract")?;
    let hi = finite(&band[1]).ok_or("invalid octave T60 analysis contract")?;
    let threshold = finite(&view["min_r2"]).ok_or("invalid octave T60 analysis contract")?;
    if view["method"] != "octave_schroeder_t30_t20_v1"
        || !(0.0..=1.0).contains(&threshold)
        || threshold == 0.0
        || lo <= 0.0
        || hi <= lo
    {
        return Err("invalid octave T60 analysis contract".into());
    }
    for side in ["pre", "post"] {
        let rows = view[side]
            .as_array()
            .filter(|rows| rows.len() == centers.len())
            .ok_or("incomplete octave T60 bands")?;
        for (center, row) in centers.iter().zip(rows) {
            let valid = row["valid"]
                .as_bool()
                .ok_or("invalid octave T60 fit quality")?;
            let r2 = finite(&row["r2"]).ok_or("invalid octave T60 fit quality")?;
            let value = finite(&row["t60_s"]);
            let reason = row["reason"].as_str();
            if row["centre_hz"].as_i64() != Some(*center as i64)
                || !(0.0..=1.0).contains(&r2)
                || (valid
                    && (value.is_none_or(|value| value <= 0.0)
                        || !matches!(row["fit_range"].as_str(), Some("T20" | "T30"))
                        || r2 < threshold
                        || reason.is_none_or(|reason| !reason.is_empty())))
                || (!valid && (!row["t60_s"].is_null() || reason.is_none_or(str::is_empty)))
            {
                return Err(if valid {
                    "invalid accepted octave T60 fit"
                } else {
                    "invalid rejected octave T60 fit"
                }
                .into());
            }
        }
    }
    let scope = view["scope"]
        .as_str()
        .ok_or("invalid octave T60 analysis contract")?;
    let mut html = format!(
        "<h3>Capture IR octave T60 diagnostic</h3><p>{}</p><p>Declared supported band: {}–{} Hz; minimum fit R²: {}. No invalid or EDT-only band is plotted; gaps are unavailable fits.</p>",
        escape_html(scope),
        python_number(&band[0]),
        python_number(&band[1]),
        python_number(&view["min_r2"])
    );
    let valid_values = ["pre", "post"].iter().any(|side| {
        view[*side]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["valid"] == true))
    });
    if valid_values {
        let mut series = Vec::new();
        for (side, label, color) in [
            ("pre", "Baseline", "#3089c5"),
            ("post", "Candidate", "#c56816"),
        ] {
            let rows = view[side].as_array().unwrap();
            let octave_edges = (0..centers.len())
                .map(|index| {
                    if index == 0 {
                        centers[index] as f64 / 2.0_f64.sqrt()
                    } else {
                        ((centers[index - 1] * centers[index]) as f64).sqrt()
                    }
                })
                .chain(std::iter::once(
                    centers[centers.len() - 1] as f64 * 2.0_f64.sqrt(),
                ))
                .collect::<Vec<_>>();
            let mut x = Vec::with_capacity(rows.len() * 2);
            let mut y = Vec::with_capacity(rows.len() * 2);
            for (index, row) in rows.iter().enumerate() {
                let value = if row["valid"] == true {
                    row["t60_s"].clone()
                } else {
                    Value::Null
                };
                x.extend([json!(octave_edges[index]), json!(octave_edges[index + 1])]);
                y.extend([value.clone(), value]);
            }
            series.push(json!({"name":label,"x":x,"y":y,"color":color,"width":2.0,"dash":"solid","visible":true,"y_axis":0}));
        }
        html.push_str(&embedded_report_figure(&json!({"kind":"figure","figure":{"title":"Baseline: measured octave-band T60","x":{"label":"Frequency (Hz)","scale":"log","min":20,"max":20000},"y":{"label":"T60 (s)","scale":"linear","min":0,"max":null},"y2":null,"series":series,"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true},"tab":null})));
    }
    html.push_str("<table><caption>Per-band T30/T20 estimates from matched IR captures</caption><tr><th>Hz</th><th>Baseline T60 (s)</th><th>Candidate T60 (s)</th><th>Baseline fit</th><th>Candidate fit</th></tr>");
    for (index, center) in centers.iter().enumerate() {
        let pre = &view["pre"][index];
        let post = &view["post"][index];
        let cell = |row: &Value| {
            if row["valid"] == true {
                format!("{:.3}", finite(&row["t60_s"]).unwrap_or_default())
            } else {
                format!(
                    "n/a: {}",
                    escape_html(row["reason"].as_str().unwrap_or_default())
                )
            }
        };
        html.push_str(&format!("<tr><td>{center}</td><td>{}</td><td>{}</td><td>{}; R²={:.3}</td><td>{}; R²={:.3}</td></tr>", cell(pre), cell(post), escape_html(pre["fit_range"].as_str().unwrap_or("n/a")), finite(&pre["r2"]).unwrap_or_default(), escape_html(post["fit_range"].as_str().unwrap_or("n/a")), finite(&post["r2"]).unwrap_or_default()));
    }
    html.push_str("</table>");
    Ok(html)
}

fn capture_room_mean_t60_html(groups: &[(String, String, Value)]) -> String {
    let mut html = String::new();
    let mut keys = Vec::<String>::new();
    for (key, _, _) in groups {
        if !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    let centers = [63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    for key in keys {
        let entries = groups
            .iter()
            .filter(|(candidate, _, _)| *candidate == key)
            .collect::<Vec<_>>();
        let sources = entries
            .iter()
            .map(|(_, source, _)| source.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if sources.len() < 2 || sources.len() != entries.len() {
            continue;
        }
        let first = &entries[0].2;
        let mut averages = [[(None, 0usize); 9]; 2];
        for (side_index, average_side) in averages.iter_mut().enumerate() {
            let side = if side_index == 0 { "pre" } else { "post" };
            for band_index in 0..9 {
                let values = entries
                    .iter()
                    .filter_map(|(_, _, view)| {
                        let row = &view[side][band_index];
                        (row["valid"] == true)
                            .then(|| finite(&row["t60_s"]))
                            .flatten()
                    })
                    .collect::<Vec<_>>();
                if !values.is_empty() {
                    let mean = values.iter().map(|v| *v / values.len() as f64).sum::<f64>();
                    average_side[band_index] = (Some(mean), values.len());
                }
            }
        }
        let seat = key.split('|').next().unwrap_or_default();
        let kind = key.split('|').nth(1).unwrap_or_default();
        let rate = key.split('|').nth(4).unwrap_or("0");
        html.push_str(&format!("<h3>Capture room mean octave T60: {}</h3><p>{}; {} distinct sources with matching declared settings, {} Hz sample rate, and capture band ({}). Means include accepted T30/T20 fits only. Counts vary by band. This advisory average does not establish a passive-room damping change.</p>", escape_html(seat.trim_matches('"')), escape_html(kind.trim_matches('"')), sources.len(), rate, escape_html(&sources.iter().cloned().collect::<Vec<_>>().join(", "))));
        if averages.iter().flatten().any(|(value, _)| value.is_some()) {
            let mut series = Vec::new();
            for (side_index, label, color) in
                [(0, "Baseline", "#2864b4"), (1, "Candidate", "#c56816")]
            {
                series.push(json!({"name":label,"x":centers,"y":averages[side_index].iter().map(|(v,_)| v.map(Value::from).unwrap_or(Value::Null)).collect::<Vec<_>>(),"color":color,"width":2.0,"dash":"solid","visible":true,"y_axis":0}));
            }
            html.push_str(&embedded_report_figure(&json!({"kind":"figure","figure":{"title":"Capture room mean octave T60","x":{"label":"Frequency (Hz)","scale":"log","min":null,"max":null},"y":{"label":"T60 (s)","scale":"linear","min":null,"max":null},"y2":null,"series":series,"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true},"tab":null})));
        }
        html.push_str("<table><caption>Accepted source fits; n is the number of contributing sources</caption><tr><th>Hz</th><th>Baseline mean (s)</th><th>Baseline n</th><th>Candidate mean (s)</th><th>Candidate n</th></tr>");
        for (index, center) in centers.iter().enumerate() {
            let (pre, pre_n) = averages[0][index];
            let (post, post_n) = averages[1][index];
            let pre = pre
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "n/a".into());
            let post = post
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "n/a".into());
            html.push_str(&format!("<tr><td>{center}</td><td>{pre}</td><td>{pre_n}</td><td>{post}</td><td>{post_n}</td></tr>"));
        }
        html.push_str("</table>");
        let _ = first;
    }
    html
}

fn capture_decay_html(view: &Value, graph: &str) -> Result<String, String> {
    if view["provenance"]["graph_identity"] != graph
        || view["method"] != "finite_window_octave_schroeder_v1"
    {
        return Err("decay graph identity or analysis method mismatch".into());
    }
    let bands = view["bands"]
        .as_array()
        .filter(|bands| !bands.is_empty())
        .ok_or("missing decay bands")?;
    let scope = view["scope"].as_str().ok_or("invalid decay scope")?;
    let mut html = format!(
        "<h3>Matched octave decay</h3><p>{}</p><p>Finite-window T20 slope extrapolation, not passive-room RT. Noise-window and fit budgets are operator declarations: {}</p>",
        escape_html(scope),
        escape_html(&python_repr(&view["settings"]))
    );
    for band in bands {
        let center = finite(&band["center_hz"]).ok_or("unsupported decay octave")?;
        if ![63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0].contains(&center) {
            return Err("unsupported decay octave".into());
        }
        let trace = json!({"times_ms":band["times_ms"],"pre_decay":band["pre_db"],"post_decay":band["post_db"]});
        html.push_str(&capture_trace_figure(
            &trace,
            "decay",
            "dB relative to the same baseline-band integrated energy",
            &format!(
                "Common-reference decay {} Hz",
                python_number(&band["center_hz"])
            ),
        )?);
        if !band["post_normalized_db"].is_null() {
            let normalized = json!({"times_ms":band["times_ms"],"pre_decay":band["pre_normalized_db"],"post_decay":band["post_normalized_db"]});
            html.push_str(&capture_trace_figure(
                &normalized,
                "decay",
                "dB relative to each capture’s own initial band energy",
                &format!("Normalized decay {} Hz", python_number(&band["center_hz"])),
            )?);
        } else {
            html.push_str(
                "<p>Candidate normalized decay unavailable: no nonzero energy reference.</p>",
            );
        }
        for prefix in ["pre", "post"] {
            let fit = &band[format!("{prefix}_fit")];
            if fit.is_null() {
                html.push_str(&format!(
                    "<p>{prefix} fit unavailable: {}</p>",
                    escape_html(
                        band[format!("{prefix}_fit_unavailable")]
                            .as_str()
                            .unwrap_or("unspecified")
                    )
                ));
            } else {
                let seconds = finite(&fit["extrapolated_60_db_seconds"])
                    .filter(|v| *v > 0.0)
                    .ok_or("invalid decay fit")?;
                let r2 = finite(&fit["r_squared"])
                    .filter(|v| (0.0..=1.0).contains(v))
                    .ok_or("invalid decay fit")?;
                html.push_str(&format!("<p>{prefix} finite-window extrapolation: {seconds:.4} s; R²={r2:.4}; fit diagnostics: {}</p>", escape_html(&python_json(fit))));
            }
        }
    }
    Ok(html)
}

fn capture_waterfall_html(view: &Value) -> Result<String, String> {
    if view["method"] != "hann_stft_waterfall_v1"
        || view["reference"] != "each_full_grid_peak"
        || finite(&view["window_ms"]) != Some(32.0)
        || finite(&view["hop_ms"]) != Some(2.0)
        || finite(&view["post_ms"]) != Some(500.0)
    {
        return Err("invalid capture waterfall contract".into());
    }
    let band = view["valid_band_hz"]
        .as_array()
        .filter(|v| v.len() == 2)
        .ok_or("invalid capture waterfall contract")?;
    let lo = finite(&band[0]).ok_or("invalid capture waterfall contract")?;
    let hi = finite(&band[1]).ok_or("invalid capture waterfall contract")?;
    if lo <= 0.0 || hi <= lo {
        return Err("invalid capture waterfall contract".into());
    }
    let scope = view["scope"]
        .as_str()
        .ok_or("invalid capture waterfall scope")?;
    let mut html = format!(
        "<h3>Captured IR waterfall and resonance decay</h3><p>{}</p><p>Declared supported band: {}–{} Hz. Each grid is referenced to its own full-grid peak; colors and levels do not compare absolute output between captures.</p>",
        escape_html(scope),
        python_number(&band[0]),
        python_number(&band[1])
    );
    for (side, label) in [("pre", "Baseline"), ("post", "Candidate")] {
        let grid = &view[side]["grid"];
        let times = grid["times_ms"]
            .as_array()
            .ok_or("invalid capture waterfall samples")?;
        let freqs = grid["freqs_hz"]
            .as_array()
            .ok_or("invalid capture waterfall samples")?;
        let rows = grid["mags_db"]
            .as_array()
            .ok_or("invalid capture waterfall samples")?;
        if times.len() < 2
            || times.len() > 100
            || freqs.len() < 2
            || freqs.len() > 64
            || rows.len() != times.len()
        {
            return Err("invalid capture waterfall samples".into());
        }
        let t = finite_values(times).ok_or("invalid capture waterfall samples")?;
        let f = finite_values(freqs).ok_or("invalid capture waterfall samples")?;
        let mut z = Vec::new();
        for row in rows {
            let values = row
                .as_array()
                .filter(|v| v.len() == f.len())
                .and_then(|v| finite_values(v))
                .ok_or("invalid capture waterfall samples")?;
            if values.iter().any(|v| !(-100.01..=0.01).contains(v)) {
                return Err("invalid capture waterfall samples".into());
            }
            z.push(row.clone());
        }
        if t.windows(2).any(|p| p[0] >= p[1])
            || f.windows(2).any(|p| p[0] >= p[1])
            || t[0] < -5.01
            || *t.last().unwrap() > 500.01
            || f.iter().any(|v| *v < lo || *v > hi)
        {
            return Err("invalid capture waterfall samples".into());
        }
        let resonances = view[side]["resonances"]
            .as_array()
            .filter(|v| v.len() <= 64)
            .ok_or("invalid capture resonance list")?;
        for r in resonances {
            let rf = finite(&r["freq_hz"]).ok_or("invalid capture resonance values")?;
            let level = finite(&r["level_db"]).ok_or("invalid capture resonance values")?;
            if rf < f[0]
                || rf > *f.last().unwrap()
                || !(-100.01..=0.01).contains(&level)
                || (!r["decay_time_s"].is_null()
                    && finite(&r["decay_time_s"]).is_none_or(|v| v <= 0.0))
            {
                return Err("invalid capture resonance values".into());
            }
        }
        let mut ranked = resonances.iter().enumerate().collect::<Vec<_>>();
        ranked.sort_by(|(_, left), (_, right)| {
            let rank = |row: &Value| {
                (
                    !row["decay_time_s"].is_null(),
                    finite(&row["decay_time_s"]).unwrap_or(0.0),
                    finite(&row["level_db"]).unwrap_or(0.0),
                )
            };
            let left = rank(left);
            let right = rank(right);
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.total_cmp(&left.1))
                .then_with(|| right.2.total_cmp(&left.2))
        });
        let highlights = ranked
            .iter()
            .take(2)
            .filter_map(|(_, row)| {
                let frequency = finite(&row["freq_hz"])?;
                f.iter()
                    .enumerate()
                    .min_by(|(_, left), (_, right)| {
                        (*left - frequency)
                            .abs()
                            .total_cmp(&(*right - frequency).abs())
                    })
                    .map(|(index, _)| json!(index))
            })
            .collect::<Vec<_>>();
        let highlight_series = ranked.iter().take(2).map(|(_, row)| {
            let frequency = finite(&row["freq_hz"]).unwrap_or_default();
            json!({"name":format!("{frequency:.1} Hz"),"x":[],"y":[],"color":resonance_color(frequency),"width":2.0,"dash":"solid","visible":true,"y_axis":0})
        }).collect::<Vec<_>>();
        let grid_figure = json!({
            "kind":"grid",
            "figure":{
                "title":"Captured IR waterfall",
                "x":{"label":"Frequency (Hz)","scale":"log","min":freqs[0],"max":if *f.last().unwrap_or(&20000.0) > 20000.0 { json!(20000) } else { freqs[freqs.len()-1].clone() }},
                "y":{"label":"Time from broadband peak (ms)","scale":"linear","min":if t[0] < -1.0 { json!(-1) } else { times[0].clone() },"max":times[times.len()-1]},
                "y2":null,"series":highlight_series,"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true
            },
            "grid":{"x":freqs,"y":times,"z":z,"surface":true,"zmin":-60,"zmax":0,"highlights":highlights,"colormap":"turbo","show_surface":false,"show_contours":true},
            "tab":null
        });
        html.push_str(&format!("<h4>{label}</h4>{}<table><caption>Detected 60 ms peaks and fitted 20–200 ms decay</caption><tr><th>Frequency (Hz)</th><th>Level (dB re own grid peak)</th><th>Fitted decay (s)</th></tr>",embedded_report_figure(&grid_figure)));
        for r in resonances {
            let freq = finite(&r["freq_hz"]).unwrap_or_default();
            let level = finite(&r["level_db"]).unwrap_or_default();
            let decay = finite(&r["decay_time_s"])
                .map(|v| python_general(v, 3))
                .unwrap_or_else(|| "unavailable".into());
            html.push_str(&format!(
                "<tr><td>{}</td><td>{level:.2}</td><td>{decay}</td></tr>",
                python_general(freq, 3)
            ));
        }
        html.push_str("</table>");
        if !ranked.is_empty() {
            html.push_str("<p>Highlighted: up to two longest fitted decays, then strongest 60 ms peak. Curves retain the full-grid peak reference; no fitted intercept is exported, so no extrapolated fit line is invented.</p>");
            let selected = ranked.iter().take(2).map(|(_, row)| {
                let frequency = finite(&row["freq_hz"]).unwrap_or_default();
                let label = finite(&row["decay_time_s"])
                    .map(|decay| format!("{frequency:.1} Hz — T60 {decay:.3} s"))
                    .unwrap_or_else(|| format!("{frequency:.1} Hz"));
                let color = resonance_color(frequency);
                let index = f.iter().enumerate().min_by(|(_, left), (_, right)| {
                    (*left - frequency).abs().total_cmp(&(*right - frequency).abs())
                }).map(|(index, _)| index).unwrap_or_default();
                let values = z
                    .iter()
                    .map(|row| json!(finite(&row[index]).unwrap_or_default()))
                    .collect::<Vec<_>>();
                json!({"name":label,"x":t,"y":values,"color":color,"width":2.0,"dash":"solid","visible":true,"y_axis":0})
            }).collect::<Vec<_>>();
            let figure = json!({"kind":"figure","figure":{"title":format!("{label}: decay of highlighted resonances"),"x":{"label":"Time from broadband peak (ms)","scale":"linear","min":0,"max":500},"y":{"label":"Level relative to full grid peak (dB)","scale":"linear","min":-60,"max":0},"y2":null,"series":selected,"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true},"tab":null});
            html.push_str(&embedded_report_figure(&figure));
        }
    }
    Ok(html)
}

fn capture_wavelet_html(view: &Value) -> Result<String, String> {
    if view["method"] != "complex_morlet_three_cycle_v1"
        || view["reference"] != "each_full_grid_peak"
        || finite(&view["cycles"]) != Some(3.0)
        || finite(&view["freqs_per_octave"]) != Some(6.0)
        || finite(&view["hop_ms"]) != Some(1.0)
        || !numeric_array_equals(&view["display_range_db"], &[-30.0, 0.0])
    {
        return Err("invalid capture wavelet contract".into());
    }
    let band = view["valid_band_hz"]
        .as_array()
        .filter(|v| v.len() == 2)
        .ok_or("invalid capture wavelet contract")?;
    let lo = finite(&band[0]).ok_or("invalid capture wavelet contract")?;
    let hi = finite(&band[1]).ok_or("invalid capture wavelet contract")?;
    if lo <= 0.0 || hi <= lo {
        return Err("invalid capture wavelet contract".into());
    }
    let scope = view["scope"]
        .as_str()
        .ok_or("invalid capture wavelet scope")?;
    let mut html = format!(
        "<h3>Captured IR three-cycle wavelet</h3><p>{}</p><p>Blue = −30 dB, red = 0 dB relative to each capture’s own full wavelet-grid peak. Colors do not compare absolute output levels.</p>",
        escape_html(scope)
    );
    for (side, _label) in [("pre", "Baseline"), ("post", "Candidate")] {
        let grid = &view[side];
        let freqs = grid["freqs_hz"]
            .as_array()
            .ok_or("invalid capture wavelet samples")?;
        let times = grid["times_ms"]
            .as_array()
            .ok_or("invalid capture wavelet samples")?;
        let rows = grid["mags_db"]
            .as_array()
            .ok_or("invalid capture wavelet samples")?;
        if freqs.len() < 2
            || freqs.len() > 64
            || times.len() < 2
            || times.len() > 100
            || rows.len() != freqs.len()
        {
            return Err("invalid capture wavelet samples".into());
        }
        let f = finite_values(freqs).ok_or("invalid capture wavelet samples")?;
        let t = finite_values(times).ok_or("invalid capture wavelet samples")?;
        let mut z = Vec::new();
        for row in rows {
            let v = row
                .as_array()
                .filter(|v| v.len() == t.len())
                .and_then(|v| finite_values(v))
                .ok_or("invalid capture wavelet samples")?;
            if v.iter().any(|x| !(-30.01..=0.01).contains(x)) {
                return Err("invalid capture wavelet samples".into());
            }
            z.push(v);
        }
        if f.windows(2).any(|p| p[0] >= p[1])
            || t.windows(2).any(|p| p[0] >= p[1])
            || f.iter().any(|v| *v < lo || *v > hi)
            || t[0] < -5.01
            || *t.last().unwrap() > 500.01
        {
            return Err("invalid capture wavelet samples".into());
        }
        let early_count = t
            .iter()
            .filter(|time| **time >= -1.0 && **time <= 15.0)
            .count();
        html.push_str(&format!("<p>Three-cycle wavelet; first 15 ms ({early_count} exported time samples). Log-frequency x-axis, time y-axis. Colours retain the full-grid peak reference. The cyan dashed ridge marks peak energy arrival time per frequency. Display cells reflect the exported sampling; zoom cannot restore omitted samples.</p>"));
        let time_major = (0..times.len())
            .map(|time| {
                (0..freqs.len())
                    .map(|frequency| rows[frequency][time].clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let grid_figure = json!({
            "kind":"grid",
            "figure":{
                "title":"Captured three-cycle wavelet heatmap",
                "x":{"label":"Frequency (Hz)","scale":"log","min":freqs[0],"max":freqs[freqs.len()-1]},
                "y":{"label":"Time from broadband peak (ms)","scale":"linear","min":if t[0] < -1.0 { json!(-1) } else { times[0].clone() },"max":15},
                "y2":null,"series":[],"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true
            },
            "grid":{"x":freqs,"y":times,"z":time_major,"surface":false,"zmin":-30,"zmax":0,"highlights":[]},
            "tab":null
        });
        html.push_str(&embedded_report_figure(&grid_figure));
    }
    Ok(html)
}

fn capture_noise_html(view: &Value, graph: &str) -> Result<String, String> {
    let analysis = &view["analysis"];
    if analysis["method"] != "periodic_hann_welch_pressure_v1"
        || analysis["spectrum"]["provenance"]["graph_identity"] != graph
        || !matches!(
            view["evidence_kind"].as_str(),
            Some("synthetic_noise_capture" | "operator_declared_recorded_noise_capture")
        )
    {
        return Err("noise graph, evidence kind, or analysis method mismatch".into());
    }
    let freqs = analysis["bin_freqs_hz"]
        .as_array()
        .ok_or("invalid calibrated noise PSD")?;
    let powers = analysis["pressure_psd_pa2_per_hz"]
        .as_array()
        .ok_or("invalid calibrated noise PSD")?;
    if freqs.is_empty() || freqs.len() != powers.len() || freqs.len() > 32768 {
        return Err("invalid calibrated noise PSD".into());
    }
    let x = finite_values(freqs).ok_or("invalid calibrated noise PSD")?;
    let p = finite_values(powers).ok_or("invalid calibrated noise PSD")?;
    if x.iter().any(|v| *v <= 0.0)
        || x.windows(2).any(|w| w[0] >= w[1])
        || p.iter().any(|v| *v < 0.0)
    {
        return Err("invalid calibrated noise PSD".into());
    }
    let kind = view["evidence_kind"]
        .as_str()
        .ok_or("invalid calibrated noise evidence")?;
    let conditions = view["conditions"]
        .as_str()
        .ok_or("invalid calibrated noise conditions")?;
    let scope = analysis["scope"]
        .as_str()
        .ok_or("invalid calibrated noise scope")?;
    let mut html = format!(
        "<h3>Calibrated ambient noise</h3><p>{}; {}</p><p>{}</p><p>Observation: {} s; bin spacing: {} Hz; Hann ENBW: {} Hz. Bin spacing is not resolving power. No self-noise subtraction.</p><details><summary>Numeric calibration and acquisition limitations</summary><pre>{}</pre></details>",
        escape_html(kind),
        escape_html(conditions),
        escape_html(scope),
        escape_html(&python_json(&analysis["duration_seconds"])),
        escape_html(&python_json(&analysis["bin_spacing_hz"])),
        escape_html(&python_json(&analysis["window_enbw_hz"])),
        escape_html(&serde_json::to_string_pretty(&analysis["calibration"]).unwrap_or_default())
    );
    let levels = p
        .iter()
        .map(|v| {
            if *v > 0.0 {
                Some(10.0 * (v.log10() - (20e-6_f64.powi(2)).log10()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let positive = levels.iter().any(Option::is_some);
    if positive {
        let figure = json!({"kind":"figure","figure":{"title":"Calibrated ambient pressure PSD","x":{"label":"Frequency (Hz)","scale":"log","min":null,"max":null},"y":{"label":"dB re (20 µPa)²/Hz","scale":"linear","min":null,"max":null},"y2":null,"series":[{"name":"Pressure PSD","x":x,"y":levels,"color":"#2864b4","width":2.0,"dash":"solid","visible":true,"y_axis":0}],"hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true},"tab":null});
        html.push_str(&embedded_report_figure(&figure));
    } else {
        html.push_str("<p>No finite noise SPL: all estimated supported powers are zero. This is not proof of a noiseless room.</p>");
    }
    let spectrum = &analysis["spectrum"];
    let centers = spectrum["freqs"]
        .as_array()
        .ok_or("invalid noise octave arrays")?;
    let db = spectrum["noise_spl_db"]
        .as_array()
        .ok_or("invalid noise octave arrays")?;
    let edges = analysis["octave_edges_hz"]
        .as_array()
        .ok_or("invalid noise octave arrays")?;
    let bins = analysis["octave_bin_centers_hz"]
        .as_array()
        .ok_or("invalid noise octave arrays")?;
    if centers.len() != db.len()
        || centers.len() != edges.len()
        || centers.len() != bins.len()
        || centers.len() > 10
    {
        return Err("invalid noise octave arrays".into());
    }
    html.push_str("<table><caption>FFT-bin-summed nominal octave levels, unweighted dB SPL re 20 µPa; not certified octave filters</caption><tr><th>Center (Hz)</th><th>Nominal band (Hz)</th><th>Summed bin centers (Hz)</th><th>dB SPL</th></tr>");
    for i in 0..centers.len() {
        let level = finite(&db[i]).ok_or("invalid noise octave arrays")?;
        let edge = edges[i]
            .as_array()
            .filter(|v| v.len() == 2)
            .ok_or("invalid noise octave support")?;
        let extent = bins[i]
            .as_array()
            .filter(|v| v.len() == 2)
            .ok_or("invalid noise octave support")?;
        let elo = finite(&edge[0]).ok_or("invalid noise octave support")?;
        let ehi = finite(&edge[1]).ok_or("invalid noise octave support")?;
        let blo = finite(&extent[0]).ok_or("invalid noise octave support")?;
        let bhi = finite(&extent[1]).ok_or("invalid noise octave support")?;
        if finite(&centers[i]).is_none_or(|c| c <= 0.0)
            || elo <= 0.0
            || elo >= ehi
            || blo < elo
            || blo > bhi
            || bhi >= ehi
        {
            return Err("invalid noise octave support".into());
        }
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}–{}</td><td>{}–{}</td><td>{}</td></tr>",
            python_general(finite(&centers[i]).unwrap_or_default(), 6),
            python_general(elo, 4),
            python_general(ehi, 4),
            python_general(blo, 4),
            python_general(bhi, 4),
            python_general(level, 4)
        ));
    }
    html.push_str("</table>");
    if let Some(unavailable) = analysis["unavailable_bands"].as_object() {
        for (band, reason) in unavailable {
            html.push_str(&format!(
                "<p>Noise {} Hz unavailable: {}</p>",
                escape_html(band),
                escape_html(&python_string(reason))
            ));
        }
    }
    Ok(html)
}

fn finite_values(values: &[Value]) -> Option<Vec<f64>> {
    values.iter().map(finite).collect()
}

fn numeric_array_equals(value: &Value, expected: &[f64]) -> bool {
    value.as_array().is_some_and(|values| {
        values.len() == expected.len()
            && values
                .iter()
                .zip(expected)
                .all(|(value, expected)| finite(value) == Some(*expected))
    })
}

fn python_json(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "None".into(),
        _ => value.to_string(),
    }
}

fn python_number(value: &Value) -> String {
    finite(value)
        .map(|number| {
            if number.fract() == 0.0 {
                format!("{number:.0}")
            } else {
                number.to_string()
            }
        })
        .unwrap_or_else(|| python_json(value))
}

fn python_general(value: f64, precision: usize) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let exponent = value.abs().log10().floor() as i32;
    if exponent < -4 || exponent >= precision as i32 {
        let text = format!("{:.*e}", precision.saturating_sub(1), value);
        if let Some((mantissa, exp)) = text.split_once('e') {
            let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
            let exponent = exp.parse::<i32>().unwrap_or_default();
            return format!("{mantissa}e{exponent:+03}");
        }
        text
    } else {
        let decimals = precision.saturating_sub(1 + exponent.max(0) as usize);
        let text = format!("{value:.decimals$}");
        if decimals == 0 {
            text
        } else {
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(value) => if *value { "True" } else { "False" }.into(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    python_repr(&Value::String(key.clone())),
                    python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn json_python_separators(value: &Value) -> String {
    let compact = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let mut result = String::with_capacity(compact.len() + compact.len() / 8);
    let mut in_string = false;
    let mut escaped = false;
    for character in compact.chars() {
        if in_string {
            result.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
        } else {
            if character == '"' {
                in_string = true;
            }
            result.push(character);
            if matches!(character, ',' | ':') {
                result.push(' ');
            }
        }
    }
    result
}

fn magnitude_figure_html(view: &Value, label: &str) -> Result<String, String> {
    let frequencies = view["freqs"]
        .as_array()
        .ok_or("magnitude traces are missing")?;
    let pre = view["pre_db"]
        .as_array()
        .ok_or("magnitude traces are missing")?;
    let post = view["post_db"]
        .as_array()
        .ok_or("magnitude traces are missing")?;
    if frequencies.len() < 2
        || frequencies.len() != pre.len()
        || frequencies.len() != post.len()
        || frequencies.len() > 100_000
    {
        return Err("magnitude traces are empty, unaligned, or oversized".into());
    }
    let numeric = |values: &[Value]| -> Result<Vec<f64>, String> {
        values
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| "magnitude traces contain invalid values".into())
            })
            .collect()
    };
    let x = numeric(frequencies)?;
    let before = numeric(pre)?;
    let after = numeric(post)?;
    if x[0] <= 0.0 || x.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("magnitude frequency axis is invalid".into());
    }
    let mut all_levels = before.iter().chain(&after);
    let first = all_levels
        .next()
        .copied()
        .ok_or("magnitude traces are missing")?;
    let (low, high) = all_levels.fold((first, first), |(lo, hi), value| {
        (lo.min(*value), hi.max(*value))
    });
    if !(high - low).max(1.0).is_finite() {
        return Err("magnitude range overflow".into());
    }
    let axis =
        |label: &str, scale: &str| json!({"label":label,"scale":scale,"min":null,"max":null});
    let series = |name: &str, values: &[f64], color: &str| json!({"name":name,"x":x,"y":values,"color":color,"width":2.0,"dash":"solid","visible":true,"y_axis":0});
    let figure = json!({
        "kind":"figure",
        "figure":{
            "title":label,
            "x":axis("Frequency (Hz)","log"),
            "y":axis("Magnitude (dB)","linear"),
            "y2":null,
            "series":[series("Before",&before,"#2864b4"),series("Retained prediction after",&after,"#c56816")],
            "hlines":[],"vlines":[],"xranges":[],"annotations":[],"legend":true
        },
        "tab":null
    });
    Ok(format!(
        "<div class=\"report-figure\" data-section=\"{}\"></div>",
        escape_html(&json_python_separators(&figure))
    ))
}

fn acceptance_views_html(request: &Value) -> String {
    let data = &request["data"];
    let label = request["label"].as_str().unwrap_or_default();
    let heading = labelled_heading("Acceptance diagnostics", label);
    let start = format!(
        "<section class=\"acceptance-views\"><h2>{}</h2>",
        escape_html(&heading)
    );
    let unavailable = |reason: &str| {
        format!(
            "{start}<p>Unavailable: {}</p></section>",
            escape_html(reason)
        )
    };
    let evidence = &data["correction_decisions"]["acceptance_evidence"];
    if evidence.is_null() {
        return format!(
            "{start}<p>Unavailable: no production acceptance views were recorded (legacy or unfinalized output).</p></section>"
        );
    }
    if request["binding_verified"] != true {
        return unavailable(
            request["binding_reason"]
                .as_str()
                .unwrap_or("Payload binding unavailable."),
        );
    }
    if request["acceptance_evidence_verified"] != true {
        return unavailable(
            request["acceptance_evidence_reason"]
                .as_str()
                .unwrap_or("Acceptance-view payload binding is unavailable."),
        );
    }
    let graph_identity = request["binding_graph_identity"]
        .as_str()
        .unwrap_or_default();
    if graph_identity.is_empty() {
        return unavailable("Payload binding has no graph identity.");
    }
    if evidence["version"] != "acceptance-evidence-v1" {
        return unavailable("unsupported acceptance-evidence version");
    }
    let payload = &evidence["payload"];
    let channels = payload["channels"].as_object();
    let data_channels = data["channels"].as_object();
    let (Some(channels), Some(data_channels)) = (channels, data_channels) else {
        return unavailable("acceptance diagnostics do not cover the delivered channel set");
    };
    if channels.keys().collect::<std::collections::BTreeSet<_>>()
        != data_channels
            .keys()
            .collect::<std::collections::BTreeSet<_>>()
    {
        return unavailable("acceptance diagnostics do not cover the delivered channel set");
    }
    let mut parts = vec![
        start.clone(),
        "<p>Retained predictions and serialized DSP controls—not recorded playback, physical safety certification, or demonstrated listener benefit. Existing measurement conditioning is retained; additional smoothing cannot recover raw detail.</p>".into(),
        format!("<p>Graph: {}</p>", escape_html(graph_identity)),
    ];
    for (channel, entry) in channels {
        if entry["source_id"].as_str() != Some(channel)
            || entry["evidence_kind"] != "retained_prediction"
        {
            return unavailable("unsupported source or evidence classification");
        }
        parts.push(format!("<h3>{}</h3>", escape_html(channel)));
        parts.push(format!(
            "<p>Seat provenance: {}</p>",
            escape_html(
                entry
                    .get("seat_provenance")
                    .and_then(Value::as_str)
                    .unwrap_or("unavailable")
            )
        ));
        let Some(bundle) = entry.get("bundle").filter(|value| value.is_object()) else {
            return unavailable("acceptance bundle is missing or malformed");
        };
        let Some(settings) = bundle.get("settings").filter(|value| value.is_object()) else {
            return unavailable("acceptance settings are missing or malformed");
        };
        let rate = python_string(&settings["sample_rate_hz"]);
        let Some(normalization) = settings["normalization"].as_str() else {
            return unavailable("acceptance normalization is missing or malformed");
        };
        let Some(window) = settings["window"].as_str() else {
            return unavailable("acceptance window is missing or malformed");
        };
        parts.push(format!(
            "<p>Rate: {} Hz; levels: {}; window: {}</p>",
            escape_html(&rate),
            escape_html(normalization),
            escape_html(window)
        ));
        for (key, title, view) in [
            (
                "magnitude",
                "General magnitude (additional 1/12-octave smoothing)",
                &bundle["magnitude"],
            ),
            (
                "fine_magnitude",
                "Fine magnitude (additional 1/24-octave smoothing)",
                &entry["fine_magnitude"],
            ),
        ] {
            if !view.is_null() {
                if view["provenance"]["graph_identity"] != graph_identity {
                    return unavailable(&format!("{key} graph provenance mismatch"));
                }
                match magnitude_figure_html(view, title) {
                    Ok(html) => parts.push(html),
                    Err(error) => return unavailable(&error),
                }
            }
        }
        let disposition = &bundle["disposition"];
        if !disposition.is_null() {
            if disposition["graph_identity"] != graph_identity {
                return unavailable("DSP disposition graph identity mismatch");
            }
            parts.push(
                "<p>Serialized DSP disposition (not calibrated output capability):</p><ul>".into(),
            );
            if let Some(controls) = disposition["channels"].as_array() {
                for control in controls {
                    let text = format!(
                        "{}: gain {} dB; delay {} ms; inverted {}",
                        python_string(&control["channel"]),
                        python_string(&control["gain_db"]),
                        python_string(&control["delay_ms"]),
                        python_string(&control["inverted"])
                    );
                    parts.push(format!("<li>{}</li>", escape_html(&text)));
                }
            }
            parts.push("</ul>".into());
        }
        parts.push("<p>Unavailable evidence:</p><ul>".into());
        let Some(unavailable) = entry.get("unavailable").and_then(Value::as_object) else {
            return unavailable("acceptance unavailable-view inventory is missing or malformed");
        };
        for (view, reason) in unavailable {
            parts.push(format!(
                "<li>{}: {}</li>",
                escape_html(view),
                escape_html(&python_string(reason))
            ));
        }
        parts.push("</ul><p>No complete acoustic/headroom acceptance is established by these diagnostics.</p>".into());
    }
    format!("{}</section>", parts.join(""))
}

fn evidence_band(value: &Value) -> Option<(f64, f64)> {
    let band = value.as_array()?;
    if band.len() != 2 {
        return None;
    }
    let low = finite(&band[0])?;
    let high = finite(&band[1])?;
    (low > 0.0 && low < high).then_some((low, high))
}

fn format_general(value: f64, precision: usize) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let exponent = value.abs().log10().floor() as i32;
    if exponent < -4 || exponent >= precision as i32 {
        let formatted = format!("{:.*e}", precision.saturating_sub(1), value);
        let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        let parsed = exponent.parse::<i32>().unwrap_or(0);
        let sign = if parsed >= 0 { '+' } else { '-' };
        format!("{mantissa}e{sign}{:02}", parsed.unsigned_abs())
    } else {
        let decimals = precision.saturating_sub((exponent + 1).max(0) as usize);
        format!("{:.*}", decimals, value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

fn evidence_band_label(band: (f64, f64)) -> String {
    for precision in [6, 10, 15, 17] {
        let low = format_general(band.0, precision);
        let high = format_general(band.1, precision);
        if low != high {
            return format!("{low}–{high} Hz");
        }
    }
    format!(
        "{}–{} Hz",
        format_general(band.0, 17),
        format_general(band.1, 17)
    )
}

fn evidence_row(rows: &mut String, cells: &[String]) {
    rows.push_str("<tr>");
    for cell in cells {
        rows.push_str(&format!(
            "<td style=\"padding:8px;border-bottom:1px solid #ddd;vertical-align:top\">{}</td>",
            escape_html(cell)
        ));
    }
    rows.push_str("</tr>");
}

fn quantity(value: &Value, unit: &str) -> Option<String> {
    finite(value).map(|number| {
        format!("{} {unit}", format_general(number, 6))
            .trim()
            .to_string()
    })
}

fn verification_html(metadata: &Value) -> String {
    let comparisons = metadata
        .get("playback_comparisons")
        .filter(|value| value.as_array().is_some_and(|items| !items.is_empty()))
        .unwrap_or(&metadata["verification"]);
    let Some(comparisons) = comparisons.as_array().filter(|items| !items.is_empty()) else {
        return String::new();
    };
    let kind_text = [
        ("stationary_ir", "acoustic capture"),
        (
            "spatial_magnitude",
            "acoustic magnitude capture (no timing reference)",
        ),
        ("direct_sound", "acoustic direct-sound capture"),
        (
            "simulated_backend",
            "exported-backend simulation (not an acoustic recording)",
        ),
        ("unknown", "uncharacterized capture"),
    ];
    let mut items = Vec::new();
    for comparison in comparisons {
        if !comparison.is_object() {
            continue;
        }
        let kind = kind_text
            .iter()
            .find(|(value, _)| comparison["capture_kind"] == *value)
            .map(|(_, label)| *label)
            .unwrap_or("uncharacterized capture");
        let get_or = |name: &str, fallback: &str| {
            comparison[name]
                .as_str()
                .filter(|value| !value.is_empty())
                .unwrap_or(fallback)
                .to_string()
        };
        let baseline = get_or("baseline_graph_identity", "identity missing");
        let candidate = get_or("candidate_graph_identity", "identity missing");
        let stimulus = get_or("stimulus_hash", "stimulus hash missing");
        let assessed = comparison["baseline_graph_identity"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
            && comparison["candidate_graph_identity"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
            && baseline != candidate
            && comparison["stimulus_hash"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty());
        let verdict = if assessed {
            "recorded"
        } else {
            "unassessed: insufficient evidence, no promotion"
        };
        let seats = comparison["seat_ids"]
            .as_array()
            .map(|items| items.iter().map(python_string).collect::<Vec<_>>())
            .unwrap_or_default();
        let seat_text = if seats.is_empty() {
            String::new()
        } else {
            format!(" Seats: {}.", seats.join(", "))
        };
        items.push(format!(
            "Playback check ({kind}; {}): {verdict}. Source {}.{seat_text} Baseline {baseline}; candidate {candidate}; stimulus {stimulus}.",
            get_or("processing_state", "unstated signal level"),
            get_or("source_id", "unstated source")
        ));
    }
    if metadata["listening_evidence"].is_object() {
        let listening = &metadata["listening_evidence"];
        items.push(format!(
            "Recorded listening outcome: {} (protocol {}). A modeled perceptual score is not a listener result; an inconclusive or unassessed outcome is not listening benefit.",
            listening["result"].as_str().filter(|value| !value.is_empty()).unwrap_or("unassessed"),
            listening["protocol_id"].as_str().filter(|value| !value.is_empty()).unwrap_or("unstated protocol")
        ));
    }
    if items.is_empty() {
        return String::new();
    }
    format!(
        "<h3>Recorded playback and listening verification</h3><ul>{}</ul>",
        items
            .iter()
            .map(|item| format!("<li>{}</li>", escape_html(item)))
            .collect::<String>()
    )
}

fn measurement_conditioning_history(stage: &Value) -> Vec<String> {
    let names = [
        (
            "source_overlap_alignment",
            "Aligned source grids within shared support",
        ),
        (
            "source_spatial_power_rms",
            "Averaged spatial magnitudes in the power domain (no averaged phase)",
        ),
        (
            "source_coherent_pressure_mean",
            "Computed a declared coherent pressure mean",
        ),
        (
            "roomeq_dense_grid_conditioning",
            "Executed the dense-grid conditioning policy",
        ),
    ];
    let mut details = Vec::new();
    for check in stage["checks"].as_array().into_iter().flatten() {
        if !check.is_object() || check["passed"] != true {
            continue;
        }
        let parsed = check["diagnostic"]
            .as_str()
            .and_then(|diagnostic| serde_json::from_str::<Value>(diagnostic).ok());
        let Some(payload) =
            parsed.filter(|payload| payload.is_object() && payload["receipt"].is_object())
        else {
            continue;
        };
        let Some(entries) = payload["receipt"]["entries"].as_array() else {
            continue;
        };
        let channel = payload["channel"].as_str().unwrap_or("unknown channel");
        if payload["source_snapshot_binding"] == "verified_parsed_curve_snapshot" {
            details.push(format!("Measurement preparation for {channel}: native response identities match the frozen input snapshot in source order; this does not authenticate the original recording."));
        }
        if entries.is_empty() {
            details.push(format!("Measurement preparation for {channel}: no numerical loading change recorded; this does not describe later processing."));
        }
        for entry in entries {
            if !entry.is_object() {
                continue;
            }
            let operation = entry["operation"].as_str().unwrap_or("unknown operation");
            let mut description = names
                .iter()
                .find(|(known, _)| *known == operation)
                .map(|(_, text)| *text)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Recorded operation: {operation}"));
            if let Some(band) = evidence_band(&entry["parameters"]["declared_valid_band_hz"]) {
                description.push_str(&format!(
                    "; declared usable band {}",
                    evidence_band_label(band)
                ));
            }
            if !entry["parameters"]["input_bins"].is_null()
                && !entry["parameters"]["output_bins"].is_null()
            {
                description.push_str(&format!(
                    "; {} → {} bins",
                    python_string(&entry["parameters"]["input_bins"]),
                    python_string(&entry["parameters"]["output_bins"])
                ));
            }
            details.push(format!(
                "Measurement preparation for {channel}: {description}."
            ));
        }
    }
    if !details.is_empty() {
        details.push("Measurement preparation is partial recorded history, not proof of capture/calibration validity or complete conditioning lineage.".into());
    }
    details
}

fn output_loss_html(quality: &Value) -> String {
    let Some(entries) = quality["useful_output"]
        .as_array()
        .filter(|items| !items.is_empty())
    else {
        return String::new();
    };
    let mut rows = String::new();
    for entry in entries {
        if !entry.is_object() {
            continue;
        }
        let scope = format!(
            "{} / {} / seat {}",
            entry["logical_input"]
                .as_str()
                .filter(|v| !v.is_empty())
                .unwrap_or("Unknown input"),
            entry["partition"]
                .as_str()
                .filter(|v| !v.is_empty())
                .unwrap_or("unknown partition"),
            entry
                .get("seat_index")
                .map(python_string)
                .unwrap_or_else(|| "?".into())
        );
        let mean = quantity(&entry["mean_level_change_db"], "dB");
        let unexplained = quantity(&entry["unexplained_loss_rms_db"], "dB");
        let worst = quantity(&entry["worst_unexplained_loss_db"], "dB");
        let worst_hz = number(&entry["worst_loss_frequency_hz"]);
        let mut detail = format!(
            "Mean level change: {}. Unexplained loss (RMS): {}.",
            mean.as_deref().unwrap_or("unknown"),
            unexplained.as_deref().unwrap_or("unknown")
        );
        if let Some(worst) = worst {
            detail.push_str(&format!(" Worst unexplained loss: {worst}"));
            if let Some(hz) = worst_hz {
                detail.push_str(&format!(" at {hz} Hz."));
            } else {
                detail.push('.');
            }
        }
        detail.push_str(" Raw output evidence; display normalization cannot hide this loss.");
        evidence_row(&mut rows, &[scope, detail]);
    }
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "<h3>Recorded raw output loss (unnormalized)</h3><div style=\"overflow-x:auto\"><table style=\"width:100%;border-collapse:collapse;text-align:left\"><thead><tr><th>Seat</th><th>Recorded loss</th></tr></thead><tbody>{rows}</tbody></table></div>"
    )
}

fn k4_frequency_text(record: &Value) -> String {
    if let Some(band) = evidence_band(&record["frequency_band_hz"]) {
        return evidence_band_label(band);
    }
    finite(&record["filter_center_hz"])
        .filter(|value| *value > 0.0)
        .map(|value| format!("{} Hz (filter center)", format_general(value, 6)))
        .unwrap_or_else(|| "Frequency unavailable".into())
}

fn k4_scope_text(record: &Value) -> String {
    let logical = record["logical_input"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or("Unknown input");
    let physical = record["physical_output"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown output");
    let mut scope = format!("{logical} / {physical}");
    let seats = record["seat_refs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(python_string)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if !seats.is_empty() {
        scope.push_str(&format!(" / seat {}", seats.join(", ")));
    }
    let measurements = record["measurement_refs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(python_string)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if !measurements.is_empty() {
        scope.push_str(&format!(" (measured: {})", measurements.join(", ")));
    }
    scope
}

fn k4_decision_text(record: &Value) -> String {
    let action = match record["action"].as_str() {
        Some("equalize") => "Equalize",
        Some("phase_correct") => "Phase correction",
        Some("gain_adjust") => "Gain adjustment",
        Some("reroute") => "Reroute",
        Some("prune") => "Prune",
        _ => "Recorded action unavailable",
    };
    let status = match record["status"].as_str() {
        Some("applied") => "Applied",
        Some("already_acceptable") => "Already acceptable (no correction needed)",
        Some("insufficient_evidence") => "Insufficient evidence (no correction applied)",
        Some("outside_scope") => "Outside scope (no correction applied)",
        Some("constrained") => "Constrained (partial correction only; remainder withheld)",
        Some("reverted") => "Reverted (attempted correction rolled back; not delivered)",
        Some("unresolved") => "Unresolved (no decision reached)",
        Some("advisory") => "Advisory nomination only (nothing applied or removed)",
        _ => "Recorded status unavailable",
    };
    if record["stage"] == "final" {
        format!("{action} — {status} (final)")
    } else {
        format!("{action} — {status} (provisional; not a delivery claim)")
    }
}

fn k4_reason_text(record: &Value) -> String {
    let reason_names = [
        ("serialized_final_gain", "Gain present in the delivered DSP"),
        (
            "final_electrical_headroom",
            "Attenuation for the configured digital headroom ceiling and any declared physical-drive limits",
        ),
        (
            "final_channel_level_alignment",
            "Final channel-level alignment",
        ),
        (
            "branch_gain_not_acoustic_benefit_or_net_parallel_gain",
            "Scalar gain on this input/output branch; not net parallel-path gain or measured acoustic benefit",
        ),
        (
            "joint_array_proposal_reverted",
            "Joint subwoofer gain/delay proposal reverted",
        ),
        (
            "joint_shared_eq_proposal_reverted",
            "Shared subwoofer EQ proposal reverted",
        ),
        (
            "stage_history_not_final_route_acceptance",
            "This is stage history, not approval of later routing or recorded playback",
        ),
    ];
    let codes = record["reason_codes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(python_string)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    let labels = codes
        .iter()
        .map(|code| {
            reason_names
                .iter()
                .find(|(known, _)| *known == code)
                .map(|(_, label)| *label)
                .unwrap_or(code)
        })
        .collect::<Vec<_>>();
    let mut reason = if labels.is_empty() {
        "No reason recorded.".into()
    } else {
        format!("Recorded reason: {}.", labels.join(", "))
    };
    reason.push_str(&format!(
        " Confidence: {}.",
        record["confidence"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
    ));
    let quantity_list = |key: &str| -> String {
        record[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                let value = quantity(&item["value"], item["unit"].as_str().unwrap_or(""))?;
                let name = item["name"]
                    .as_str()
                    .filter(|name| !name.trim().is_empty())?;
                Some(format!("{}: {}", name.trim(), value))
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    let observed = quantity_list("observed");
    if !observed.is_empty() {
        reason.push_str(&format!(" Observed: {observed}."));
    }
    let limits = quantity_list("limits");
    if !limits.is_empty() {
        reason.push_str(&format!(" Limit: {limits}."));
    }
    let evidence = record["evidence_refs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(python_string)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if evidence.is_empty() {
        reason.push_str(" Evidence: unavailable.");
    } else {
        reason.push_str(&format!(" Evidence: {}.", evidence.join(", ")));
    }
    for (key, template) in [
        (
            "related_decision_ids",
            "Linked record: {}. Both the applied partial correction and its constrained remainder stay visible.",
        ),
        (
            "supersedes_ids",
            "Supersedes: {}. The superseded attempt is not delivered correction.",
        ),
    ] {
        let linked = record[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(python_string)
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>();
        if !linked.is_empty() {
            reason.push(' ');
            reason.push_str(&template.replace("{}", &linked.join(", ")));
        }
    }
    reason
}

fn k4_section_html(data: &Value, verified: bool, binding_reason: &str) -> (String, Vec<String>) {
    let ledger = &data["correction_decisions"];
    let Some(decisions) = ledger["decisions"]
        .as_array()
        .filter(|items| !items.is_empty())
    else {
        return (String::new(), Vec::new());
    };
    let metadata = &data["metadata"];
    let acceptance = &metadata["correction_acceptance"];
    let delivery_blocked =
        acceptance["decision"] == "identity_fallback" || acceptance["outcome"] == "rejected";
    let delivered = metadata["delivered_graph_identity"]
        .as_str()
        .filter(|value| !value.trim().is_empty());
    let final_records = decisions
        .iter()
        .filter(|record| record.is_object() && record["stage"] == "final")
        .collect::<Vec<_>>();
    let identities = final_records
        .iter()
        .filter_map(|record| {
            record["final_graph_identity"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
        })
        .collect::<std::collections::BTreeSet<_>>();
    let disagree = identities.len() > 1;
    let payload_identity = ledger["payload_binding"]["graph_identity"]
        .as_str()
        .unwrap_or("");
    let binding_invalid = !verified
        || disagree
        || final_records
            .iter()
            .any(|record| record["final_graph_identity"].as_str() != Some(payload_identity))
        || delivered.is_some_and(|value| value != payload_identity);
    let mut rows = String::new();
    let mut history = Vec::new();
    for record in decisions {
        if !record.is_object() {
            history.push(
                "Unverified decision record: unreadable entry; not delivered correction.".into(),
            );
            continue;
        }
        let Some(decision_id) = record["decision_id"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
        else {
            history.push(
                "Unverified decision record: missing decision ID; not delivered correction.".into(),
            );
            continue;
        };
        let scope = k4_scope_text(record);
        let frequency = k4_frequency_text(record);
        let mut decision = k4_decision_text(record);
        let mut reason = k4_reason_text(record);
        if record["stage"] != "final" {
            history.push(format!("Provisional record {decision_id} ({scope}; {frequency}; {decision}): {reason} Final reconciled records above take precedence."));
            continue;
        }
        let identity = record["final_graph_identity"].as_str().unwrap_or("");
        if delivery_blocked
            && matches!(
                record["status"].as_str(),
                Some("applied" | "already_acceptable" | "constrained")
            )
        {
            decision.push_str(" (not delivered correction)");
            reason
                .push_str(" Final rejection or identity fallback overrides this candidate record.");
        }
        let row_verified = verified
            && identity == payload_identity
            && !identity.trim().is_empty()
            && !disagree
            && delivered.is_none_or(|value| identity == value);
        if !row_verified {
            decision.push_str(" (unverified; not a delivery claim)");
            reason.push(' ');
            reason.push_str(binding_reason);
            if identity.trim().is_empty() {
                reason.push_str(" Unverified: final record without a delivered-graph identity is not a delivery claim.");
            } else if disagree {
                reason.push_str(" Unverified: final records disagree on the delivered-graph identity; the explanation cannot be bound to one delivered graph.");
            } else if delivered.is_some_and(|value| identity != value) || verified {
                reason.push_str(" Unverified: record identity does not match the recorded delivered graph; not displayed as applied.");
            }
        }
        evidence_row(&mut rows, &[scope, frequency, decision, reason]);
    }
    if rows.is_empty() && history.is_empty() {
        return (String::new(), Vec::new());
    }
    let mut banner = format!("<p><strong>{}</strong></p>", escape_html(binding_reason));
    if disagree {
        banner.push_str("<p><strong>Final records disagree on the delivered-graph identity; this explanation is unverified and nothing below is shown as delivered.</strong></p>");
    }
    let statuses: [(&str, &[&str]); 3] = [
        ("applied", &["applied", "already_acceptable", "constrained"]),
        ("reverted", &["reverted"]),
        (
            "withheld",
            &[
                "insufficient_evidence",
                "outside_scope",
                "unresolved",
                "advisory",
            ],
        ),
    ];
    let set_labels = [
        ("applied", "applied delivery claims"),
        ("reverted", "reverted (rolled back; not delivered)"),
        ("provisional", "provisional history (not delivery claims)"),
        ("withheld", "withheld (final rows without delivery)"),
    ];
    let mut set_ids: std::collections::BTreeMap<&str, Vec<String>> =
        ["applied", "reverted", "provisional", "withheld"]
            .into_iter()
            .map(|key| (key, Vec::new()))
            .collect();
    let mut set_reasons: std::collections::BTreeMap<&str, Vec<String>> =
        ["applied", "reverted", "provisional", "withheld"]
            .into_iter()
            .map(|key| (key, Vec::new()))
            .collect();
    for record in decisions {
        if !record.is_object() {
            set_ids
                .get_mut("provisional")
                .unwrap()
                .push("unreadable entry".into());
            continue;
        }
        let id = record["decision_id"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("missing decision ID")
            .to_string();
        let bucket = if record["stage"] != "final" {
            "provisional"
        } else {
            statuses
                .iter()
                .find(|(_, allowed)| allowed.iter().any(|status| record["status"] == *status))
                .map(|(bucket, _)| *bucket)
                .unwrap_or("withheld")
        };
        let bucket = if (delivery_blocked || binding_invalid) && bucket == "applied" {
            "withheld"
        } else {
            bucket
        };
        set_ids.get_mut(bucket).unwrap().push(id);
        if let Some(codes) = record["reason_codes"].as_array() {
            for code in codes
                .iter()
                .map(python_string)
                .filter(|value| !value.trim().is_empty())
            {
                let reasons = set_reasons.get_mut(bucket).unwrap();
                if !reasons.contains(&code) {
                    reasons.push(code);
                }
            }
        }
    }
    let summary_rows = set_labels
        .iter()
        .map(|(bucket, label)| {
            let ids = set_ids[*bucket].join(", ");
            let ids = if ids.is_empty() { "none" } else { &ids };
            let reasons = &set_reasons[*bucket];
            let why = if reasons.is_empty() {
                " No reason recorded.".into()
            } else {
                format!(" Reasons: {}.", reasons.join(", "))
            };
            format!(
                "<li>{}</li>",
                escape_html(&format!("{} {label} ({ids}).{why}", set_ids[*bucket].len()))
            )
        })
        .collect::<String>();
    let mut table = format!(
        "<h3>Recorded final correction decisions</h3><h4>Final acceptance sets:</h4><ul class=\"acceptance-sets\">{summary_rows}</ul>{banner}"
    );
    if !rows.is_empty() {
        table.push_str(&format!("<div style=\"overflow-x:auto\"><table style=\"width:100%;border-collapse:collapse;text-align:left\"><thead><tr><th>Channel / output / seat</th><th>Frequency</th><th>Decision</th><th>Why</th></tr></thead><tbody>{rows}</tbody></table></div>"));
    } else {
        table.push_str("<p>No final reconciled decisions recorded; provisional history below is not delivered correction.</p>");
    }
    (table, history)
}

fn correction_explanation_html(
    data: &Value,
    label: &str,
    binding_verified: bool,
    binding_reason: &str,
) -> String {
    let metadata = &data["metadata"];
    let acceptance = &metadata["correction_acceptance"];
    let quality = &acceptance["acoustic_quality"];
    let optimizer = &metadata["effective_config"]["optimizer"];
    let requested = if optimizer["correction_band"].is_null() {
        evidence_band(&json!([optimizer["min_freq"], optimizer["max_freq"]]))
    } else {
        evidence_band(&json!([
            optimizer["correction_band"]["min_hz"],
            optimizer["correction_band"]["max_hz"]
        ]))
    };
    let recorded = evidence_band(&quality["correction_band_hz"]);
    let mut observation = evidence_band(
        &optimizer["min_freq"]
            .as_f64()
            .zip(optimizer["max_freq"].as_f64())
            .map(|(a, b)| json!([a, b]))
            .unwrap_or(Value::Null),
    );
    observation = evidence_band(&quality["evaluated_band_hz"]).or(observation);
    let mut rows = String::new();
    let outcome = acceptance["outcome"].as_str();
    let mut outcome_message = match outcome {
        Some("accepted") if acceptance["accepted"] != true || acceptance["decision"] != "accepted" => "The acceptance record is incomplete or inconsistent; final correction is not verified.".to_string(),
        Some("accepted") => "The final correction passed the recorded acceptance checks. This is not a listening-benefit claim.".into(),
        Some("unchanged") => "The final outcome is unchanged; earlier correction attempts are not proof of delivered correction.".into(),
        Some("rejected") => "The final correction was rejected. Earlier applied stages do not establish an accepted result.".into(),
        Some("insufficient_evidence") => "The available evidence was insufficient to accept the final correction.".into(),
        _ => "No final correction decision was recorded.".into(),
    };
    if matches!(outcome, Some("accepted" | "unchanged"))
        && populated(&data["correction_decisions"])
        && !binding_verified
    {
        outcome_message = format!(
            "Recorded outcome: {}; not verified for this payload. {binding_reason}",
            outcome.unwrap_or_default()
        );
    }
    if acceptance["decision"] == "identity_fallback" {
        outcome_message = "The final outcome is unchanged: it fell back to identity; no applied correction records can override that outcome. The fallback withheld correction; candidate records below are not delivered correction.".into();
    }
    evidence_row(
        &mut rows,
        &[
            "System".into(),
            "All assessed frequencies".into(),
            "Final outcome".into(),
            outcome_message,
        ],
    );
    let correction = recorded.or(requested);
    if let Some(correction) = correction {
        evidence_row(
            &mut rows,
            &[
                "System".into(),
                evidence_band_label(correction),
                "Correction scope".into(),
                if recorded.is_some() {
                    "Recorded active-correction band.".into()
                } else {
                    "The configuration permits response shaping in this band; it does not prove that every frequency was corrected.".into()
                },
            ],
        );
        if let (Some(requested), Some(recorded)) = (requested, recorded)
            && requested != recorded
        {
            evidence_row(&mut rows, &["System".into(), evidence_band_label(requested), "Requested scope".into(), "The requested correction band differs from the recorded active-correction band shown above.".into()]);
        }
        if let Some(observation) = observation {
            for outside in [
                (observation.0, correction.0.min(observation.1)),
                (correction.1.max(observation.0), observation.1),
            ] {
                if outside.0 > 0.0 && outside.0 < outside.1 {
                    evidence_row(&mut rows, &["System".into(), evidence_band_label(outside), "Outside correction scope".into(), "Excluded from the active correction band. Routing, crossovers, gain and filter tails can still affect playback here.".into()]);
                }
            }
        }
    } else {
        evidence_row(&mut rows, &["System".into(), "Frequency range unavailable".into(), "Scope not recorded".into(), "This output does not provide an explicit correction band. It is not inferred from the response curves.".into()]);
    }
    if let Some(seats) = quality["final_seats"].as_array() {
        for seat in seats {
            let scope = format!(
                "{} / {} / seat {}",
                seat["logical_input"].as_str().unwrap_or("Unknown input"),
                seat["partition"].as_str().unwrap_or("unknown partition"),
                seat.get("seat_index")
                    .map(python_string)
                    .unwrap_or_else(|| "?".into())
            );
            if let Some(bands) = seat["unassessed_bands_hz"].as_array() {
                for band in bands.iter().filter_map(evidence_band) {
                    evidence_row(&mut rows, &[scope.clone(), evidence_band_label(band), "Not assessed".into(), "Final playback was not assessed in this band. This does not establish that correction was absent or unnecessary.".into()]);
                }
            }
        }
    }
    let (k4_section, k4_history) = k4_section_html(data, binding_verified, binding_reason);
    if k4_section.is_empty() {
        evidence_row(&mut rows, &["System".into(), "Individual decisions unavailable".into(), "Reason unavailable".into(), "No versioned decision ledger was recorded with this output. Detailed reasons for individual peaks and dips are unavailable.".into()]);
    }
    let verification = verification_html(metadata);
    let output_loss = output_loss_html(quality);
    let veto_reasons = [
        (
            "SubJnd",
            "Below the configured level-difference proxy threshold.",
        ),
        (
            "SubErbWidth",
            "Below the configured affected-width proxy threshold.",
        ),
        (
            "HighQAboveGuard",
            "Exceeds the configured high-frequency Q guard.",
        ),
        (
            "ModeProximityBan",
            "Flagged by the recorded mode-proximity rule.",
        ),
        (
            "Audible",
            "Passed the configured veto rules; this is not proof of audibility.",
        ),
    ];
    if let Some(channels) = metadata["audibility_veto"].as_object() {
        for (channel, verdicts) in channels {
            if let Some(verdicts) = verdicts.as_array() {
                for verdict in verdicts {
                    let Some(frequency) = finite(&verdict["center_hz"]).filter(|v| *v > 0.0) else {
                        continue;
                    };
                    let decision = if verdict["decision"] == "Remove" {
                        "Removal nominated"
                    } else {
                        "Recorded veto decision"
                    };
                    let action = format!(
                        "{decision} ({})",
                        if verdict["enforced"] == true {
                            "stage history"
                        } else {
                            "advisory"
                        }
                    );
                    let reason = veto_reasons
                        .iter()
                        .find(|(code, _)| verdict["reason"] == *code)
                        .map(|(_, text)| *text)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            format!(
                                "Recorded reason: {}.",
                                verdict["reason"].as_str().unwrap_or("unknown")
                            )
                        });
                    let assessment = &verdict["acceptance"];
                    let reason = format!(
                        "{reason} Confidence: {}. This filter-center record does not establish a frequency interval or the final exported filter set.",
                        assessment["confidence"].as_str().unwrap_or("unknown")
                    );
                    evidence_row(
                        &mut rows,
                        &[
                            channel.clone(),
                            format!("{} Hz (filter center)", format_general(frequency, 6)),
                            action,
                            reason,
                        ],
                    );
                }
            }
        }
    }
    let mut details = k4_history;
    if let Some(violations) = acceptance["violations"].as_array() {
        details.extend(
            violations
                .iter()
                .map(|value| format!("Final acceptance constraint: {}", python_string(value))),
        );
    }
    if let Some(stages) = acceptance["reverted_stages"].as_array() {
        details.extend(
            stages
                .iter()
                .map(|value| format!("Reverted stage: {}", python_string(value))),
        );
    }
    if let Some(stages) = metadata["stage_outcomes"].as_array() {
        for stage in stages {
            let name = stage["stage"].as_str().unwrap_or("unknown");
            let status = stage["status"].as_str().unwrap_or("unknown");
            if name == "measurement_input_conditioning" {
                details.extend(measurement_conditioning_history(stage));
            }
            if let Some(advisories) = stage["advisories"].as_array() {
                details.extend(advisories.iter().map(|advisory| {
                    format!("Stage {name} ({status}): {}", python_string(advisory))
                }));
            }
            let mut has_failure = false;
            if let Some(checks) = stage["checks"].as_array() {
                for check in checks {
                    if check["passed"] == false {
                        has_failure = true;
                        details.push(format!(
                            "Stage {name} ({status}), {}: {}",
                            check["id"].as_str().unwrap_or("unknown check"),
                            check["diagnostic"]
                                .as_str()
                                .filter(|value| !value.is_empty())
                                .unwrap_or("Check failed; no explanation recorded.")
                        ));
                    }
                }
            }
            if matches!(status, "skipped" | "degraded" | "failed")
                && stage["advisories"].as_array().is_none_or(Vec::is_empty)
                && !has_failure
            {
                details.push(format!("Stage {name}: {status}; no reason recorded."));
            }
        }
    }
    let diagnostics = if details.is_empty() {
        String::new()
    } else {
        format!(
            "<details><summary>Recorded constraints and stage history</summary><p>Stage history may describe attempts that were later reverted. The final outcome above takes precedence.</p><ul>{}</ul></details>",
            details
                .iter()
                .map(|item| format!("<li>{}</li>", escape_html(item)))
                .collect::<String>()
        )
    };
    let title = labelled_heading("Why this correction?", label);
    format!(
        "<section class=\"correction-explanation\"><h2>{}</h2><p>Recorded reasons for correction scope, limitations and the final outcome.</p><div style=\"overflow-x:auto\"><table style=\"width:100%;border-collapse:collapse;text-align:left\"><thead><tr><th>Channel / scope</th><th>Frequency</th><th>Decision</th><th>Why</th></tr></thead><tbody>{rows}</tbody></table></div>{k4_section}{verification}{output_loss}{diagnostics}<p>Detailed reasons for individual peaks and dips are unavailable unless recorded above. An unchanged response alone does not identify a cancellation, poor measurement, or an already acceptable result.</p></section>\n",
        escape_html(&title)
    )
}

/// Builds four global evidence renderers from saved data and externally
/// verified binding evidence. Missing verification remains unavailable.
pub fn global_evidence_sections(data: &Value, label: &str, binding: &Value) -> Map<String, Value> {
    let verified = binding["verified"].as_bool().unwrap_or(false);
    let reason = binding["reason"]
        .as_str()
        .unwrap_or("Payload binding evidence unavailable.");
    let graph_identity = binding["graph_identity"].as_str().unwrap_or("");
    let mut sections = Map::new();
    sections.insert(
        "correction_explanation_html".into(),
        json!(correction_explanation_html(data, label, verified, reason)),
    );
    sections.insert(
        "acceptance_views_html".into(),
        json!(acceptance_views_html(&json!({
            "data":data,
            "label":label,
            "binding_verified":verified,
            "binding_reason":reason,
            "binding_graph_identity":graph_identity,
            "acceptance_evidence_verified":binding["acceptance_evidence_verified"],
            "acceptance_evidence_reason":binding["acceptance_evidence_reason"],
        }))),
    );
    sections.insert(
        "waveform_status_html".into(),
        json!(waveform_status_html(data, label)),
    );
    sections.insert(
        "capture_clock_qa_html".into(),
        json!(capture_clock_qa_html(data)),
    );
    sections.insert(
        "capture_reflections_html".into(),
        json!(capture_reflections_html(data)),
    );
    sections.insert(
        "modes_summary_html".into(),
        json!(resonance_summary_html(data)),
    );
    sections
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
        "waveform_status" => json!(waveform_status_html(
            &request["data"],
            request["label"].as_str().unwrap_or_default()
        )),
        "capture_clock_qa" => json!(capture_clock_qa_html(&request["data"])),
        "capture_reflections" => json!(capture_reflections_html(&request["data"])),
        "modes_summary" => json!(resonance_summary_html(&request["data"])),
        "capture_views" => json!(capture_views_html(
            &request["report"],
            &request["verification"]
        )),
        "acceptance_views" => json!(acceptance_views_html(request)),
        "correction_explanation" => json!(correction_explanation_html(
            &request["data"],
            request["label"].as_str().unwrap_or_default(),
            request["binding_verified"].as_bool().unwrap_or(false),
            request["binding_reason"]
                .as_str()
                .unwrap_or("Payload binding evidence unavailable.")
        )),
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

    #[test]
    fn global_evidence_renderers_match_pre_port_python_snapshots() {
        let cases: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/global_evidence_python.json"
        ))
        .unwrap();
        for (index, case) in cases.as_array().unwrap().iter().enumerate() {
            let request = case.get("request").cloned().unwrap_or_else(|| {
                json!({
                    "builder": case["builder"],
                    "data": case["data"],
                    "label": case.get("label").unwrap_or(&Value::Null),
                })
            });
            let actual = evidence_section(&request).unwrap();
            assert_eq!(actual, case["expected"], "case {index}");
        }
    }

    #[test]
    fn waveform_status_distinguishes_absent_stage_outcomes_from_null() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/waveform_missing_null_python.json"
        ))
        .unwrap();
        for case in ["missing", "null"] {
            assert_eq!(
                waveform_status_html(&fixture[case]["data"], ""),
                fixture[case]["expected"]
            );
        }
    }

    #[test]
    fn global_capture_sections_match_pre_port_python_snapshots() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/global_capture_sections_python.json"
        ))
        .unwrap();
        let capture_actual = capture_views_html(
            &fixture["capture_views"]["report"],
            &fixture["capture_views"]["verification"],
        );
        assert_embedded_html_equal(
            &capture_actual,
            fixture["capture_views"]["expected"].as_str().unwrap(),
        );
        assert_eq!(
            resonance_summary_html(&fixture["resonance_summary"]["data"]),
            fixture["resonance_summary"]["expected"]
        );
        let reflection_actual = capture_reflections_html(&fixture["capture_reflections"]["data"]);
        assert_embedded_html_equal(
            &reflection_actual,
            fixture["capture_reflections"]["expected"].as_str().unwrap(),
        );
    }

    #[test]
    fn complete_nested_capture_views_match_python_snapshot() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/capture_views_nested_python.json"
        ))
        .unwrap();
        let actual = capture_views_html(&fixture["report"], &fixture["verification"]);
        let (actual_text, actual_figures) = split_embedded_figures(&actual);
        let (expected_text, expected_figures) =
            split_embedded_figures(fixture["expected"].as_str().unwrap());
        assert_eq!(actual_text, expected_text);
        assert_eq!(actual_figures.len(), expected_figures.len());
        for (index, (actual, expected)) in actual_figures.iter().zip(&expected_figures).enumerate()
        {
            assert_eq!(actual, expected, "embedded figure {index}");
        }
    }

    #[test]
    fn capture_views_require_native_comparison_binding() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/global_capture_sections_python.json"
        ))
        .unwrap();
        let mut verification = fixture["capture_views"]["verification"].clone();
        verification["comparisons"][0]["verified"] = json!(false);
        let html = capture_views_html(&fixture["capture_views"]["report"], &verification);
        assert!(html.contains("capture view payload or graph/source/seat binding changed"));
        assert!(!html.contains("Matched IR evidence"));
    }

    #[test]
    fn coherent_clock_gate_resolves_saved_sources_and_fails_closed() {
        let data = json!({
            "system":{"speakers":{"L":"left_source","R":"right_source"}},
            "speakers":{"left_source":{},"right_source":{}}
        });
        assert_eq!(
            coherent_capture_clock_reason(&data, ["L", "R"]).as_deref(),
            Some("left_source: per-microphone clock bounds are unavailable")
        );
        assert_eq!(
            coherent_capture_clock_reason(&data, ["L", "L"]).as_deref(),
            Some("two distinct output channels are required")
        );
        let missing_source = json!({"speakers":{}});
        assert_eq!(
            coherent_capture_clock_reason(&missing_source, ["L", "R"]).as_deref(),
            Some("capture provenance is unavailable for L")
        );
    }

    fn assert_embedded_html_equal(actual: &str, expected: &str) {
        let (actual_text, actual_figures) = split_embedded_figures(actual);
        let (expected_text, expected_figures) = split_embedded_figures(expected);
        assert_eq!(actual_text, expected_text);
        assert_eq!(actual_figures, expected_figures);
    }

    fn split_embedded_figures(html: &str) -> (String, Vec<Value>) {
        let marker = "<div class=\"report-figure\" data-section=\"";
        let mut remaining = html;
        let mut plain = String::new();
        let mut figures = Vec::new();
        while let Some(start) = remaining.find(marker) {
            plain.push_str(&remaining[..start]);
            let rest = &remaining[start + marker.len()..];
            let Some(end) = rest.find("\"></div>") else {
                panic!("unterminated embedded figure");
            };
            let encoded = &rest[..end];
            let decoded = encoded
                .replace("&quot;", "\"")
                .replace("&#x27;", "'")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&");
            figures.push(serde_json::from_str(&decoded).expect("embedded figure JSON"));
            plain.push_str("<figure></figure>");
            remaining = &rest[end + 8..];
        }
        plain.push_str(remaining);
        (plain, figures)
    }
}
