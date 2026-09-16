//! Generate a public saved-IR temporal example and run the canonical workflow.
//! cargo run -p roomeq-workflow --example temporal_fir_synthetic -- /tmp/temporal-fir-example
use autoeq_measurements::{MeasurementOrigin, MeasurementRecord, write_sidecar};
use ndarray::Array1;
use roomeq_model::{
    Curve, FirConfig, InlineMeasurement, MeasurementRef, MeasurementSingle, MeasurementSource,
    ProcessingMode, RoomConfig, SpeakerConfig, TemporalFirConfig, TemporalIrEvidence,
    TemporalIrSidecar, TemporalPartition,
};
use roomeq_workflow::optimize_room;
use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};

fn curve(tail: f64) -> Curve {
    let freq = Array1::logspace(10.0, f64::log10(20.0), f64::log10(20_000.0), 128);
    let spl = freq.mapv(|frequency| {
        let phase = 2.0 * std::f64::consts::PI * frequency * 0.001;
        let magnitude = (1.0 + tail * phase.cos()).hypot(tail * phase.sin());
        80.0 + 20.0 * magnitude.log10()
    });
    Curve {
        freq,
        spl,
        phase: None,
        ..Default::default()
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn evidence(
    dir: &Path,
    label: &str,
    partition: TemporalPartition,
    tail: f32,
) -> Result<TemporalIrEvidence, Box<dyn std::error::Error>> {
    let path = dir.join(format!("{label}.wav"));
    let mut samples = vec![0.0_f32; 256];
    samples[64] = 1.0;
    samples[112] = tail;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    let mut record = MeasurementRecord::from_source_path(
        curve(f64::from(tail)),
        MeasurementOrigin::Recording,
        &path,
    )?;
    record.id = format!("public-synthetic:{label}");
    record.curve = serde_json::from_slice(&serde_json::to_vec(&record.curve)?)?;
    record.provenance.content_hash = record.curve.content_hash()?;
    record.provenance.extensions.insert(
        "temporal_ir_v1".into(),
        serde_json::to_value(TemporalIrSidecar {
            wav_path: path.clone(),
            sha256: sha256(&fs::read(&path)?),
            anchor_sample: 48,
            processing_state: "isolated_raw".into(),
            stimulus_reference: "public-synthetic-sweep".into(),
            capture_rate_hz: 48_000,
            channel: "L".into(),
            position_id: label.into(),
            partition,
            orcmeasurement: None,
        })?,
    );
    Ok(TemporalIrEvidence {
        sidecar_path: write_sidecar(&path, &record)?,
        position_id: label.into(),
        partition,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = env::args()
        .nth(1)
        .ok_or("output directory argument required")?;
    let dir = Path::new(&directory);
    fs::create_dir_all(dir)?;
    let dir = dir.canonicalize()?;
    let mut config = RoomConfig::default();
    let raw_curve = curve(0.5);
    config.speakers.insert(
        "L".into(),
        SpeakerConfig::Single(MeasurementSource::Single(MeasurementSingle {
            measurement: MeasurementRef::Inline(InlineMeasurement {
                frequencies: raw_curve.freq.to_vec(),
                magnitude_db: raw_curve.spl.to_vec(),
                phase_deg: None,
                name: Some("public synthetic speaker".into()),
                wav_path: None,
                csv_path: None,
            }),
            speaker_name: None,
        })),
    );
    config.optimizer.processing_mode = ProcessingMode::PhaseLinear;
    config.optimizer.fir = Some(FirConfig {
        taps: 128,
        ..FirConfig::default()
    });
    config.provenance.temporal_fir = Some(TemporalFirConfig {
        channel: "L".into(),
        evidence: vec![
            evidence(&dir, "training", TemporalPartition::Training, 0.5)?,
            evidence(&dir, "held", TemporalPartition::HeldOut, 0.45)?,
        ],
        frequencies_hz: vec![80.0],
        frequency_weights: vec![1.0],
        window_seconds: 0.0007,
        starts_seconds: vec![0.0, 0.001],
        delays_seconds: vec![0.001],
        strengths: vec![0.4],
        minimum_late_improvement_db: 0.1,
        maximum_early_change_db: 1.0,
        maximum_spectral_change_db: 12.0,
    });
    fs::write(dir.join("input.json"), serde_json::to_vec_pretty(&config)?)?;
    let result = optimize_room(&config, 48_000.0, None, Some(&dir))?;
    fs::write(
        dir.join("result.json"),
        serde_json::to_vec_pretty(&result.to_dsp_chain_output())?,
    )?;
    println!("{}", dir.display());
    Ok(())
}
