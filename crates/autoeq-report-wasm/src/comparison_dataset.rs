//! Composes ordered comparison payloads from prepared report modes.
//!
//! This module coordinates existing evidence, channel, target, table, and
//! figure owners. It does not load files or reproduce DSP calculations.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeSet;
mod playback_scores;

use serde_json::{Value, json};

use crate::{PreparedReportData, report_dataset::ReportDatasetOptions};

const DEFAULT_SMOOTHING_OCTAVES: f64 = 1.0 / 6.0;
const BASS_MIN_HZ: f64 = 20.0;
const BASS_MAX_HZ: f64 = 500.0;
const TARGET_MATCH_LOW_HZ: f64 = 100.0;
const TARGET_MATCH_HIGH_HZ: f64 = 10_000.0;

struct ModeInput {
    name: String,
    prepared: PreparedReportData,
    options: ReportDatasetOptions,
    figure_context: Value,
    channel_blocks: Vec<Value>,
}

#[derive(Clone)]
struct ChannelCurves {
    initial: Option<Value>,
    final_curve: Option<Value>,
    target: Option<Value>,
    pre_ir: Option<Value>,
    post_ir: Option<Value>,
    eq_response: Option<Value>,
}

/// Builds a complete comparison payload from ordered prepared report modes.
///
/// Each mode contains `name`, `prepared`, `options`, and optional
/// `figure_context`. The context carries canonical precomputed comparison
/// arrays such as group delay and the coherence-gated L+R source. Missing
/// context remains explicitly unavailable.
///
/// # Errors
/// Returns an error for invalid prepared modes, options, or evidence shapes.
pub fn build_comparison_report(request: &Value) -> Result<Value, String> {
    let requested_modes = request
        .get("modes")
        .and_then(Value::as_array)
        .ok_or("comparison request requires a modes array")?;
    if requested_modes.is_empty() {
        return Err("comparison request requires at least one mode".into());
    }

    let mut modes = Vec::with_capacity(requested_modes.len());
    let mut preparation_status = Vec::new();
    for (index, mode) in requested_modes.iter().enumerate() {
        let name = mode
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("comparison mode {index} requires a nonempty name"))?
            .to_owned();
        let prepared: PreparedReportData = serde_json::from_value(
            mode.get("prepared")
                .cloned()
                .ok_or_else(|| format!("comparison mode {name} requires prepared data"))?,
        )
        .map_err(|error| format!("invalid prepared comparison mode {name}: {error}"))?;
        let options: ReportDatasetOptions = match mode.get("options") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid options for comparison mode {name}: {error}"))?,
            None => ReportDatasetOptions::default(),
        };
        prepared.validate()?;
        options.validate()?;
        let figure_context = mode
            .get("figure_context")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !figure_context.is_object() {
            return Err(format!(
                "comparison mode {name} figure_context must be an object"
            ));
        }
        let channel_blocks =
            crate::report_dataset_channel::build_dataset_channel_blocks(&prepared, &options)?;
        let global =
            crate::report_dataset_global::build_global_report_sections(&prepared, &options)?;
        for item in global["preparation_status"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut item = item.clone();
            item["mode"] = json!(name);
            preparation_status.push(item);
        }
        modes.push(ModeInput {
            name,
            prepared,
            options,
            figure_context,
            channel_blocks,
        });
    }

    let page_title = format!(
        "RoomEQ Mode Comparison: {}",
        modes
            .iter()
            .map(|mode| mode.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut sections = Vec::new();
    append_mode_evidence(&modes, &mut sections)?;
    append_summary(&modes, &mut sections)?;

    let mut names = BTreeSet::new();
    for mode in &modes {
        if let Some(channels) = mode.prepared.graph["channels"].as_object() {
            names.extend(channels.keys().cloned());
        }
    }
    let mut sorted_channels: Vec<_> = names.into_iter().collect();
    sorted_channels.sort_by(|left, right| {
        crate::report_helpers::channel_sort_key(left)
            .cmp(&crate::report_helpers::channel_sort_key(right))
    });
    let has_lr_pair = modes.iter().any(|mode| {
        mode.prepared.graph["channels"].get("L").is_some()
            && mode.prepared.graph["channels"].get("R").is_some()
    });
    if has_lr_pair {
        sorted_channels.push("L+R".into());
    }

    for channel in &sorted_channels {
        append_comparison_channel(channel, &modes, &mut sections, &mut preparation_status)?;
    }

    let payload = json!({
        "schema":"autoeq-report-data-v1",
        "title":page_title,
        "sections":sections,
        "provenance":request.get("provenance").cloned().unwrap_or(Value::Null),
    });
    Ok(json!({"payload":payload,"preparation_status":preparation_status}))
}

fn append_mode_evidence(modes: &[ModeInput], sections: &mut Vec<Value>) -> Result<(), String> {
    for mode in modes {
        let binding = crate::report_dataset::binding_evidence(&mode.prepared, &mode.options)?;
        let metadata = mode.prepared.graph.get("metadata").unwrap_or(&Value::Null);
        let playback = crate::evidence_sections::evidence_section(&json!({
            "builder":"playback_status",
            "metadata":metadata,
            "label":mode.name,
            "binding_verified":binding["verified"],
            "binding_reason":binding["reason"],
        }))?;
        push_html_section(sections, playback.as_str().unwrap_or_default(), true, None);
        let evidence = crate::report_sections::build_global_evidence_sections(
            &mode.prepared.graph,
            &mode.name,
            &binding,
        );
        for key in ["capture_clock_qa_html", "capture_reflections_html"] {
            push_html_section(
                sections,
                evidence
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                false,
                None,
            );
        }
        for key in [
            "correction_explanation_html",
            "acceptance_views_html",
            "waveform_status_html",
        ] {
            push_html_section(
                sections,
                evidence
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                false,
                None,
            );
        }
        if let (Some(capture), Some(view_bindings)) = (
            mode.options.capture_verification.as_ref(),
            mode.options.capture_view_bindings.as_ref(),
        ) {
            let html = crate::evidence_sections::capture_views_html(capture, view_bindings);
            push_html_section(sections, &html, false, None);
        }
    }
    Ok(())
}

fn append_summary(modes: &[ModeInput], sections: &mut Vec<Value>) -> Result<(), String> {
    let has_playback = modes.iter().any(|mode| {
        !mode.prepared.graph["metadata"]["correction_acceptance"]["acoustic_quality"]
            ["training"]["configured_baseline"].is_null()
    });
    if has_playback {
        let mut html = String::from(
            "<h2>Delivered playback</h2><table class=\"summary-table\"><thead><tr><th>Mode</th><th>Outcome</th><th>Pre RMS (dB)</th><th>Post RMS (dB)</th><th>Improvement (dB)</th><th>Responses</th></tr></thead><tbody>",
        );
        for mode in modes {
            let metadata = &mode.prepared.graph["metadata"];
            let label = crate::report_helpers::escape_html(mode_label(&mode.name));
            let outcome = crate::report_helpers::escape_html(
                metadata["correction_acceptance"]["outcome"]
                    .as_str()
                    .unwrap_or("Unavailable"),
            );
            match playback_scores::configured_mean(metadata) {
                Some((pre, post, count)) => html.push_str(&format!(
                    "<tr><td>{label}</td><td>{outcome}</td><td>{pre:.4}</td><td>{post:.4}</td><td>{:+.4}</td><td>{count}</td></tr>", pre-post,
                )),
                None => html.push_str(&format!(
                    "<tr><td>{label}</td><td>{outcome}</td><td colspan=\"4\">Unavailable: complete playback comparison evidence is missing</td></tr>",
                )),
            }
        }
        html.push_str("</tbody></table><p>Scores average the configured playback responses on their fixed baseline grid and level reference. Compare modes only when their baseline, target and level match. Response counts do not establish coverage of multiple listening positions.</p>");
        push_html_section(sections, &html, false, None);
    }
    let has_gd = modes
        .iter()
        .any(|mode| !mode.prepared.graph["metadata"]["group_delay"].is_null());
    let has_auto = modes.iter().any(|mode| mode.name.contains("_auto"));
    let mut losses = BTreeSet::new();
    for mode in modes {
        if let Some(loss) = mode.prepared.graph["metadata"]["loss_type"]
            .as_str()
            .filter(|loss| !loss.is_empty())
        {
            losses.insert(loss.to_owned());
        }
    }
    let mut html = String::from(
        "<h2>Summary</h2>\n<table class=\"summary-table\">\n<thead><tr><th>Mode</th><th>Loss</th><th title=\"Flat loss before EQ (always computed via compute_flat_loss, regardless of which objective was minimized)\">Pre flat-loss</th><th title=\"Flat loss after EQ (always computed via compute_flat_loss, regardless of which objective was minimized)\">Post flat-loss</th><th>Improvement</th><th>EPA Pref (pre)</th><th>EPA Pref (post)</th><th>EPA Δ</th>",
    );
    if has_playback {
        html = html.replacen("<h2>Summary</h2>", "<h2>Optimizer training</h2>", 1);
    }
    if has_auto {
        html.push_str("<th title=\"Emitted EQ sections per channel, including driver-local sections; Kautz banks count all basis sections and shared entries count once; ranges show min-max across channels\">EQ sections</th>");
    }
    if has_gd {
        html.push_str("<th title=\"Group-delay optimization advisory\">GD advisory</th><th title=\"Whether GD controls were inserted into exported DSP\">GD applied</th><th title=\"Summed in-band GD RMS before and after GD optimization\">GD RMS</th><th title=\"GD RMS improvement, 20*log10(pre/post)\">GD Δ</th><th title=\"Total emitted all-pass filters; coh is mean in-band coherence\">GD AP/coh</th>");
    }
    html.push_str("</tr></thead>\n<tbody>\n");
    let mut score_rows = Vec::new();
    let loss_values: Vec<_> = modes
        .iter()
        .map(|mode| mode.prepared.graph["metadata"]["loss_type"].clone())
        .collect();
    for mode in modes {
        let metadata = &mode.prepared.graph["metadata"];
        let pre = number_or_zero(&metadata["pre_score"]);
        let post = number_or_zero(&metadata["post_score"]);
        let improvement = if pre > 0.0 {
            (pre - post) / pre * 100.0
        } else {
            0.0
        };
        score_rows.push(json!({"label":mode_label(&mode.name),"pre":pre,"post":post}));
        let color = mode_color(&mode.name);
        let loss = metadata["loss_type"].as_str();
        let loss_cell = loss.map_or_else(
            || "<td style=\"color:#999\">-</td>".to_owned(),
            |value| format!("<td>{}</td>", crate::report_helpers::escape_html(value)),
        );
        let epa = crate::evidence_sections::evidence_section(&json!({
            "builder":"epa_summary","metadata":metadata
        }))?;
        let (epa_pre, epa_post) = epa.as_array().map_or((None, None), |values| {
            (finite_option(values.first()), finite_option(values.get(1)))
        });
        let epa_cells = match (epa_pre, epa_post) {
            (Some(before), Some(after)) => {
                let delta = after - before;
                let color = if delta >= 0.0 { "#2ecc71" } else { "#e74c3c" };
                format!("<td>{before:.2}</td><td>{after:.2}</td><td style=\"color:{color};font-weight:600\">{delta:+.2}</td>")
            }
            _ => "<td style=\"color:#999\">-</td><td style=\"color:#999\">-</td><td style=\"color:#999\">-</td>".into(),
        };
        let mut row = format!(
            "<tr><td style=\"color:{color};font-weight:600\">{}</td>{loss_cell}<td>{pre:.4}</td><td>{post:.4}</td><td class=\"improvement\">{improvement:.1}%</td>{epa_cells}",
            crate::report_helpers::escape_html(mode_label(&mode.name))
        );
        if has_auto {
            let counts = mode
                .figure_context
                .get("eq_filter_counts")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let eq_summary =
                crate::eq_sections::eq_section_html("summary", &json!({"counts":counts}))
                    .map_err(|error| error.to_string())?;
            row.push_str(&eq_summary);
        }
        if has_gd {
            let gd = crate::evidence_sections::evidence_section(&json!({
                "builder":"group_delay","metadata":metadata
            }))?;
            row.push_str(gd.as_str().unwrap_or_default());
        }
        row.push_str("</tr>\n");
        html.push_str(&row);
    }
    html.push_str("</tbody></table>\n");
    push_html_section(sections, &html, false, None);

    if losses.len() >= 2 {
        let losses = losses.into_iter().collect::<Vec<_>>().join(", ");
        push_html_section(
            sections,
            &format!(
                "<p style=\"color:#555;font-size:0.9em;margin-top:10px;\">ℹ This report mixes runs that minimized different loss functions ({losses}). The <strong>Pre flat-loss</strong> and <strong>Post flat-loss</strong> columns are <em>always</em> computed via the flat-loss helper (<code>compute_flat_loss</code> over <code>[min_freq, max_freq]</code>), regardless of which objective each mode actually minimized — this keeps every mode on the same scale so the columns answer <em>“how flat is the response?”</em> for all modes. For <em>perceptual</em> outcomes across loss types, the <strong>EPA Pref</strong> columns are the right place to look.</p>\n"
            ),
            false,
            None,
        );
    }
    if has_gd {
        push_html_section(
            sections,
            "<p style=\"color:#555;font-size:0.9em;margin-top:10px;\">GD columns come from <code>metadata.group_delay</code>. A non-success advisory is expected for recordings that do not carry coherence or independent sweep realisations; it means the production safety gates downgraded or skipped the requested GD path.</p>\n",
            false,
            None,
        );
    }
    if has_auto {
        push_html_section(
            sections,
            "<p style=\"color:#555;font-size:0.9em;margin-top:10px;\">Auto columns show emitted EQ filter counts from the output JSON. Resolved automatic Q and gain bounds are logged by roomeq during the run but are not currently persisted in comparison JSON.</p>\n",
            false,
            None,
        );
    }
    let score = crate::comparison_tables::score_comparison_figure(&json!({
        "scores":score_rows,
        "loss_types":loss_values,
        "tab":Value::Null
    }));
    sections.push(score);
    Ok(())
}

fn append_comparison_channel(
    channel: &str,
    modes: &[ModeInput],
    sections: &mut Vec<Value>,
    preparation_status: &mut Vec<Value>,
) -> Result<(), String> {
    let is_lr = channel == "L+R";
    let mode_curves = modes
        .iter()
        .map(|mode| channel_curves(mode, channel))
        .collect::<Vec<_>>();
    let includes_route = !is_lr && modes.iter().any(|mode| has_redirected_route(mode, channel));
    let source_label = if includes_route {
        format!("Logical source {channel} (physical {channel} + redirected bass)")
    } else if is_lr {
        "Logical source L+R".into()
    } else {
        format!("Logical source {channel}")
    };
    let tab = channel;
    let heading = if is_lr {
        "<h2>Logical source L+R</h2>\n<p>Complex sums require valid per-microphone clock evidence and measured phase. Otherwise the plot uses a magnitude-only power sum, which is not a coherent pressure prediction.</p>\n".to_owned()
    } else {
        format!(
            "<h2>{}</h2>\n",
            crate::report_helpers::escape_html(&source_label)
        )
    };
    push_html_section(sections, &heading, false, Some(tab));
    if includes_route {
        push_html_section(
            sections,
            &format!(
                "<p style=\"color:#666;font-size:0.9em;margin:-10px 0 15px 0;\">The response traces are the coherent microphone prediction for this input source: its high-passed physical speaker output plus its low-passed route through the physical subwoofer. They are not the isolated {channel} loudspeaker output.</p>"
            ),
            false,
            Some(tab),
        );
    }

    let mut selected = Vec::new();
    for (mode, curves) in modes.iter().zip(mode_curves) {
        if is_lr && let Some(reason) = mode.figure_context["lr_fallback_reason"].as_str() {
            push_html_section(
                sections,
                &format!(
                    "<p><strong>{}: magnitude-only fallback.</strong> {}</p>\n",
                    crate::report_helpers::escape_html(&mode.name),
                    crate::report_helpers::escape_html(reason)
                ),
                false,
                Some(tab),
            );
        }
        if curves.initial.is_some() || curves.final_curve.is_some() {
            selected.push((mode, curves));
        }
    }
    if selected.is_empty() {
        push_html_section(
            sections,
            "<p>No data for this channel.</p>\n",
            false,
            Some(tab),
        );
        return Ok(());
    }

    let mut comparison_target = if is_lr {
        select_lr_target(&selected)
    } else {
        select_comparison_target(&selected)
    };
    if !is_lr && let Some(offset) = global_target_offset(modes) {
        comparison_target = comparison_target.map(|curve| offset_curve(&curve, offset));
    }
    append_overlay_figures(
        channel,
        tab,
        &source_label,
        &selected,
        comparison_target.as_ref(),
        sections,
        preparation_status,
    )?;
    append_phase_figures(channel, tab, &selected, sections);
    append_group_delay_figures(channel, tab, &selected, sections, preparation_status);
    append_ir_figures(channel, tab, &selected, sections);
    append_mode_details(
        channel,
        tab,
        &selected,
        comparison_target.as_ref(),
        sections,
    );
    append_eq_figure(channel, tab, &selected, sections);
    append_epa_table(channel, &selected, sections)?;
    Ok(())
}

fn channel_curves(mode: &ModeInput, channel_name: &str) -> ChannelCurves {
    if channel_name == "L+R" {
        let lr = &mode.figure_context["lr_channel"];
        return ChannelCurves {
            initial: nonnull(lr.get("initial_curve")),
            final_curve: nonnull(lr.get("final_curve")),
            target: None,
            pre_ir: nonnull(lr.get("pre_ir")),
            post_ir: nonnull(lr.get("post_ir")),
            eq_response: nonnull(lr.get("eq_response")),
        };
    }
    let channel = &mode.prepared.graph["channels"][channel_name];
    let baseline = if mode.prepared.replay.is_some()
        && channel["drivers"]
            .as_array()
            .is_some_and(|drivers| !drivers.is_empty())
    {
        mode.prepared
            .replay
            .as_ref()
            .and_then(|replay| replay.context.get("channel_baselines"))
            .and_then(|items| items.get(channel_name))
            .filter(|state| state["status"] == "available")
            .and_then(|state| nonnull(state.get("curve")))
    } else {
        nonnull(channel.get("initial_curve"))
    };
    let final_curve = source_curve(mode, channel_name);
    let target = if mode.prepared.target_snapshot["state"] == "available" {
        nonnull(mode.prepared.target_snapshot.get("target")).and_then(|target| {
            if crate::target_overlay::has_lfe_lowpass_route(&mode.prepared.graph, channel_name) {
                comparison_routed_target(mode, channel_name, &target, final_curve.as_ref())
            } else {
                Some(target)
            }
        })
    } else if mode.prepared.target_snapshot["state"] == "unavailable" {
        None
    } else {
        nonnull(channel.get("target_curve"))
    };
    let context = &mode.figure_context["channels"][channel_name];
    let eq_response = context
        .get("eq_response")
        .and_then(|value| nonnull(Some(value)))
        .or_else(|| block_eq_response(mode, channel_name));
    ChannelCurves {
        initial: baseline,
        final_curve,
        target,
        pre_ir: nonnull(context.get("pre_ir")).or_else(|| nonnull(channel.get("pre_ir"))),
        post_ir: nonnull(context.get("post_ir")).or_else(|| nonnull(channel.get("post_ir"))),
        eq_response: eq_response.or_else(|| nonnull(channel.get("eq_response"))),
    }
}

fn comparison_routed_target(
    mode: &ModeInput,
    channel_name: &str,
    target: &Value,
    reference: Option<&Value>,
) -> Option<Value> {
    let reference = reference?;
    let mut graph = mode.prepared.graph.clone();
    if let Some(channel) = graph
        .get_mut("channels")
        .and_then(Value::as_object_mut)
        .and_then(|channels| channels.get_mut(channel_name))
        .and_then(Value::as_object_mut)
    {
        // The comparison target is the loaded file snapshot; a graph-local absolute
        // target must not bypass its saved route crossover on this path.
        channel.remove("target_curve");
    }
    let references = json!({channel_name:reference});
    crate::target_overlay::build_target_overlay_curves_with_status(
        &graph,
        &references,
        Some(target),
        mode.options.route_crossover_db.as_ref(),
        mode.options.route_crossover_status.as_ref(),
    )
    .get(channel_name)
    .cloned()
}

fn block_eq_response(mode: &ModeInput, channel_name: &str) -> Option<Value> {
    let block = mode
        .channel_blocks
        .iter()
        .find(|block| block["label"] == channel_name)?;
    let figures = block["groups"]["speakers"].as_array()?;
    for section in figures {
        let Some(series) = section["figure"]["series"].as_array() else {
            continue;
        };
        let traces = series
            .iter()
            .filter(|trace| trace["y_axis"].as_u64() == Some(1))
            .map(|trace| json!({"freq":trace["x"],"spl":trace["y"]}))
            .collect::<Vec<_>>();
        if let Some(trace) = traces.into_iter().next() {
            return Some(trace);
        }
    }
    None
}

fn append_overlay_figures(
    channel: &str,
    tab: &str,
    source_label: &str,
    selected: &[(&ModeInput, ChannelCurves)],
    target: Option<&Value>,
    sections: &mut Vec<Value>,
    preparation_status: &mut Vec<Value>,
) -> Result<(), String> {
    let initial = selected
        .iter()
        .find_map(|(_, curves)| curves.initial.as_ref());
    let mut overview_series = Vec::new();
    let mut all_curves = Vec::new();
    if let Some(curve) = initial {
        all_curves.push(curve.clone());
        if let Some(smoothed) =
            crate::acoustic_curves::smooth_curve(curve, DEFAULT_SMOOTHING_OCTAVES)
        {
            overview_series.push(series(
                "Before EQ",
                &smoothed,
                "rgba(200, 200, 200, 0.6)",
                2.0,
                "solid",
            ));
        }
    }
    let mut bass_series = overview_series.clone();
    let mut any_target = false;
    for (mode, curves) in selected {
        if let Some(final_curve) = &curves.final_curve {
            all_curves.push(final_curve.clone());
            if let Some(smoothed) =
                crate::acoustic_curves::smooth_curve(final_curve, DEFAULT_SMOOTHING_OCTAVES)
            {
                let trace = series(
                    mode_label(&mode.name),
                    &smoothed,
                    mode_color(&mode.name),
                    2.0,
                    "solid",
                );
                overview_series.push(trace.clone());
                bass_series.push(trace);
            }
        }
    }
    if let Some(target) = target.filter(|curve| valid_curve(curve)) {
        all_curves.push(target.clone());
        overview_series.push(series(
            "Target",
            target,
            "rgba(40, 40, 40, 0.9)",
            2.0,
            "dot",
        ));
        any_target = true;
    }
    let (min, max) = crate::report_data::compute_y_range(&Value::Array(all_curves));
    let descriptors = json!({"figures":[{
        "title":format!("{source_label}: Mode Comparison"),
        "x":axis("Frequency (Hz)","log",Some(20.0),Some(20_000.0)),
        "y":axis("SPL (dB)","linear",Some(min),Some(max)),
        "tab":tab,"series":overview_series
    }]});
    sections.extend(
        crate::comparison_figures::comparison_figure_sections(&descriptors)
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );

    let ref_curve = selected
        .iter()
        .find_map(|(_, curves)| curves.final_curve.as_ref())
        .or(initial);
    let average = ref_curve
        .map(|curve| crate::report_data::average_spl(curve, BASS_MIN_HZ, BASS_MAX_HZ))
        .unwrap_or(0.0);
    let bass = json!({"figures":[{
        "title":format!("{source_label}: Bass (20-500 Hz)"),
        "x":axis("Frequency (Hz)","log",Some(BASS_MIN_HZ),Some(BASS_MAX_HZ)),
        "y":axis("SPL (dB)","linear",Some(average-25.0),Some(average+25.0)),
        "tab":tab,"series":bass_series
    }]});
    sections.extend(
        crate::comparison_figures::comparison_figure_sections(&bass)
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    let has_route_target_status = preparation_status
        .iter()
        .any(|item| item["section"] == "target_overlay" && item["channel"] == channel);
    if !any_target && !is_lr_channel(channel) && !has_route_target_status {
        preparation_status.push(json!({"section":"comparison_target","channel":channel,"status":"unavailable","reason":"no saved absolute target curve is available"}));
    }
    Ok(())
}

fn append_phase_figures(
    channel: &str,
    tab: &str,
    selected: &[(&ModeInput, ChannelCurves)],
    sections: &mut Vec<Value>,
) {
    let has_phase = selected.iter().any(|(_, curves)| {
        curves
            .initial
            .as_ref()
            .is_some_and(|curve| curve["phase"].is_array())
            || curves
                .final_curve
                .as_ref()
                .is_some_and(|curve| curve["phase"].is_array())
    });
    if !has_phase {
        return;
    }
    let mut figures = Vec::new();
    for (mode, curves) in selected {
        let mut traces = Vec::new();
        for (curve, label, color, width) in [
            (
                curves.initial.as_ref(),
                "Before EQ",
                "rgba(200, 200, 200, 0.6)",
                1.5,
            ),
            (
                curves.final_curve.as_ref(),
                mode_label(&mode.name),
                mode_color(&mode.name),
                2.0,
            ),
        ] {
            if let Some(curve) = curve {
                let frequency = curve["freq"].as_array().cloned().unwrap_or_default();
                let phase = curve["phase"].as_array().cloned().unwrap_or_default();
                if !phase.is_empty() {
                    let wrapped = phase
                        .iter()
                        .map(|value| {
                            value
                                .as_f64()
                                .filter(|value| value.is_finite())
                                .map(|value| json!((value + 180.0).rem_euclid(360.0) - 180.0))
                                .unwrap_or(Value::Null)
                        })
                        .collect::<Vec<_>>();
                    traces.push(json!({"name":label,"x":frequency,"y":wrapped,"color":color,"width":width,"dash":"solid"}));
                }
            }
        }
        figures.push(json!({
            "title":format!("{channel}: Phase Before / After EQ — {}",mode_label(&mode.name)),
            "x":axis("Frequency (Hz)","log",Some(20.0),Some(20_000.0)),
            "y":axis("Phase (°)","linear",None,None),"tab":tab,"series":traces
        }));
    }
    sections.extend(
        crate::comparison_figures::comparison_figure_sections(&json!({"figures":figures}))
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
}

fn append_group_delay_figures(
    channel: &str,
    tab: &str,
    selected: &[(&ModeInput, ChannelCurves)],
    sections: &mut Vec<Value>,
    status: &mut Vec<Value>,
) {
    let has_context = selected
        .iter()
        .any(|(mode, _)| !mode.figure_context["group_delay"][channel].is_null());
    if !has_context {
        status.push(json!({"section":"comparison_group_delay","channel":channel,"status":"unavailable","reason":"canonical group-delay traces were not supplied"}));
        return;
    }
    let all_values = selected
        .iter()
        .flat_map(|(mode, _)| {
            ["before", "after"].into_iter().flat_map(|kind| {
                mode.figure_context["group_delay"][channel][kind]["spl"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64)
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let (min, max) = percentile_range(&all_values);
    let mut figures = Vec::new();
    for (mode, _) in selected {
        let traces = &mode.figure_context["group_delay"][channel];
        let mut trace_values = Vec::new();
        for (key, name, color, width) in [
            ("before", "Before EQ", "rgba(200, 200, 200, 0.6)", 1.5),
            ("after", mode_label(&mode.name), mode_color(&mode.name), 2.0),
        ] {
            if let Some(curve) = traces.get(key).filter(|curve| valid_curve(curve)) {
                trace_values.push(series(name, curve, color, width, "solid"));
            }
        }
        figures.push(json!({
            "title":format!("{channel}: Group Delay Before / After EQ — {}",mode_label(&mode.name)),
            "x":axis("Frequency (Hz)","log",Some(20.0),Some(20_000.0)),
            "y":axis("Group Delay (ms)","linear",min,max),"tab":tab,"series":trace_values
        }));
    }
    sections.extend(
        crate::comparison_figures::comparison_figure_sections(&json!({"figures":figures}))
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    status.push(json!({"section":"comparison_group_delay","channel":channel,"status":"available","reason":"canonical group-delay traces were supplied"}));
}

fn append_ir_figures(
    channel: &str,
    tab: &str,
    selected: &[(&ModeInput, ChannelCurves)],
    sections: &mut Vec<Value>,
) {
    let has_ir = selected
        .iter()
        .any(|(_, curves)| curves.pre_ir.is_some() || curves.post_ir.is_some());
    if !has_ir {
        return;
    }
    let mut figures = Vec::new();
    for (mode, curves) in selected {
        let mut traces = Vec::new();
        for (ir, label, color) in [
            (
                curves.pre_ir.as_ref(),
                "Before EQ",
                "rgba(200, 200, 200, 0.6)",
            ),
            (
                curves.post_ir.as_ref(),
                mode_label(&mode.name),
                mode_color(&mode.name),
            ),
        ] {
            if let Some(ir) = ir {
                let (times, amplitudes) =
                    crate::plot_sampling::display_samples(&ir["time_ms"], &ir["amplitude"]);
                traces.push(json!({"name":label,"x":times,"y":amplitudes,"color":color,"width":1.0,"dash":"solid"}));
            }
        }
        figures.push(json!({"title":format!("{channel}: Impulse Response Before / After EQ — {}",mode_label(&mode.name)),"x":axis("Time (ms)","linear",Some(0.0),Some(100.0)),"y":axis("Amplitude","linear",Some(-1.1),Some(1.1)),"tab":tab,"series":traces}));
    }
    sections.extend(
        crate::comparison_figures::comparison_figure_sections(&json!({"figures":figures}))
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
}

fn append_mode_details(
    channel: &str,
    tab: &str,
    selected: &[(&ModeInput, ChannelCurves)],
    target: Option<&Value>,
    sections: &mut Vec<Value>,
) {
    let mut all = Vec::new();
    for (_, curves) in selected {
        all.extend(
            curves
                .initial
                .iter()
                .chain(curves.final_curve.iter())
                .cloned(),
        );
    }
    if let Some(target) = target {
        all.push(target.clone());
    }
    let (min, max) = crate::report_data::compute_y_range(&Value::Array(all));
    let mut figures = Vec::new();
    for (mode, curves) in selected {
        let mut traces = Vec::new();
        for (curve, name, color, width) in [
            (
                curves.initial.as_ref(),
                "Before EQ",
                "rgba(200, 200, 200, 0.6)",
                1.5,
            ),
            (
                curves.final_curve.as_ref(),
                mode_label(&mode.name),
                mode_color(&mode.name),
                2.0,
            ),
            (target, "Target", "rgba(40, 40, 40, 0.9)", 1.5),
        ] {
            if let Some(curve) = curve {
                let curve = if name == "Target" {
                    curve.clone()
                } else {
                    crate::acoustic_curves::smooth_curve(curve, DEFAULT_SMOOTHING_OCTAVES)
                        .unwrap_or_else(|| curve.clone())
                };
                traces.push(series(
                    name,
                    &curve,
                    color,
                    width,
                    if name == "Target" { "dot" } else { "solid" },
                ));
            }
        }
        figures.push(json!({"title":format!("{}: {channel} Per-Mode Detail",mode_label(&mode.name)),"x":axis("Frequency (Hz)","log",Some(20.0),Some(20_000.0)),"y":axis("SPL (dB)","linear",Some(min),Some(max)),"tab":tab,"series":traces}));
    }
    sections.extend(
        crate::comparison_figures::comparison_figure_sections(&json!({"figures":figures}))
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
}

fn append_eq_figure(
    channel: &str,
    tab: &str,
    selected: &[(&ModeInput, ChannelCurves)],
    sections: &mut Vec<Value>,
) {
    let mut traces = Vec::new();
    for (mode, curves) in selected {
        let curve = curves.eq_response.as_ref().or_else(|| {
            mode.figure_context["eq_response"][channel]
                .as_object()
                .map(|_| &mode.figure_context["eq_response"][channel])
        });
        if let Some(curve) = curve.filter(|curve| valid_curve(curve)) {
            traces.push(series(
                mode_label(&mode.name),
                curve,
                mode_color(&mode.name),
                2.0,
                "solid",
            ));
        }
    }
    if traces.is_empty() {
        return;
    }
    let mut series = traces;
    series.push(json!({"name":"0 dB","x":[20.000000000000004,20000.000000000004],"y":[0.0,0.0],"color":"rgba(150, 150, 150, 0.5)","width":1.0,"dash":"dash"}));
    let figure = json!({"figures":[{"title":format!("EQ Response Comparison: {channel}"),"x":axis("Frequency (Hz)","log",Some(20.0),Some(20_000.0)),"y":axis("Gain (dB)","linear",Some(-15.0),Some(15.0)),"tab":tab,"series":series}]});
    sections.extend(
        crate::comparison_figures::comparison_figure_sections(&figure)
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
}

fn append_epa_table(
    channel: &str,
    selected: &[(&ModeInput, ChannelCurves)],
    sections: &mut Vec<Value>,
) -> Result<(), String> {
    let entries = selected.iter().filter_map(|(mode, _)| {
        let epa = &mode.prepared.graph["metadata"]["epa_per_channel"][channel];
        if epa.is_null() || (epa["pre"].is_null() && epa["post"].is_null()) { return None; }
        Some(json!({"mode_label":mode_label(&mode.name),"pre":epa.get("pre").cloned().unwrap_or_else(|| json!({})) ,"post":epa.get("post").cloned().unwrap_or_else(|| json!({}))}))
    }).collect::<Vec<_>>();
    if entries.is_empty() {
        return Ok(());
    }
    let table =
        crate::comparison_tables::epa_comparison_tables(&json!({"channels":{channel:entries}}))?;
    push_html_section(
        sections,
        table[channel].as_str().unwrap_or_default(),
        false,
        Some(channel),
    );
    Ok(())
}

fn select_comparison_target(selected: &[(&ModeInput, ChannelCurves)]) -> Option<Value> {
    selected
        .iter()
        .find_map(|(_, curves)| curves.target.clone())
}

fn select_lr_target(selected: &[(&ModeInput, ChannelCurves)]) -> Option<Value> {
    let main_target = selected.iter().find_map(|(mode, _)| {
        let left = channel_curves(mode, "L").target;
        let right = channel_curves(mode, "R").target;
        left.or(right)
    })?;
    let lr_curve = selected
        .iter()
        .find_map(|(_, curves)| curves.final_curve.as_ref().or(curves.initial.as_ref()))?;
    crate::target_overlay::shift_target_to_reference_band_mean(
        &main_target,
        lr_curve,
        TARGET_MATCH_LOW_HZ,
        TARGET_MATCH_HIGH_HZ,
    )
}

fn source_curve(mode: &ModeInput, channel: &str) -> Option<Value> {
    if let Some(replay) = &mode.prepared.replay {
        return replay
            .context
            .get("routed_sources")
            .and_then(|items| items.get(channel))
            .filter(|state| state["status"] == "available")
            .and_then(|state| nonnull(state.get("curve")));
    }
    mode.options
        .legacy_source_curves
        .as_ref()?
        .get(channel)
        .and_then(|curve| nonnull(Some(curve)))
}

fn global_target_offset(modes: &[ModeInput]) -> Option<f64> {
    for mode in modes {
        let left = channel_curves(mode, "L");
        let right = channel_curves(mode, "R");
        let target = left.target.as_ref().or(right.target.as_ref());
        if let Some(offset) = crate::target_overlay::global_target_offset_for_pair(
            left.final_curve.as_ref().or(left.initial.as_ref()),
            right.final_curve.as_ref().or(right.initial.as_ref()),
            target,
            TARGET_MATCH_LOW_HZ,
            TARGET_MATCH_HIGH_HZ,
        ) {
            return Some(offset);
        }
    }
    None
}

fn offset_curve(curve: &Value, offset: f64) -> Value {
    let mut result = curve.clone();
    if let Some(levels) = result.get_mut("spl").and_then(Value::as_array_mut) {
        for level in levels {
            if let Some(value) = level.as_f64() {
                *level = json!(value + offset);
            }
        }
    }
    result
}

fn has_redirected_route(mode: &ModeInput, source: &str) -> bool {
    mode.prepared.graph["metadata"]["bass_management"]["routing_graph"]["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|route| {
            route["source_channel"] == source
                && route["route_kind"] == "redirected_bass_lowpass_to_sub"
        })
}

fn push_html_section(sections: &mut Vec<Value>, html: &str, flat: bool, tab: Option<&str>) {
    if html.is_empty() {
        return;
    }
    let mut section = json!({"kind":"html","html":html,"tab":tab});
    if flat {
        section["flat"] = json!(true);
    }
    sections.push(section);
}

fn series(name: &str, curve: &Value, color: impl Into<Value>, width: f64, dash: &str) -> Value {
    json!({"name":name,"x":curve.get("freq").cloned().unwrap_or_else(|| json!([])),"y":curve.get("spl").cloned().unwrap_or_else(|| json!([])),"color":color.into(),"width":width,"dash":dash})
}

fn axis(label: &str, scale: &str, min: Option<f64>, max: Option<f64>) -> Value {
    json!({"label":label,"scale":scale,"min":min,"max":max})
}

fn valid_curve(curve: &Value) -> bool {
    let (Some(freq), Some(spl)) = (curve["freq"].as_array(), curve["spl"].as_array()) else {
        return false;
    };
    !freq.is_empty()
        && freq.len() == spl.len()
        && freq
            .iter()
            .chain(spl)
            .all(|value| value.as_f64().is_some_and(f64::is_finite))
}

fn nonnull(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

fn number_or_zero(value: &Value) -> f64 {
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

fn finite_option(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn percentile_range(values: &[f64]) -> (Option<f64>, Option<f64>) {
    if values.is_empty() {
        return (None, None);
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let low = values[((values.len() as f64 * 0.02) as usize).min(values.len() - 1)];
    let high = values[((values.len() as f64 * 0.98) as usize).min(values.len() - 1)];
    let margin = (high - low) * 0.15;
    (Some(low - margin), Some(high + margin))
}

fn mode_color(name: &str) -> &'static str {
    match name {
        "iir" => "#1f77b4",
        "iir_epa" => "#ff7f0e",
        "fir" => "#2ca02c",
        "fir_epa" => "#d62728",
        "hybrid" => "#9467bd",
        "hybrid_epa" => "#8c564b",
        "mixed_phase" => "#e377c2",
        "mixed_phase_epa" => "#17becf",
        "iir_auto_filters" => "#393b79",
        "iir_auto_bounds" => "#637939",
        "iir_auto_all" => "#8c6d31",
        "mixed_phase_auto_all" => "#843c39",
        "iir_gd_safety_gate" => "#ad494a",
        "iir_gd_delay_only" => "#bcbd22",
        "iir_gd_fixed_allpass" => "#7f7f7f",
        "iir_gd_adaptive_allpass" => "#1f9e89",
        "fir_gd_phase_linear" => "#ff9896",
        "mixed_phase_gd" => "#c5b0d5",
        _ => "#888888",
    }
}

fn mode_label(name: &str) -> &str {
    match name {
        "iir" => "IIR (flat)",
        "iir_epa" => "IIR (EPA)",
        "fir" => "FIR (flat)",
        "fir_epa" => "FIR (EPA)",
        "hybrid" => "Hybrid (flat)",
        "hybrid_epa" => "Hybrid (EPA)",
        "mixed_phase" => "MixedPhase (flat)",
        "mixed_phase_epa" => "MixedPhase (EPA)",
        "iir_auto_filters" => "IIR auto filters",
        "iir_auto_bounds" => "IIR auto bounds",
        "iir_auto_all" => "IIR auto all",
        "mixed_phase_auto_all" => "MixedPhase auto all",
        "iir_gd_safety_gate" => "IIR GD safety gate",
        "iir_gd_delay_only" => "IIR GD delay-only",
        "iir_gd_fixed_allpass" => "IIR GD fixed AP",
        "iir_gd_adaptive_allpass" => "IIR GD adaptive AP",
        "fir_gd_phase_linear" => "FIR GD target",
        "mixed_phase_gd" => "MixedPhase + GD",
        _ => name,
    }
}

fn is_lr_channel(channel: &str) -> bool {
    channel == "L+R"
}

#[cfg(test)]
mod tests {
    use super::build_comparison_report;
    use serde_json::{Value, json};

    #[test]
    fn comparison_leads_with_delivered_scores_and_keeps_optimizer_training_separate() {
        let rows: Vec<_> = [1.0, 2.0, 9.0].into_iter().enumerate().map(|(index, pre)| json!({
            "seat_index":index,"physical_outputs":["L"],"score":{
                "curve_index":index,"logical_input":"L","sample_count":128,
                "frequency_grid_sha256":"a".repeat(64),"baseline_samples_sha256":"b".repeat(64),
                "target_samples_sha256":"c".repeat(64),"evaluated_band_hz":[20.0,20000.0],
                "normalized_to_pre":true,"level_offset_db":80.0,"post_reference_gain_db":0.0,
                "post_covers_baseline_grid":true,"pre_weighted_rms_db":pre,
                "post_weighted_rms_db":1.0,"improvement_db":pre-1.0
            }
        })).collect();
        let graph = json!({"channels":{},"metadata":{
            "pre_score":5.8,"post_score":3.2,"correction_acceptance":{
                "outcome":"safe_fallback","acoustic_quality":{"training":{
                    "curve_count":3,"configured_baseline":{
                        "method":"configured_routing_unprocessed_captures_v1",
                        "playback_graph_sha256":"d".repeat(64),"unavailable":[],"residuals":rows
                    }
                }}
            }
        }});
        let report = build_comparison_report(&json!({"modes":[{
            "name":"iir","prepared":{"schema_version":1,"graph":graph,
                "target_snapshot":{"schema_version":1,"state":"not_configured","target":null}},
            "options":{},"figure_context":{}
        }]}))
        .unwrap();
        let payload = report["payload"].to_string();
        let delivered = payload.find("Delivered playback").unwrap();
        let training = payload.find("Optimizer training").unwrap();
        assert!(delivered < training);
        assert!(payload.contains("4.0000</td><td>1.0000</td><td>+3.0000"));
        assert!(payload.contains("5.8000</td><td>3.2000"));
        assert!(payload.contains("safe_fallback"));
    }

    #[test]
    fn comparison_does_not_show_unshaped_target_for_unavailable_lfe_route() {
        let graph = json!({
            "channels":{"LFE":{"initial_curve":{"freq":[40.0,80.0,160.0],"spl":[70.0,70.0,70.0]}}},
            "metadata":{"bass_management":{"routing_graph":{"routes":[{
                "source_channel":"LFE","route_kind":"lfe_lowpass_to_sub","low_pass_hz":80.0
            }]}}}
        });
        let output = build_comparison_report(&json!({"modes":[{
            "name":"No playback rate",
            "prepared":{
                "schema_version":1,
                "graph":graph,
                "target_snapshot":{"schema_version":1,"state":"available","target":{"freq":[20.0,20000.0],"spl":[75.0,75.0]}}
            },
            "options":{
                "legacy_source_curves":{"LFE":{"freq":[40.0,80.0,160.0],"spl":[70.0,70.0,70.0]}},
                "route_crossover_db":{},
                "route_crossover_status":{"LFE":{"status":"unavailable","reason":"explicit sample rate is missing"}}
            },
            "figure_context":{}
        }]})).expect("comparison composer preserves route unavailability");
        assert!(
            output["preparation_status"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| {
                    item["section"] == "target_overlay"
                        && item["channel"] == "LFE"
                        && item["status"] == "unavailable"
                        && item["reason"] == "explicit sample rate is missing"
                })
        );
        assert!(
            !output["preparation_status"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| { item["section"] == "comparison_target" && item["channel"] == "LFE" })
        );
        assert!(
            output["payload"]["sections"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|section| section["figure"]["series"].as_array().into_iter().flatten())
                .all(|series| series["name"] != "Target")
        );
    }

    #[test]
    fn complete_comparison_payload_matches_captured_python_golden() {
        let golden: Value = serde_json::from_str(include_str!(
            "../../../scripts/tests/fixtures/comparison_payload_python_goldens.json"
        ))
        .expect("Python comparison golden parses");
        let context: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/comparison_prepared_context_python.json"
        ))
        .expect("captured prepared comparison context parses");
        let case = &golden["cases"][0];
        let mut modes = Vec::new();
        for entry in case["modes"].as_array().unwrap() {
            let name = entry[0].as_str().expect("golden mode name");
            let graph = &entry[1];
            let prepared = json!({
                "schema_version": 1,
                "graph": graph,
                "target_snapshot": {"schema_version":1,"state":"not_configured","target":null}
            });
            let captured = context["modes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["mode"] == name)
                .expect("matching captured comparison context");
            modes.push(json!({
                "name": name,
                "prepared": prepared,
                "options": {
                    "legacy_source_curves":captured["source_curves"],
                    "payload_binding":{
                        "verified":false,
                        "reason":"Recomputable payload binding unavailable (legacy or unbound output).",
                        "acceptance_evidence_verified":false,
                        "acceptance_evidence_reason":"acceptance evidence is unavailable"
                    }
                },
                "figure_context": captured,
            }));
        }
        let actual = build_comparison_report(&json!({
            "modes": modes,
            "provenance": case["payload"]["provenance"]
        }))
        .expect("comparison composition succeeds");
        if !json_semantically_equal(&actual["payload"], &case["payload"]) {
            let actual_sections = actual["payload"]["sections"].as_array().unwrap();
            let expected_sections = case["payload"]["sections"].as_array().unwrap();
            let first_difference = actual_sections
                .iter()
                .zip(expected_sections)
                .position(|(left, right)| !json_semantically_equal(left, right));
            let detail = first_difference
                .and_then(|index| actual_sections.get(index).zip(expected_sections.get(index)))
                .and_then(|(left, right)| first_json_difference(left, right, "$"))
                .unwrap_or_else(|| "section counts differ".into());
            panic!(
                "comparison payload differs (actual title {:?}, expected {:?}, actual count {}, expected count {}, first section difference {:?}: actual={} expected={}; first value difference={})",
                actual["payload"]["title"],
                case["payload"]["title"],
                actual_sections.len(),
                expected_sections.len(),
                first_difference,
                first_difference
                    .and_then(|index| actual_sections.get(index))
                    .map(section_label)
                    .unwrap_or_default(),
                first_difference
                    .and_then(|index| expected_sections.get(index))
                    .map(section_label)
                    .unwrap_or_default(),
                detail,
            );
        }
        assert_eq!(
            actual["payload"]["sections"].as_array().unwrap().len(),
            60,
            "golden covers all 60 ordered report sections"
        );
        assert!(actual["preparation_status"].is_array());
    }

    #[test]
    fn unavailable_group_delay_is_reported_without_phase_derivative_fallback() {
        let graph = json!({"channels":{"L":{
            "initial_curve":{"freq":[100.0, 1000.0],"spl":[70.0, 70.0]},
            "final_curve":{"freq":[100.0, 1000.0],"spl":[71.0, 71.0],"phase":[0.0, 90.0]}
        }},"metadata":{}});
        let output = build_comparison_report(&json!({
            "modes":[{
                "name":"iir_auto",
                "prepared":{"schema_version":1,"graph":graph,"target_snapshot":{"schema_version":1,"state":"not_configured","target":null}},
                "options":{"legacy_source_curves":{"L":{"freq":[100.0,1000.0],"spl":[71.0,71.0],"phase":[0.0,90.0]}}},
                "figure_context":{"group_delay":{}}
            }]
        }))
        .expect("comparison composition reports an unavailable trace");
        assert!(
            output["preparation_status"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| {
                    item["section"] == "comparison_group_delay"
                        && item["channel"] == "L"
                        && item["status"] == "unavailable"
                })
        );
        assert!(
            !output["payload"]["sections"]
                .as_array()
                .unwrap()
                .iter()
                .any(|section| {
                    section["kind"] == "figure"
                        && section["figure"]["title"] == "Group Delay Comparison: L"
                })
        );
    }

    fn json_semantically_equal(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Number(left), Value::Number(right)) => left.as_f64() == right.as_f64(),
            (Value::Array(left), Value::Array(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| json_semantically_equal(left, right))
            }
            (Value::Object(left), Value::Object(right)) => {
                left.len() == right.len()
                    && left.iter().all(|(key, value)| {
                        right
                            .get(key)
                            .is_some_and(|other| json_semantically_equal(value, other))
                    })
            }
            _ => left == right,
        }
    }

    fn section_label(section: &Value) -> String {
        if let Some(title) = section["figure"]["title"].as_str() {
            return title.to_owned();
        }
        section["html"]
            .as_str()
            .map(|html| html.chars().take(55).collect())
            .unwrap_or_else(|| section["kind"].as_str().unwrap_or("?").to_owned())
    }

    fn first_json_difference(left: &Value, right: &Value, path: &str) -> Option<String> {
        match (left, right) {
            (Value::Number(left), Value::Number(right)) if left.as_f64() == right.as_f64() => None,
            (Value::Number(_), Value::Number(_)) => Some(path.to_owned()),
            (Value::Array(left), Value::Array(right)) => {
                if left.len() != right.len() {
                    return Some(format!("{path}.length"));
                }
                left.iter()
                    .zip(right)
                    .enumerate()
                    .find_map(|(index, (left, right))| {
                        first_json_difference(left, right, &format!("{path}[{index}]"))
                    })
            }
            (Value::Object(left), Value::Object(right)) => {
                if left.len() != right.len() {
                    return Some(format!("{path}.keys"));
                }
                left.iter().find_map(|(key, value)| {
                    right.get(key).map_or_else(
                        || Some(format!("{path}.{key}")),
                        |other| first_json_difference(value, other, &format!("{path}.{key}")),
                    )
                })
            }
            _ if left == right => None,
            _ => Some(path.to_owned()),
        }
    }
}
