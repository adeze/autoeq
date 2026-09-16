//! Read-only adapter for Swift schema-v1 `.orcmeasurement` Zarr v3 packages.
//! Supports local rank-one Float32 arrays, regular chunks, little-endian bytes, and optional gzip.
//! The derived impulse is usable only after the caller independently qualifies its raw source.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, error::Error, fs, path::Path, sync::Arc};
use zarrs::{array::Array, filesystem::FilesystemStore};

type ReadResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug)]
pub struct MeasurementImpulse {
    pub samples: Vec<f32>,
    pub sample_rate_hz: u32,
    pub campaign_id: String,
    pub source_digest: String,
    pub logical_digest: String,
    pub processing_state: String,
    pub time_reference: String,
}

/// Open a package only when both the source and Swift logical identities match expected values.
/// `array_path` is the manifest path, e.g. `positions/MLP/FL/impulse_response`.
/// This validates every array because `logical.sha256` binds the whole package, not one channel.
pub fn read_orcmeasurement_impulse(
    package: &Path,
    array_path: &str,
    expected_source_digest: &str,
    expected_logical_digest: &str,
) -> ReadResult<MeasurementImpulse> {
    if package.extension().and_then(|s| s.to_str()) != Some("orcmeasurement") {
        return Err("expected .orcmeasurement directory".into());
    }
    let root = json_file(&package.join("zarr.json"))?;
    ensure(
        root["zarr_format"] == 3 && root["node_type"] == "group",
        "invalid Zarr root",
    )?;
    let manifest = json_file(&package.join("manifest.json"))?;
    let version = number(&manifest["schemaVersion"])?;
    ensure(version == 1, "only measurement schema v1 is supported")?;
    let source_digest = string(&manifest["sourceDigest"])?;
    ensure(
        source_digest == expected_source_digest,
        "source digest mismatch",
    )?;
    ensure(
        manifest["rawWaveformAuthority"] == true,
        "raw waveform authority absent",
    )?;
    let campaign_id = string(&manifest["campaignID"])?;
    let paths: Vec<&str> = array_path.split('/').collect();
    ensure(
        paths.len() == 4 && paths[0] == "positions" && paths[3] == "impulse_response",
        "invalid impulse path",
    )?;
    let channels = manifest["channels"].as_array().ok_or("missing channels")?;
    let channel = channels
        .iter()
        .find(|c| c["positionID"] == paths[1] && c["channelID"] == paths[2])
        .ok_or("channel not declared")?;
    ensure(
        channel["quality"]["valid"] == true,
        "channel quality invalid",
    )?;
    let processing_state = string(&channel["acquisition"]["processingState"])?;
    ensure(
        processing_state == "isolated_physical_output",
        "non-isolated processing state",
    )?;
    let sample_rate_hz =
        u32::try_from(number(&channel["sampleRate"])?).map_err(|_| "invalid sample rate")?;
    ensure(sample_rate_hz > 0, "invalid sample rate")?;
    let time_reference = string(&channel["acquisition"]["timeReference"])?;

    let declared = manifest["arrays"].as_array().ok_or("missing arrays")?;
    let mut seen = HashSet::new();
    let store = Arc::new(FilesystemStore::new(package)?);
    let mut values = Vec::with_capacity(declared.len());
    let mut selected = None;
    let mut total_elements = 0usize;
    for descriptor in declared {
        let path = string(&descriptor["path"])?;
        ensure(seen.insert(path.to_owned()), "duplicate array path")?;
        ensure(valid_path(path), "invalid array path")?;
        let data = read_array(&store, package, descriptor)?;
        total_elements = total_elements
            .checked_add(data.len())
            .ok_or("package too large")?;
        ensure(
            total_elements <= 32_000_000,
            "package exceeds local read limit",
        )?;
        if path == array_path {
            ensure(
                descriptor["role"] == "impulse_response",
                "array role mismatch",
            )?;
            ensure(
                descriptor["units"] == "linear_amplitude",
                "impulse unit mismatch",
            )?;
            ensure(
                descriptor["processingState"] == processing_state,
                "impulse processing mismatch",
            )?;
            ensure(
                descriptor["dimensions"].is_null()
                    || descriptor["dimensions"]
                        .as_array()
                        .is_some_and(|d| d.len() == 1 && d[0] == "sample"),
                "impulse axis mismatch",
            )?;
            selected = Some(data.clone());
        }
        values.push((descriptor, data));
    }
    let samples = selected.ok_or("impulse not declared")?;
    let digest = logical_digest(&manifest, &values)?;
    let recorded = fs::read_to_string(package.join("logical.sha256"))?;
    ensure(recorded.trim() == digest, "package logical digest mismatch")?;
    ensure(
        expected_logical_digest == digest,
        "expected logical digest mismatch",
    )?;
    Ok(MeasurementImpulse {
        samples,
        sample_rate_hz,
        campaign_id: campaign_id.to_owned(),
        source_digest: source_digest.to_owned(),
        logical_digest: digest,
        processing_state: processing_state.to_owned(),
        time_reference: time_reference.to_owned(),
    })
}

