//! Opt-in temporal refinement of an already realized independent-channel FIR.
#[cfg(all(test, feature = "measurement-zarr"))]
mod zarr_test_fixture;
use super::*;
use math_audio_dsp::analysis::{FiniteWindowFirConfig, FiniteWindowFirObjective};
use roomeq_model::{TemporalFirConfig, TemporalIrSidecar, TemporalPartition};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;

struct Evidence {
    identity: String,
    sidecar_sha256: String,
    partition: TemporalPartition,
    objective: FiniteWindowFirObjective,
}

fn invalid(message: impl Into<String>) -> AutoeqError {
    AutoeqError::InvalidMeasurement {
        message: format!("temporal_fir: {}", message.into()),
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn prepared(config: &RoomConfig, sample_rate: f64, tap_count: usize) -> Result<Vec<Evidence>> {
    let Some(strategy) = &config.provenance.temporal_fir else {
        return Ok(Vec::new());
    };
    let rate = crate::ctc::checked_sample_rate(sample_rate)?;
    if rate as f64 != sample_rate || tap_count == 0 {
        return Err(invalid(
            "filter rate must be integral and FIR taps must be configured",
        ));
    }
    if !matches!(
        config.optimizer.processing_mode,
        ProcessingMode::PhaseLinear | ProcessingMode::Hybrid
    ) || config
        .optimizer
        .fir
        .as_ref()
        .is_none_or(|fir| fir.taps != tap_count)
        || !matches!(
            config.speakers.get(&strategy.channel),
            Some(SpeakerConfig::Single(_))
        )
        || config.system.as_ref().is_some_and(|system| {
            system.subwoofers.is_some()
                || system
                    .bass_management
                    .as_ref()
                    .is_some_and(|bass| bass.enabled)
        })
    {
        return Err(invalid(
            "requires an independent single channel with one configured phase_linear or hybrid FIR",
        ));
    }
    if strategy.evidence.is_empty()
        || strategy.delays_seconds.is_empty()
        || strategy.strengths.is_empty()
        || !strategy.minimum_late_improvement_db.is_finite()
        || strategy.minimum_late_improvement_db < 0.0
        || !strategy.maximum_early_change_db.is_finite()
        || strategy.maximum_early_change_db < 0.0
        || !strategy.maximum_spectral_change_db.is_finite()
        || strategy.maximum_spectral_change_db < 0.0
        || strategy
            .minimum_relative_tail_improvement_db
            .is_some_and(|v| !v.is_finite() || v < 0.1)
        || strategy
            .minimum_absolute_late_improvement_db
            .is_some_and(|v| !v.is_finite() || v < 0.1)
        || strategy
            .minimum_early_change_db
            .is_some_and(|v| !v.is_finite() || v < -0.75)
        || strategy
            .maximum_early_increase_db
            .is_some_and(|v| !v.is_finite() || v > 0.25)
        || strategy
            .minimum_early_change_db
            .zip(strategy.maximum_early_increase_db)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        || strategy.minimum_positions.is_some_and(|count| count < 3)
        || strategy
            .delays_seconds
            .iter()
            .any(|d| !d.is_finite() || *d <= 0.0)
        || strategy
            .strengths
            .iter()
            .any(|s| !s.is_finite() || *s <= 0.0 || *s > 1.0)
    {
        return Err(invalid(
            "invalid evidence, candidate grid, or acceptance settings",
        ));
    }
    let mut seen_positions = HashSet::new();
    let mut seen_bytes = HashSet::new();
    let mut seen_samples = HashSet::new();
    let mut seen_records = HashSet::new();
    let mut reference = None;
    let mut evidence = Vec::with_capacity(strategy.evidence.len());
    for item in &strategy.evidence {
        let sidecar_bytes = fs::read(&item.sidecar_path)
            .map_err(|error| invalid(format!("{}: {error}", item.sidecar_path.display())))?;
        if sidecar_bytes.len() > 16 * 1024 * 1024 {
            return Err(invalid("provenance sidecar is too large"));
        }
        let record: autoeq_measurements::MeasurementRecord = serde_json::from_slice(&sidecar_bytes)
            .map_err(|error| invalid(format!("invalid provenance sidecar: {error}")))?;
        let validation = record.validate(autoeq_measurements::ValidationMode::Strict);
        if !validation.is_valid() {
            return Err(invalid(format!(
                "sidecar provenance invalid: {}",
                validation.errors.join("; ")
            )));
        }
        if !seen_records.insert(record.id.clone()) {
            return Err(invalid(
                "measurement record reused across evidence partitions",
            ));
        }
        let metadata: TemporalIrSidecar = serde_json::from_value(
            record
                .provenance
                .extensions
                .get("temporal_ir_v1")
                .cloned()
                .ok_or_else(|| invalid("provenance sidecar lacks temporal_ir_v1"))?,
        )
        .map_err(|error| invalid(format!("invalid temporal_ir_v1: {error}")))?;
        if metadata.channel != strategy.channel
            || metadata.position_id != item.position_id
            || metadata.partition != item.partition
            || metadata.processing_state != "isolated_raw"
            || metadata.stimulus_reference.trim().is_empty()
            || metadata.capture_rate_hz < rate
            || !metadata.capture_rate_hz.is_multiple_of(rate)
        {
            return Err(invalid(
                "IR channel, position, partition, processing state, reference, or sample rate mismatch",
            ));
        }
        if !seen_positions.insert((metadata.partition as u8, metadata.position_id.clone())) {
            return Err(invalid(
                "duplicate position identity within one evidence partition",
            ));
        }
        if reference.get_or_insert_with(|| metadata.stimulus_reference.clone())
            != &metadata.stimulus_reference
        {
            return Err(invalid("IR stimulus references differ"));
        }
        let path = if metadata.wav_path.is_absolute() {
            metadata.wav_path.clone()
        } else {
            item.sidecar_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&metadata.wav_path)
        };
        let bytes =
            fs::read(&path).map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        let digest = sha256(&bytes);
        if digest != metadata.sha256 || !seen_bytes.insert(digest.clone()) {
            return Err(invalid(
                "IR byte digest mismatch or training/held-out source reused",
            ));
        }
        if !record
            .provenance
            .source_artifacts
            .iter()
            .any(|source| source.content_hash.as_deref() == Some(digest.as_str()))
        {
            return Err(invalid(
                "IR WAV is not bound as a source artifact in its provenance sidecar",
            ));
        }
        let wav = hound::WavReader::new(std::io::Cursor::new(&bytes))
            .map_err(|error| invalid(error.to_string()))?;
        if wav.spec().channels != 1 {
            return Err(invalid("saved IR WAV must be mono"));
        }
        let decoded = crate::ctc::read_wav_bytes_channels_f64(
            &bytes,
            &path,
            metadata.capture_rate_hz,
            "temporal IR",
        )?;
        let samples = decoded
            .into_iter()
            .next()
            .ok_or_else(|| invalid("empty IR WAV"))?;
        let sample_bits: Vec<f32> = samples.iter().map(|sample| *sample as f32).collect();
        let sample_digest = sha256(
            &sample_bits
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        if !seen_samples.insert(sample_digest) {
            return Err(invalid(
                "independent evidence repeats identical decoded samples",
            ));
        }
        if let Some(array) = &metadata.orcmeasurement {
            if array.array_path
                != format!(
                    "positions/{}/{}/impulse_response",
                    metadata.position_id, metadata.channel
                )
            {
                return Err(invalid(
                    "orcmeasurement array path does not match position/channel identity",
                ));
            }
            #[cfg(feature = "measurement-zarr")]
            {
                let package = if array.package_path.is_absolute() {
                    array.package_path.clone()
                } else {
                    item.sidecar_path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(&array.package_path)
                };
                let loaded = roomeq_engine::measurement_array::read_orcmeasurement_impulse(
                    &package,
                    &array.array_path,
                    &array.source_digest,
                    &array.logical_sha256,
                )
                .map_err(|error| invalid(format!("orcmeasurement: {error}")))?;
                if loaded.campaign_id != array.campaign_id
                    || loaded.sample_rate_hz != metadata.capture_rate_hz
                    || loaded.processing_state != "isolated_physical_output"
                    || loaded.samples.len() != sample_bits.len()
                    || !loaded
                        .samples
                        .iter()
                        .zip(sample_bits.iter())
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                {
                    return Err(invalid(
                        "orcmeasurement and raw WAV samples or identity differ",
                    ));
                }
            }
            #[cfg(not(feature = "measurement-zarr"))]
            {
                let _ = array;
                return Err(invalid(
                    "orcmeasurement evidence requires measurement-zarr feature",
                ));
            }
        }
        let objective = FiniteWindowFirObjective::new(
            &samples,
            &FiniteWindowFirConfig {
                capture_rate_hz: metadata.capture_rate_hz,
                filter_rate_hz: rate,
                tap_count,
                anchor_sample: metadata.anchor_sample,
                frequencies_hz: strategy.frequencies_hz.clone(),
                frequency_weights: strategy.frequency_weights.clone(),
                window_seconds: strategy.window_seconds,
                starts_seconds: strategy.starts_seconds.clone(),
            },
        )
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        let sidecar_sha256 = sha256(&sidecar_bytes);
        evidence.push(Evidence {
            identity: digest,
            sidecar_sha256,
            partition: item.partition,
            objective,
        });
    }
    if !evidence
        .iter()
        .any(|item| item.partition == TemporalPartition::Training)
        || !evidence
            .iter()
            .any(|item| item.partition == TemporalPartition::HeldOut)
    {
        return Err(invalid(
            "at least one training and one held-out IR are required",
        ));
    }
    if strategy.minimum_positions.is_some_and(|required| {
        seen_positions
            .iter()
            .map(|(_, position)| position)
            .collect::<HashSet<_>>()
            .len()
            < required
    }) {
        return Err(invalid(
            "insufficient independent positions for temporal qualification",
        ));
    }
    Ok(evidence)
}

pub(super) fn validate_evidence(config: &RoomConfig, sample_rate: f64) -> Result<()> {
    if config.provenance.temporal_fir.is_some() {
        let taps = config.optimizer.fir.as_ref().map_or(0, |fir| fir.taps);
        prepared(config, sample_rate, taps)?;
    }
    Ok(())
}

fn acceptable(
    evidence: &[Evidence],
    candidate: &[f64],
    baseline: &[f64],
    strategy: &TemporalFirConfig,
    partition: TemporalPartition,
) -> Option<f64> {
    let (relative_gain, absolute_gain, early) =
        partition_metrics(evidence, candidate, baseline, partition)?;
    (early <= strategy.maximum_early_change_db
        && relative_gain
            >= strategy
                .minimum_relative_tail_improvement_db
                .unwrap_or(strategy.minimum_late_improvement_db)
        && absolute_gain
            >= strategy
                .minimum_absolute_late_improvement_db
                .unwrap_or(strategy.minimum_late_improvement_db)
        && evidence
            .iter()
            .filter(|item| item.partition == partition)
            .all(|item| {
                item.objective
                    .compare(candidate, baseline)
                    .is_ok_and(|comparison| {
                        strategy
                            .minimum_early_change_db
                            .is_none_or(|minimum| comparison.early_change_db >= minimum)
                            && strategy
                                .maximum_early_increase_db
                                .is_none_or(|maximum| comparison.early_change_db <= maximum)
                    })
            }))
    .then_some(relative_gain.min(absolute_gain))
}

fn partition_metrics(
    evidence: &[Evidence],
    candidate: &[f64],
    baseline: &[f64],
    partition: TemporalPartition,
) -> Option<(f64, f64, f64)> {
    let mut relative_gains = Vec::new();
    let mut absolute_gains = Vec::new();
    let mut largest_early: f64 = 0.0;
    for item in evidence.iter().filter(|item| item.partition == partition) {
        let comparison = item.objective.compare(candidate, baseline).ok()?;
        largest_early = largest_early.max(comparison.early_change_db.abs());
        let relative_gain = -comparison
            .relative_tail_changes_db
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let absolute_gain = -comparison
            .late_changes_db
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        if !relative_gain.is_finite() || !absolute_gain.is_finite() {
            return None;
        }
        relative_gains.push(relative_gain);
        absolute_gains.push(absolute_gain);
    }
    Some((
        relative_gains.into_iter().reduce(f64::min)?,
        absolute_gains.into_iter().reduce(f64::min)?,
        largest_early,
    ))
}

fn spectral_safe(
    candidate: &[f64],
    baseline: &[f64],
    rate: f64,
    strategy: &TemporalFirConfig,
    boost_limit: Option<f64>,
) -> bool {
    if !rate.is_finite() || rate <= 0.0 || candidate.len() != baseline.len() || baseline.is_empty()
    {
        return false;
    }
    let Some(length) = baseline
        .len()
        .checked_mul(8)
        .and_then(usize::checked_next_power_of_two)
    else {
        return false;
    };
    let mut planner = rustfft::FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(length);
    let spectrum = |taps: &[f64]| {
        let mut bins = vec![Complex64::new(0.0, 0.0); length];
        for (bin, tap) in bins.iter_mut().zip(taps) {
            bin.re = *tap;
        }
        fft.process(&mut bins);
        bins
    };
    let prior = spectrum(baseline);
    let next = spectrum(candidate);
    prior
        .iter()
        .zip(next.iter())
        .take(length / 2 + 1)
        .all(|(old, new)| {
            let (old, new) = (old.norm(), new.norm());
            if !old.is_finite() || !new.is_finite() || old < 1e-9 || new < 1e-9 {
                return false;
            }
            let delta = 20.0 * (new / old).log10();
            let absolute_boost = 20.0 * new.log10();
            delta.is_finite()
                && delta.abs() <= strategy.maximum_spectral_change_db
                && boost_limit.is_none_or(|limit| absolute_boost <= limit + 1e-6)
        })
}

pub(super) fn apply(
    result: &mut RoomOptimizationResult,
    config: &RoomConfig,
    sample_rate: f64,
    dir: &Path,
    store: &dyn autoeq_artifacts::ArtifactStore,
) -> Result<()> {
    let Some(strategy) = &config.provenance.temporal_fir else {
        return Ok(());
    };
    let name = &strategy.channel;
    let chain = result
        .channels
        .get(name)
        .ok_or_else(|| invalid("target channel has no output chain"))?;
    let convolutions: Vec<_> = chain
        .plugins
        .iter()
        .filter(|plugin| plugin.plugin_type == "convolution")
        .collect();
    if convolutions.len() != 1
        || chain.drivers.is_some()
        || chain
            .plugins
            .iter()
            .any(|plugin| !matches!(plugin.plugin_type.as_str(), "convolution" | "gain"))
    {
        return Err(invalid(
            "target output must own one FIR and optional constant gain only",
        ));
    }
    let filename = convolutions[0]
        .parameters
        .get("filename")
        .and_then(|value| value.as_str())
        .or_else(|| {
            convolutions[0]
                .parameters
                .get("ir_file")
                .and_then(|value| value.as_str())
        })
        .ok_or_else(|| invalid("convolution file identity unavailable"))?;
    let baseline = result
        .channel_results
        .get(name)
        .and_then(|channel| channel.fir_coeffs.clone())
        .ok_or_else(|| invalid("target channel has no retained FIR coefficients"))?;
    let baseline: Vec<f64> = baseline
        .into_iter()
        .map(|tap| (tap as f32) as f64)
        .collect();
    let evidence = prepared(config, sample_rate, baseline.len())?;
    // A comparison needs a valid denominator in every recorded window. An
    // unusable baseline is an input error, not a failed candidate search.
    for item in &evidence {
        item.objective.energy(&baseline).map_err(|error| {
            invalid(format!(
                "baseline has invalid recorded-window energy: {error}"
            ))
        })?;
    }
    let mut selected: Option<(Vec<f64>, f64)> = None;
    for &delay in &strategy.delays_seconds {
        let shift = (delay * sample_rate).round() as usize;
        if shift == 0 || shift >= baseline.len() {
            return Err(invalid("candidate delay falls outside configured FIR taps"));
        }
        for &strength in &strategy.strengths {
            let mut taps = baseline.clone();
            for index in shift..taps.len() {
                taps[index] -= strength * baseline[index - shift];
            }
            // WAV sidecars serialize f32. Evaluate precisely those delivered taps.
            let taps: Vec<f64> = taps.into_iter().map(|tap| (tap as f32) as f64).collect();
            if !spectral_safe(
                &taps,
                &baseline,
                sample_rate,
                strategy,
                config
                    .optimizer
                    .fir
                    .as_ref()
                    .and_then(|fir| fir.max_boost_db),
            ) {
                continue;
            }
            if let Some(training_gain) = acceptable(
                &evidence,
                &taps,
                &baseline,
                strategy,
                TemporalPartition::Training,
            ) && selected
                .as_ref()
                .is_none_or(|(_, best)| training_gain > *best)
            {
                selected = Some((taps, training_gain));
            }
        }
    }
    let mut reason = "no_training_candidate";
    let had_training_candidate = selected.is_some();
    let mut selected_sha256 = None;
    let mut selected_chain_sha256 = None;
    let metrics = selected.as_ref().map(|(taps, _)| {
        (
            partition_metrics(&evidence, taps, &baseline, TemporalPartition::Training),
            partition_metrics(&evidence, taps, &baseline, TemporalPartition::HeldOut),
        )
    });
    let status = if let Some((taps, _)) = selected
        && acceptable(
            &evidence,
            &taps,
            &baseline,
            strategy,
            TemporalPartition::HeldOut,
        )
        .is_some()
    {
        let channel = result
            .channel_results
            .get_mut(name)
            .expect("validated channel");
        let frequencies = &channel.final_curve.freq;
        let prior = roomeq_engine::response::compute_fir_complex_response(
            &baseline,
            frequencies,
            sample_rate,
        );
        let next =
            roomeq_engine::response::compute_fir_complex_response(&taps, frequencies, sample_rate);
        let ratio = next
            .iter()
            .zip(prior.iter())
            .map(|(new, old)| {
                if old.norm() < 1e-9 {
                    None
                } else {
                    Some(*new / *old)
                }
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| invalid("baseline FIR response has an unrepresentable zero"))?;
        let path = dir.join(filename);
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: sample_rate as u32,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let mut writer = hound::WavWriter::new(&mut bytes, spec)
                .map_err(|error| invalid(error.to_string()))?;
            for tap in &taps {
                writer
                    .write_sample(*tap as f32)
                    .map_err(|error| invalid(error.to_string()))?;
            }
            writer
                .finalize()
                .map_err(|error| invalid(error.to_string()))?;
        }
        store.write(&path, &bytes.into_inner())?;
        selected_sha256 = Some(sha256(
            &taps
                .iter()
                .flat_map(|tap| (*tap as f32).to_le_bytes())
                .collect::<Vec<_>>(),
        ));
        channel.final_curve =
            roomeq_engine::response::apply_complex_response(&channel.final_curve, &ratio);
        channel.fir_coeffs = Some(taps);
        if let Some(chain) = result.channels.get_mut(name) {
            chain.final_curve = Some((&channel.final_curve).into());
            selected_chain_sha256 = Some(sha256(
                &serde_json::to_vec(&chain.plugins).map_err(|error| invalid(error.to_string()))?,
            ));
        }
        reason = "held_out_accepted";
        roomeq_model::StageStatus::Applied
    } else {
        if had_training_candidate {
            reason = "held_out_rejected";
        }
        roomeq_model::StageStatus::Skipped
    };
    let config_hash =
        sha256(&serde_json::to_vec(strategy).map_err(|error| invalid(error.to_string()))?);
    result
        .metadata
        .stage_outcomes
        .push(roomeq_model::StageOutcome {
            stage: "temporal_fir".into(),
            status,
            checks: Vec::new(),
            advisories: std::iter::once(reason.to_string())
                .chain(std::iter::once(format!("config_sha256={config_hash}")))
                .chain(selected_sha256.map(|hash| format!("selected_taps_sha256={hash}")))
                .chain(selected_chain_sha256.map(|hash| format!("selected_chain_sha256={hash}")))
                .chain(metrics.into_iter().flat_map(|(training, held)| {
                    [("training", training), ("held_out", held)].into_iter().filter_map(|(label, values)| {
                        values.map(|(relative, absolute, early)| format!("{label}_minimum_relative_tail_improvement_db={relative:.6};minimum_absolute_late_improvement_db={absolute:.6};maximum_absolute_early_change_db={early:.6}"))
                    })
                }))
                .chain(evidence.iter().flat_map(|item| {
                    [
                        format!("ir_sha256={}", item.identity),
                        format!("sidecar_sha256={}", item.sidecar_sha256),
                    ]
                }))
                .collect(),
        });
    Ok(())
}

pub(super) fn verify_final(result: &RoomOptimizationResult, config: &RoomConfig) -> Result<()> {
    let Some(strategy) = &config.provenance.temporal_fir else {
        return Ok(());
    };
    let Some(outcome) = result
        .metadata
        .stage_outcomes
        .iter()
        .rev()
        .find(|item| item.stage == "temporal_fir")
    else {
        return Err(invalid("missing temporal FIR outcome"));
    };
    if outcome.status != roomeq_model::StageStatus::Applied {
        return Ok(());
    }
    let expected = outcome
        .advisories
        .iter()
        .find_map(|item| item.strip_prefix("selected_taps_sha256="))
        .ok_or_else(|| invalid("selected tap identity missing"))?;
    let taps = result
        .channel_results
        .get(&strategy.channel)
        .and_then(|item| item.fir_coeffs.as_ref())
        .ok_or_else(|| invalid("selected FIR was removed after qualification"))?;
    let actual = sha256(
        &taps
            .iter()
            .flat_map(|tap| (*tap as f32).to_le_bytes())
            .collect::<Vec<_>>(),
    );
    if actual != expected {
        return Err(invalid("selected FIR changed after temporal qualification"));
    }
    let expected_chain = outcome
        .advisories
        .iter()
        .find_map(|item| item.strip_prefix("selected_chain_sha256="))
        .ok_or_else(|| invalid("selected chain identity missing"))?;
    let chain = result
        .channels
        .get(&strategy.channel)
        .ok_or_else(|| invalid("selected output chain disappeared"))?;
    let actual_chain =
        sha256(&serde_json::to_vec(&chain.plugins).map_err(|error| invalid(error.to_string()))?);
    if actual_chain != expected_chain {
        return Err(invalid("output chain changed after temporal qualification"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoeq_artifacts::MemoryArtifactStore;
    use autoeq_measurements::{MeasurementOrigin, MeasurementRecord, write_sidecar};
    use roomeq_model::{FirConfig, MeasurementSource, TemporalIrEvidence};

    fn synthetic_curve(tail: f64) -> roomeq_model::Curve {
        let mut curve = crate::test_fixtures::flat_curve();
        curve.spl = curve.freq.mapv(|frequency| {
            let phase = 2.0 * std::f64::consts::PI * frequency * 0.001;
            let magnitude = (1.0 + tail * phase.cos()).hypot(tail * phase.sin());
            80.0 + 20.0 * magnitude.log10()
        });
        curve
    }

    fn fixture(
        held_tail: f32,
    ) -> (
        tempfile::TempDir,
        RoomConfig,
        RoomOptimizationResult,
        MemoryArtifactStore,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = RoomConfig::default();
        config.speakers.insert(
            "L".into(),
            SpeakerConfig::Single(MeasurementSource::InMemory(synthetic_curve(0.5))),
        );
        config.optimizer.processing_mode = ProcessingMode::PhaseLinear;
        config.optimizer.fir = Some(FirConfig {
            taps: 128,
            ..FirConfig::default()
        });
        let mut references = Vec::new();
        for (label, partition, tail) in [
            ("training", TemporalPartition::Training, 0.5_f32),
            ("held", TemporalPartition::HeldOut, held_tail),
        ] {
            let path = dir.path().join(format!("{label}.wav"));
            let mut samples = vec![0.0_f32; 256];
            samples[64] = 1.0;
            samples[112] = tail;
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for sample in samples {
                writer.write_sample(sample).unwrap();
            }
            writer.finalize().unwrap();
            let digest = sha256(&fs::read(&path).unwrap());
            let mut record = MeasurementRecord::from_source_path(
                synthetic_curve(f64::from(tail)),
                MeasurementOrigin::Recording,
                &path,
            )
            .unwrap();
            record.id = format!("synthetic:{label}");
            record.curve =
                serde_json::from_slice(&serde_json::to_vec(&record.curve).unwrap()).unwrap();
            record.provenance.content_hash = record.curve.content_hash().unwrap();
            record.provenance.extensions.insert(
                "temporal_ir_v1".into(),
                serde_json::to_value(TemporalIrSidecar {
                    wav_path: path.clone(),
                    sha256: digest,
                    anchor_sample: 48,
                    processing_state: "isolated_raw".into(),
                    stimulus_reference: "synthetic-sweep".into(),
                    capture_rate_hz: 48_000,
                    channel: "L".into(),
                    position_id: label.into(),
                    partition,
                    orcmeasurement: None,
                })
                .unwrap(),
            );
            references.push(TemporalIrEvidence {
                sidecar_path: write_sidecar(&path, &record).unwrap(),
                position_id: label.into(),
                partition,
            });
        }
        config.provenance.temporal_fir = Some(TemporalFirConfig {
            channel: "L".into(),
            evidence: references,
            frequencies_hz: vec![80.0],
            frequency_weights: vec![1.0],
            window_seconds: 0.0007,
            starts_seconds: vec![0.0, 0.001],
            delays_seconds: vec![0.001],
            strengths: vec![0.4],
            minimum_late_improvement_db: 0.1,
            maximum_early_change_db: 1.0,
            maximum_spectral_change_db: 12.0,
            minimum_relative_tail_improvement_db: None,
            minimum_absolute_late_improvement_db: None,
            minimum_early_change_db: None,
            maximum_early_increase_db: None,
            minimum_positions: None,
        });
        let mut result = crate::test_fixtures::single_channel_room_result("L");
        result.channel_results.get_mut("L").unwrap().fir_coeffs = Some(
            std::iter::once(1.0)
                .chain(std::iter::repeat_n(0.0, 127))
                .collect(),
        );
        result.channels.get_mut("L").unwrap().plugins.push(
            roomeq_engine::output::create_convolution_plugin("L-fir.wav"),
        );
        (dir, config, result, MemoryArtifactStore::new())
    }

    #[test]
    fn saved_ir_candidate_is_bound_to_export_bytes_and_held_out() {
        let (dir, config, mut result, store) = fixture(0.45);
        validate_evidence(&config, 48_000.0).unwrap();
        apply(&mut result, &config, 48_000.0, dir.path(), &store).unwrap();
        assert_eq!(
            result.metadata.stage_outcomes.last().unwrap().status,
            roomeq_model::StageStatus::Applied
        );
        assert!((result.channel_results["L"].fir_coeffs.as_ref().unwrap()[48] + 0.4).abs() < 1e-6);
        let bytes = store.get(&dir.path().join("L-fir.wav")).unwrap();
        let decoded = crate::ctc::read_wav_bytes_channels_f64(
            &bytes,
            &dir.path().join("L-fir.wav"),
            48_000,
            "selected FIR",
        )
        .unwrap();
        assert_eq!(decoded.len(), 1);
        let retained = result.channel_results["L"].fir_coeffs.as_ref().unwrap();
        assert_eq!(decoded[0].len(), retained.len());
        assert!(
            decoded[0]
                .iter()
                .zip(retained)
                .all(|(saved, tap)| *saved == (*tap as f32) as f64)
        );
        let mut changed = config.clone();
        changed.provenance.temporal_fir.as_mut().unwrap().evidence[1].position_id = "wrong".into();
        assert!(validate_evidence(&changed, 48_000.0).is_err());
    }

    #[test]
    fn held_out_regression_keeps_baseline_bytes() {
        let (dir, config, mut result, store) = fixture(-0.5);
        apply(&mut result, &config, 48_000.0, dir.path(), &store).unwrap();
        assert_eq!(
            result.metadata.stage_outcomes.last().unwrap().status,
            roomeq_model::StageStatus::Skipped
        );
        assert_eq!(
            result.channel_results["L"].fir_coeffs.as_ref().unwrap()[48],
            0.0
        );
        assert!(store.get(&dir.path().join("L-fir.wav")).is_none());
    }

    #[test]
    fn public_synthetic_workflow_reports_temporal_decision_and_binds_export() {
        let (dir, config, _, _) = fixture(0.45);
        let result =
            super::super::optimize_room(&config, 48_000.0, None, Some(dir.path())).unwrap();
        let outcome = result
            .metadata
            .stage_outcomes
            .iter()
            .find(|outcome| outcome.stage == "temporal_fir")
            .unwrap();
        assert_eq!(outcome.status, roomeq_model::StageStatus::Skipped);
        assert!(
            outcome
                .advisories
                .iter()
                .any(|item| item == "no_training_candidate")
        );
        let inventory = result.metadata.final_convolution_sha256.as_ref().unwrap();
        assert_eq!(inventory.len(), 1);
        let (filename, digest) = inventory.iter().next().unwrap();
        assert_eq!(
            digest.as_deref(),
            Some(sha256(&fs::read(dir.path().join(filename)).unwrap()).as_str())
        );
        verify_final(&result, &config).unwrap();
        let baseline_dir = tempfile::tempdir().unwrap();
        let mut baseline_config = config.clone();
        baseline_config.provenance.temporal_fir = None;
        let baseline = super::super::optimize_room(
            &baseline_config,
            48_000.0,
            None,
            Some(baseline_dir.path()),
        )
        .unwrap();
        assert_eq!(
            result.channel_results["L"].fir_coeffs,
            baseline.channel_results["L"].fir_coeffs
        );
    }

    #[test]
    fn invalid_evidence_and_post_selection_drift_fail_closed() {
        let (dir, config, mut result, store) = fixture(0.45);
        let mut silent_baseline = result.clone();
        silent_baseline
            .channel_results
            .get_mut("L")
            .unwrap()
            .fir_coeffs = Some(vec![0.0; 128]);
        assert!(apply(&mut silent_baseline, &config, 48_000.0, dir.path(), &store).is_err());
        let strategy = config.provenance.temporal_fir.as_ref().unwrap();
        let mut baseline = vec![0.0; 513];
        baseline[0] = 1.0;
        let mut aliased = vec![0.0; 513];
        aliased[0] = 0.5;
        aliased[512] = 0.5;
        assert!(!spectral_safe(
            &aliased, &baseline, 48_000.0, strategy, None
        ));
        let evidence = prepared(&config, 48_000.0, 128).unwrap();
        let mut boosted = vec![0.0; 128];
        boosted[0] = 2.0;
        boosted[48] = -0.4;
        let baseline_128 = std::iter::once(1.0)
            .chain(std::iter::repeat_n(0.0, 127))
            .collect::<Vec<_>>();
        let metrics = partition_metrics(
            &evidence,
            &boosted,
            &baseline_128,
            TemporalPartition::Training,
        )
        .unwrap();
        assert!(metrics.0 > strategy.minimum_late_improvement_db);
        assert!(metrics.1 < 0.0);
        let mut permissive = strategy.clone();
        permissive.maximum_early_change_db = 10.0;
        assert!(
            acceptable(
                &evidence,
                &boosted,
                &baseline_128,
                &permissive,
                TemporalPartition::Training
            )
            .is_none()
        );
        let mut invalid = config.clone();
        invalid.provenance.temporal_fir.as_mut().unwrap().evidence[1].sidecar_path =
            dir.path().join("missing.json");
        assert!(validate_evidence(&invalid, 48_000.0).is_err());
        let mut reused = config.clone();
        reused.provenance.temporal_fir.as_mut().unwrap().evidence[1].sidecar_path =
            reused.provenance.temporal_fir.as_ref().unwrap().evidence[0]
                .sidecar_path
                .clone();
        assert!(validate_evidence(&reused, 48_000.0).is_err());
        let mut bad_delay = config.clone();
        bad_delay
            .provenance
            .temporal_fir
            .as_mut()
            .unwrap()
            .delays_seconds = vec![1.0];
        assert!(
            apply(
                &mut result.clone(),
                &bad_delay,
                48_000.0,
                dir.path(),
                &store
            )
            .is_err()
        );
        result
            .channels
            .get_mut("L")
            .unwrap()
            .plugins
            .push(roomeq_engine::output::create_delay_plugin(1.0));
        assert!(apply(&mut result.clone(), &config, 48_000.0, dir.path(), &store).is_err());
        result.channels.get_mut("L").unwrap().plugins.pop();
        apply(&mut result, &config, 48_000.0, dir.path(), &store).unwrap();
        result
            .channel_results
            .get_mut("L")
            .unwrap()
            .fir_coeffs
            .as_mut()
            .unwrap()[48] += 0.01;
        assert!(verify_final(&result, &config).is_err());
        result
            .channel_results
            .get_mut("L")
            .unwrap()
            .fir_coeffs
            .as_mut()
            .unwrap()[48] -= 0.01;
        result
            .channels
            .get_mut("L")
            .unwrap()
            .plugins
            .push(roomeq_engine::output::create_gain_plugin(1.0));
        assert!(verify_final(&result, &config).is_err());
    }

    #[test]
    fn configured_position_and_signed_early_limits_are_enforced() {
        let (_dir, mut config, _result, _store) = fixture(0.45);
        config
            .provenance
            .temporal_fir
            .as_mut()
            .unwrap()
            .minimum_positions = Some(3);
        assert!(validate_evidence(&config, 48_000.0).is_err());
        config
            .provenance
            .temporal_fir
            .as_mut()
            .unwrap()
            .minimum_positions = None;
        let evidence = prepared(&config, 48_000.0, 128).unwrap();
        let baseline = std::iter::once(1.0)
            .chain(std::iter::repeat_n(0.0, 127))
            .collect::<Vec<_>>();
        let strategy = config.provenance.temporal_fir.as_mut().unwrap();
        strategy.minimum_late_improvement_db = 0.0;
        strategy.maximum_early_increase_db = Some(-0.1);
        assert!(
            acceptable(
                &evidence,
                &baseline,
                &baseline,
                strategy,
                TemporalPartition::Training
            )
            .is_none()
        );
        strategy.maximum_early_increase_db = Some(0.25);
        assert!(
            acceptable(
                &evidence,
                &baseline,
                &baseline,
                strategy,
                TemporalPartition::Training
            )
            .is_some()
        );
    }

    #[cfg(feature = "measurement-zarr")]
    #[test]
    fn orcmeasurement_array_matches_raw_wav_and_rejects_tampering() {
        let (dir, config, _, _) = fixture(0.45);
        let sidecar = &config.provenance.temporal_fir.as_ref().unwrap().evidence[0].sidecar_path;
        let mut record = autoeq_measurements::read_sidecar_file(sidecar).unwrap();
        let mut samples = vec![0.0_f32; 256];
        samples[64] = 1.0;
        samples[112] = 0.5;
        let package = dir.path().join("synthetic.orcmeasurement");
        let (array_path, campaign_id, source_digest, logical_sha256) =
            zarr_test_fixture::write_synthetic_orcmeasurement(
                &package, "training", "L", &samples, 48_000,
            )
            .unwrap();
        let array = roomeq_model::TemporalArrayReference {
            package_path: package.clone(),
            array_path,
            logical_sha256,
            source_digest,
            campaign_id,
        };
        record
            .provenance
            .extensions
            .get_mut("temporal_ir_v1")
            .unwrap()["orcmeasurement"] = serde_json::to_value(array).unwrap();
        fs::write(sidecar, serde_json::to_vec(&record).unwrap()).unwrap();
        validate_evidence(&config, 48_000.0).unwrap();

        let mut tampered = record.clone();
        tampered
            .provenance
            .extensions
            .get_mut("temporal_ir_v1")
            .unwrap()["orcmeasurement"]["source_digest"] = serde_json::json!("wrong");
        fs::write(sidecar, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert!(validate_evidence(&config, 48_000.0).is_err());
        fs::write(sidecar, serde_json::to_vec(&record).unwrap()).unwrap();

        let chunk = package.join("arrays/positions/training/L/impulse_response/c/0");
        let original = fs::read(&chunk).unwrap();
        let mut changed = original.clone();
        changed[0] ^= 1;
        fs::write(&chunk, changed).unwrap();
        assert!(validate_evidence(&config, 48_000.0).is_err());
        fs::write(&chunk, original).unwrap();

        let manifest = package.join("manifest.json");
        let original = fs::read(&manifest).unwrap();
        let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
        changed["schemaVersion"] = serde_json::json!(2);
        fs::write(&manifest, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(validate_evidence(&config, 48_000.0).is_err());
        fs::write(&manifest, original).unwrap();

        let metadata = package.join("arrays/positions/training/L/impulse_response/zarr.json");
        let original = fs::read(&metadata).unwrap();
        let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
        changed["dimension_names"] = serde_json::json!("sample");
        fs::write(&metadata, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(validate_evidence(&config, 48_000.0).is_err());
    }
}
