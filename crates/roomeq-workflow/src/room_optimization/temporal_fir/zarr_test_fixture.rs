//! Synthetic Swift schema-v1 package for the grouped temporal workflow test.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{error::Error, fs, path::Path};

type FixtureResult<T> = Result<T, Box<dyn Error>>;

/// Returns (array path, campaign ID, campaign source digest, logical digest).
/// Samples are test data; no private capture is copied into this package.
pub(super) fn write_synthetic_orcmeasurement(
    package: &Path,
    position: &str,
    channel: &str,
    samples: &[f32],
    sample_rate_hz: u32,
) -> FixtureResult<(String, String, String, String)> {
    assert!(samples.len() >= 2 && samples.iter().all(|sample| sample.is_finite()));
    assert!(sample_rate_hz > 0);
    assert!(safe_component(position) && safe_component(channel));
    let array_path = format!("positions/{position}/{channel}/impulse_response");
    let campaign_id = "synthetic-temporal".to_owned();
    let source_digest = "synthetic-source-digest".to_owned();
    let descriptor = json!({
        "path":array_path.clone(),"role":"impulse_response","shape":[samples.len()],
        "units":"linear_amplitude","processingState":"isolated_physical_output",
        "fillValue":0,"chunkLength":2,"compression":"none"
    });
    let channel_record = json!({
        "positionID":position,"channelID":channel,"sampleRate":sample_rate_hz,
        "acquisition":{"transport":"synthetic","microphoneChain":"synthetic",
            "timeReference":"synthetic-test-reference","processingState":"isolated_physical_output"},
        "quality":{"valid":true,"acceptedRepeatIndices":[0,1,2],"calculationHash":"synthetic-calculation"},
        "timingRepeats":[]
    });
    let manifest = json!({
        "schemaVersion":1,"campaignID":campaign_id.clone(),"sourceDigest":source_digest.clone(),
        "arrays":[descriptor],"channels":[channel_record],"sourceArtifacts":[],
        "rawWaveformAuthority":true,"calibrationStatus":"synthetic",
        "correctionOwner":"synthetic"
    });
    let logical_digest = swift_v1_digest(&manifest, samples)?;
    fs::create_dir(package)?;
    write_json(
        &package.join("zarr.json"),
        &json!({
            "zarr_format":3,"node_type":"group",
            "attributes":{"orcmeasurement_schema_version":1}
        }),
    )?;
    write_json(&package.join("manifest.json"), &manifest)?;
    fs::write(
        package.join("logical.sha256"),
        format!("{logical_digest}\n"),
    )?;
    for parent in [
        "arrays",
        "arrays/positions",
        &format!("arrays/positions/{position}"),
        &format!("arrays/positions/{position}/{channel}"),
    ] {
        let group = package.join(parent);
        fs::create_dir(&group)?;
        write_json(
            &group.join("zarr.json"),
            &json!({
                "zarr_format":3,"node_type":"group","attributes":{}
            }),
        )?;
    }
    let array = package.join("arrays").join(&array_path);
    fs::create_dir_all(array.join("c"))?;
    write_json(
        &array.join("zarr.json"),
        &json!({
            "zarr_format":3,"node_type":"array","shape":[samples.len()],
            "data_type":"float32","chunk_grid":{"name":"regular","configuration":{"chunk_shape":[2]}},
            "chunk_key_encoding":{"name":"default","configuration":{"separator":"/"}},
            "fill_value":0,"codecs":[{"name":"bytes","configuration":{"endian":"little"}}],
            "storage_transformers":[],"dimension_names":["sample"],
            "attributes":{"units":"linear_amplitude","processing_state":"isolated_physical_output",
                "role":"impulse_response","logical_path":array_path.clone(),"element_type":"float32"}
        }),
    )?;
    for (index, pair) in samples.chunks(2).enumerate() {
        let mut bytes = Vec::with_capacity(8);
        for sample in pair {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        if pair.len() == 1 {
            bytes.extend_from_slice(&0f32.to_le_bytes());
        }
        fs::write(array.join("c").join(index.to_string()), bytes)?;
    }
    Ok((array_path, campaign_id, source_digest, logical_digest))
}

fn swift_v1_digest(manifest: &Value, samples: &[f32]) -> FixtureResult<String> {
    let descriptor = &manifest["arrays"][0];
    let mut sha = Sha256::new();
    sha.update(b"orcmeasurement-logical-v1\0");
    for key in [
        "campaignID",
        "sourceDigest",
        "calibrationStatus",
        "correctionOwner",
    ] {
        sha.update(
            manifest[key]
                .as_str()
                .ok_or("invalid fixture manifest")?
                .as_bytes(),
        );
    }
    sha.update(serde_json::to_vec(&manifest["channels"])?);
    sha.update(format!(
        "\0{}\0{}\0{}\0{}\0{}\0",
        descriptor["path"].as_str().ok_or("invalid fixture path")?,
        descriptor["role"].as_str().ok_or("invalid fixture role")?,
        descriptor["units"]
            .as_str()
            .ok_or("invalid fixture units")?,
        descriptor["processingState"]
            .as_str()
            .ok_or("invalid fixture processing state")?,
        samples.len()
    ));
    for sample in samples {
        sha.update(sample.to_le_bytes());
    }
    Ok(sha
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

fn write_json(path: &Path, value: &Value) -> FixtureResult<()> {
    fs::write(path, serde_json::to_vec(value)?)?;
    Ok(())
}