fn read_array(
    store: &Arc<FilesystemStore>,
    package: &Path,
    descriptor: &Value,
) -> ReadResult<Vec<f32>> {
    let path = string(&descriptor["path"])?;
    let meta = json_file(&package.join("arrays").join(path).join("zarr.json"))?;
    ensure(
        meta["zarr_format"] == 3 && meta["node_type"] == "array" && meta["data_type"] == "float32",
        "unsupported Zarr array",
    )?;
    ensure(
        meta["chunk_grid"]["name"] == "regular"
            && meta["chunk_key_encoding"]["name"] == "default"
            && meta["chunk_key_encoding"]["configuration"]["separator"] == "/",
        "unsupported Zarr layout",
    )?;
    ensure(
        meta["storage_transformers"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "storage transformers unsupported",
    )?;
    let shape = meta["shape"].as_array().ok_or("missing shape")?;
    let declared_shape = descriptor["shape"]
        .as_array()
        .ok_or("missing declared shape")?;
    ensure(
        shape.len() == 1 && shape == declared_shape,
        "only matching rank-one arrays supported",
    )?;
    let count = usize::try_from(number(&shape[0])?).map_err(|_| "array too large")?;
    ensure(count <= 32_000_000, "array exceeds local read limit")?;
    let chunk = meta["chunk_grid"]["configuration"]["chunk_shape"]
        .as_array()
        .ok_or("missing chunk shape")?;
    ensure(
        chunk.len() == 1
            && number(&chunk[0])? > 0
            && number(&chunk[0])? == number(&descriptor["chunkLength"])?,
        "chunk length mismatch",
    )?;
    let codecs = meta["codecs"].as_array().ok_or("missing codecs")?;
    ensure(
        (codecs.len() == 1 || codecs.len() == 2)
            && codecs[0]["name"] == "bytes"
            && codecs[0]["configuration"]["endian"] == "little",
        "unsupported codec pipeline",
    )?;
    let gzip = codecs.len() == 2;
    ensure(
        !gzip
            || (codecs[1]["name"] == "gzip" && number(&codecs[1]["configuration"]["level"])? <= 9),
        "unsupported compression",
    )?;
    ensure(
        (!gzip && descriptor["compression"] == "none")
            || (gzip
                && descriptor["compression"]["gzip"]["level"]
                    == codecs[1]["configuration"]["level"]),
        "compression mismatch",
    )?;
    let fill = meta["fill_value"].as_f64().ok_or("invalid fill value")? as f32;
    ensure(
        fill.is_finite()
            && descriptor["fillValue"]
                .as_f64()
                .is_some_and(|v| (v as f32).to_bits() == fill.to_bits()),
        "fill mismatch",
    )?;
    let attrs = &meta["attributes"];
    for (key, manifest_key) in [
        ("role", "role"),
        ("units", "units"),
        ("processing_state", "processingState"),
        ("logical_path", "path"),
    ] {
        ensure(
            attrs[key] == descriptor[manifest_key],
            "array metadata mismatch",
        )?;
    }
    ensure(
        meta["dimension_names"]
            == serde_json::json!([if descriptor["role"] == "impulse_response" {
                "sample"
            } else {
                "frequency"
            }]),
        "dimension metadata mismatch",
    )?;
    let array = Array::open(store.clone(), &format!("/arrays/{path}"))?;
    let data: Vec<f32> = array.retrieve_array_subset(&array.subset_all())?;
    ensure(
        data.len() == count && data.iter().all(|x| x.is_finite()),
        "invalid decoded samples",
    )?;
    Ok(data)
}

fn logical_digest(manifest: &Value, arrays: &[(&Value, Vec<f32>)]) -> ReadResult<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"orcmeasurement-logical-v1\0");
    hasher.update(string(&manifest["campaignID"])?.as_bytes());
    hasher.update(string(&manifest["sourceDigest"])?.as_bytes());
    for key in ["calibrationStatus", "correctionOwner"] {
        hasher.update(string(&manifest[key])?.as_bytes());
    }
    hasher.update(serde_json::to_vec(&manifest["channels"])?);
    let mut artifacts = manifest["sourceArtifacts"]
        .as_array()
        .ok_or("missing source artifacts")?
        .iter()
        .collect::<Vec<_>>();
    artifacts.sort_by_key(|a| a["path"].as_str().unwrap_or(""));
    for artifact in artifacts {
        hasher.update(format!("\0{}", string(&artifact["role"])?));
        hasher.update(format!("\0{}", string(&artifact["sha256"])?));
    }
    let mut arrays = arrays.iter().collect::<Vec<_>>();
    arrays.sort_by_key(|(d, _)| d["path"].as_str().unwrap_or(""));
    for (descriptor, data) in arrays {
        let path = string(&descriptor["path"])?;
        let role = string(&descriptor["role"])?;
        let units = string(&descriptor["units"])?;
        let state = string(&descriptor["processingState"])?;
        hasher.update(format!(
            "\0{path}\0{role}\0{units}\0{state}\0{}\0",
            number(&descriptor["shape"][0])?
        ));
        for sample in data {
            hasher.update(sample.to_le_bytes());
        }
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn json_file(path: &Path) -> ReadResult<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn string(value: &Value) -> ReadResult<&str> {
    value.as_str().ok_or_else(|| "missing string".into())
}
fn number(value: &Value) -> ReadResult<u64> {
    value
        .as_u64()
        .ok_or_else(|| "missing positive integer".into())
}
fn ensure(ok: bool, message: &'static str) -> ReadResult<()> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn valid_path(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains("..")
        && path.split('/').all(|part| !part.is_empty() && part != ".")
}
