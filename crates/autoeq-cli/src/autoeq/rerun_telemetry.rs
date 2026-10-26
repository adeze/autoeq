//! Optional direct DE barrier telemetry. Observes existing DSP; never scores candidates.
use autoeq_core::display_smoothing;
#[path = "envelope_view.rs"]
mod envelope_view;
#[path = "response_envelope.rs"]
mod response_envelope;
use anyhow::{Result, anyhow, ensure};
use autoeq::de::{CheckpointFitness, DECheckpoint};
use autoeq::optim::{ObjectiveData, loss::ObjectiveContext};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) struct Telemetry {
    file: Option<rerun::RecordingStream>,
    viewer: Option<rerun::RecordingStream>,
    temp: Option<tempfile::NamedTempFile>,
    output: PathBuf,
    objective: ObjectiveData,
    run_identity: String,
    checkpoint_identity: Option<String>,
    callbacks: usize,
    first_generation: Option<usize>,
    last_generation: Option<usize>,
    terminal_observed: bool,
    plot_min_db: f64,
    plot_max_db: f64,
}

fn context(data: &ObjectiveData) -> ObjectiveContext<'_> {
    ObjectiveContext {
        freqs: &data.freqs,
        target: &data.target,
        deviation: &data.deviation,
        srate: data.srate,
        peq_model: data.peq_model,
        min_freq: data.min_freq,
        max_freq: data.max_freq,
        smooth: data.smooth,
        smooth_n: data.smooth_n,
        audibility_deadband: data.audibility_deadband.as_ref(),
        smoothness_penalty: data.smoothness_penalty.as_ref(),
    }
}

fn validate_data(data: &ObjectiveData) -> Result<()> {
    ensure!(
        data.peq_model == autoeq::PeqModel::Pk,
        "Rerun telemetry currently supports only the pk PEQ layout"
    );
    ensure!(
        data.multi_objective.is_none(),
        "Rerun telemetry supports only single-curve PEQ objectives"
    );
    let n = data.freqs.len();
    ensure!(
        n > 1
            && n <= response_envelope::MAX_FREQUENCIES
            && data.target.len() == n
            && data.deviation.len() == n,
        "invalid or oversized telemetry frequency/response arrays"
    );
    ensure!(
        data.freqs.iter().all(|v| v.is_finite() && *v > 0.0)
            && data
                .freqs
                .iter()
                .zip(data.freqs.iter().skip(1))
                .all(|(a, b)| a < b)
            && data
                .target
                .iter()
                .chain(data.deviation.iter())
                .all(|v| v.is_finite()),
        "telemetry requires finite responses on a strictly increasing positive-Hz grid"
    );
    Ok(())
}

fn tensor(values: &[f64]) -> rerun::Tensor {
    rerun::Tensor::new(rerun::encodings::TensorData::new(
        vec![values.len() as u64],
        rerun::encodings::TensorBuffer::F64(values.to_vec().into()),
    ))
}

pub(super) fn validate_population(count: usize) -> Result<()> {
    ensure!(
        (1..=response_envelope::MAX_POPULATION).contains(&count),
        "Rerun response envelopes require at most 256 population members"
    );
    Ok(())
}

impl Telemetry {
    pub(super) fn new(
        output: &Path,
        port: Option<u16>,
        objective: ObjectiveData,
        run_identity: &str,
        measurement_identity: &str,
    ) -> Result<Self> {
        validate_data(&objective)?;
        let baseline = (&*objective.target - &*objective.deviation).to_vec();
        ensure!(
            baseline.iter().all(|v| v.is_finite()),
            "invalid prepared measurement values"
        );
        let minimum = baseline
            .iter()
            .chain(objective.target.iter())
            .copied()
            .fold(0.0, f64::min);
        let maximum = baseline
            .iter()
            .chain(objective.target.iter())
            .copied()
            .fold(0.0, f64::max);
        let plot_min_db = (minimum / 10.0).floor() * 10.0 - 20.0;
        let plot_max_db = (maximum / 10.0).ceil() * 10.0 + 20.0;
        ensure!(
            (plot_max_db - plot_min_db).is_finite(),
            "response range too large for plot"
        );
        ensure!(
            !output.try_exists()?,
            "Rerun recording exists; refusing overwrite: {}",
            output.display()
        );
        ensure!(
            !run_identity.is_empty(),
            "Rerun telemetry requires run identity"
        );
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        let store_id =
            rerun::StoreId::random(rerun::StoreKind::Recording, "adeze_autoeq_direct_de");
        let builder = || rerun::RecordingStreamBuilder::from_store_id(&store_id).enabled(true);
        let file = builder().save(temp.path())?;
        let viewer = port
            .map(|port| {
                ensure!(port != 0, "Rerun viewer port must be nonzero");
                builder()
                    .connect_grpc_opts(format!("rerun+http://127.0.0.1:{port}/proxy"))
                    .map_err(Into::into)
            })
            .transpose()?;
        let telemetry = Self {
            file: Some(file),
            viewer,
            temp: Some(temp),
            output: output.to_path_buf(),
            objective,
            run_identity: run_identity.to_owned(),
            checkpoint_identity: None,
            callbacks: 0,
            first_generation: None,
            last_generation: None,
            terminal_observed: false,
            plot_min_db,
            plot_max_db,
        };
        for rec in telemetry.recordings() {
            rec.set_log_time_enabled(false);
            rec.set_log_tick_enabled(false);
            rec.log_static("provenance",&rerun::TextDocument::new(json!({
                "mode":"direct existing exact-DE generation-barrier callback; no polling",
                "run_identity":run_identity,"measurement_identity":measurement_identity,
                "recording_id":store_id.recording_id().to_string(),
                "sample_rate_hz":telemetry.objective.srate,
                "response":"existing ObjectiveContext.peq_spl; prepared measurement + PEQ magnitude (predicted, not remeasured)",
                "frequency_grid":"full canonical objective grid; no extra smoothing/interpolation",
                "plot_coordinates":"display x=12*normalized log10(Hz), y=6*normalized inverted dB; tick labels show physical units; higher level displays upward",
                "fixed_plot_db_domain":[plot_min_db,plot_max_db],
                "plot_out_of_domain":"not clipped; float64 arrays remain authoritative",
                "level_reference":"prepared/normalized measurement and target; absolute SPL not asserted",
                "storage":"float64 canonical arrays in tensors; float32 XY display only",
                "callback_index":"every callback, including same-generation terminal finalization",
                "objective_units":"loss-specific, not assumed dB",
                "transport":"local file primary; optional SDK gRPC queue; backpressure may slow runtime"
            }).to_string()))?;
            rec.log_static(
                "arrays/frequency_hz",
                &tensor(
                    telemetry
                        .objective
                        .freqs
                        .as_slice()
                        .ok_or_else(|| anyhow!("noncontiguous frequency grid"))?,
                ),
            )?;
        }
        telemetry.axes()?;
        telemetry.curve("measurement", &baseline, [120, 130, 145], true)?;
        telemetry.curve(
            "target",
            &telemetry.objective.target.to_vec(),
            [220, 150, 35],
            true,
        )?;
        Ok(telemetry)
    }

    fn recordings(&self) -> impl Iterator<Item = &rerun::RecordingStream> {
        self.file.iter().chain(self.viewer.iter())
    }

    fn plot_point(&self, hz: f64, db: f64) -> [f32; 2] {
        let lo = self.objective.freqs[0].log10();
        let hi = self.objective.freqs[self.objective.freqs.len() - 1].log10();
        [
            (12.0 * (hz.log10() - lo) / (hi - lo)) as f32,
            (6.0 * (self.plot_max_db - db) / (self.plot_max_db - self.plot_min_db)) as f32,
        ]
    }

    fn axes(&self) -> Result<()> {
        let mut lines = Vec::new();
        let mut positions = Vec::new();
        let mut labels = Vec::new();
        let min_hz = self.objective.freqs[0];
        let max_hz = self.objective.freqs[self.objective.freqs.len() - 1];
        for hz in [
            20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0,
        ] {
            if hz < min_hz || hz > max_hz {
                continue;
            }
            let x = self.plot_point(hz, 0.0)[0];
            lines.push(vec![[x, 0.0], [x, 6.0]]);
            positions.push([x, 6.3]);
            labels.push(if hz >= 1000.0 {
                format!("{:.0}k", hz / 1000.0)
            } else {
                format!("{hz:.0}")
            });
        }
        let step = (((self.plot_max_db - self.plot_min_db) / 6.0) / 5.0)
            .ceil()
            .max(1.0)
            * 5.0;
        let mut db = (self.plot_min_db / step).ceil() * step;
        while db <= self.plot_max_db {
            let y = self.plot_point(min_hz, db)[1];
            lines.push(vec![[0.0, y], [12.0, y]]);
            positions.push([-0.4, y]);
            labels.push(format!("{db:.0} dB"));
            db += step;
        }
        let grid =
            rerun::LineStrips2D::new(lines).with_colors([rerun::Color::from_rgb(65, 75, 90)]);
        let ticks = rerun::Points2D::new(positions)
            .with_labels(labels)
            .with_show_labels(true)
            .with_radii([0.015]);
        let legend = rerun::Points2D::new([
            [1.0, -1.6],
            [7.0, -1.6],
            [1.0, -0.9],
            [7.0, -0.9],
            [1.0, -2.3],
        ])
        .with_labels([
            "Prepared measurement",
            "Target",
            "Predicted corrected",
            "Filter response",
            "Final report corrected",
        ])
        .with_colors(
            [
                [120, 130, 145],
                [220, 150, 35],
                [45, 110, 215],
                [170, 100, 205],
                [35, 145, 115],
            ]
            .map(|c| rerun::Color::from_rgb(c[0], c[1], c[2])),
        )
        .with_show_labels(true)
        .with_radii([0.04]);
        let axis_labels = rerun::Points2D::new([[6.0, 7.5], [-0.5, -0.2]])
            .with_labels(["Frequency (Hz, logarithmic)", "Level (dB), upward"])
            .with_show_labels(true)
            .with_radii([0.0]);
        for rec in self.recordings() {
            rec.log_static("frequency_response/axes", &grid)?;
            rec.log_static("frequency_response/ticks", &ticks)?;
            rec.log_static("frequency_response/legend", &legend)?;
            rec.log_static("frequency_response/axis_labels", &axis_labels)?;
        }
        Ok(())
    }

    fn curve(&self, name: &str, values: &[f64], color: [u8; 3], is_static: bool) -> Result<()> {
        ensure!(
            values.len() == self.objective.freqs.len() && values.iter().all(|v| v.is_finite()),
            "nonfinite or misaligned telemetry response"
        );
        let points: Vec<[f32; 2]> = self
            .objective
            .freqs
            .iter()
            .zip(values)
            .map(|(hz, db)| self.plot_point(*hz, *db))
            .collect();
        ensure!(
            points.iter().flatten().all(|v| v.is_finite()),
            "response exceeds float32 display range"
        );
        let lines = rerun::LineStrips2D::new([points])
            .with_colors([rerun::Color::from_rgb(color[0], color[1], color[2])]);
        let raw = tensor(values);
        for rec in self.recordings() {
            rec.log_with_static(format!("frequency_response/{name}"), is_static, &lines)?;
            rec.log_with_static(format!("arrays/{name}_db"), is_static, &raw)?;
        }
        Ok(())
    }

    pub(super) fn record(&mut self, c: &DECheckpoint) -> Result<()> {
        ensure!(
            c.run_identity == self.run_identity,
            "Rerun checkpoint run identity mismatch"
        );
        ensure!(
            self.last_generation
                .is_none_or(|last| c.generation == last || c.generation == last + 1),
            "direct telemetry observed a missing or regressing generation"
        );
        ensure!(
            !c.best_params.is_empty()
                && c.best_params.len().is_multiple_of(3)
                && c.best_params.iter().all(|v| v.is_finite()),
            "invalid telemetry parameter vector"
        );
        let identity = json!({"implementation":c.implementation_identity,"solver_source":c.solver_source_identity,
            "target":c.target_identity,"build":c.build_identity,"configuration":c.configuration_fingerprint,"seed":c.seed}).to_string();
        ensure!(
            self.checkpoint_identity
                .as_ref()
                .is_none_or(|v| v == &identity),
            "Rerun checkpoint implementation/config/seed changed"
        );
        if self.checkpoint_identity.is_none() {
            for rec in self.recordings() {
                rec.log_static(
                    "checkpoint_identity",
                    &rerun::TextDocument::new(identity.as_str()),
                )?;
            }
            self.checkpoint_identity = Some(identity);
        }
        let encoded = serde_json::to_vec(c)?;
        let mut checkpoint_hash = String::with_capacity(64);
        for byte in Sha256::digest(&encoded) {
            write!(&mut checkpoint_hash, "{byte:02x}")?;
        }
        let callback_index = self.callbacks + 1;
        let filter = context(&self.objective).peq_spl(&c.best_params);
        let corrected = &filter + &*self.objective.target - &*self.objective.deviation;
        ensure!(
            c.population.len() == c.population_fitness.len(),
            "Population fitness count differs"
        );
        let baseline = (&*self.objective.target - &*self.objective.deviation).to_vec();
        let responses = c
            .population
            .iter()
            .zip(&c.population_fitness)
            .filter(|(_, fitness)| matches!(fitness, CheckpointFitness::Finite(v) if v.is_finite()))
            .map(|(params, _)| {
                ensure!(
                    params.len() == c.best_params.len() && params.iter().all(|v| v.is_finite()),
                    "Invalid population parameters"
                );
                Ok(
                    (context(&self.objective).peq_spl(params) + &*self.objective.target
                        - &*self.objective.deviation)
                        .to_vec(),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let frequency = self.objective.freqs.to_vec();
        let envelope = response_envelope::Envelope::from_candidates(
            &frequency,
            &responses,
            display_smoothing::Mode::Raw,
        )
        .map_err(|e| anyhow!(e))?;
        for rec in self.recordings() {
            rec.set_time_sequence("callback_index", callback_index as i64);
            rec.set_time_sequence("generation", c.generation as i64);
            rec.log("checkpoint",&rerun::TextDocument::new(json!({
                "checkpoint_sha256":format!("sha256:{checkpoint_hash}"),
                "generation":c.generation,"evaluations":c.evaluations,"callback_index":callback_index,
                "best_params":c.best_params,"best_fitness":c.best_fitness,"terminal":c.terminal,
                "pre_polish_best":true
            }).to_string()))?;
            match c.best_fitness {
                CheckpointFitness::Finite(v) if v.is_finite() => {
                    rec.log("objective/best", &rerun::Scalars::new([v]))?
                }
                CheckpointFitness::PositiveInfinity => {
                    rec.log("objective/best", &rerun::Clear::new(false))?
                }
                _ => return Err(anyhow!("nonfinite checkpoint objective")),
            }
            envelope_view::Chart {
                min_hz: frequency[0],
                max_hz: frequency[frequency.len() - 1],
                min_db: self.plot_min_db,
                max_db: self.plot_max_db,
            }
            .log(
                rec,
                "frequency_response/envelope",
                &frequency,
                &baseline,
                &self.objective.target.to_vec(),
                &corrected.to_vec(),
                &envelope,
                display_smoothing::Mode::Raw,
                [45, 110, 215],
            )
            .map_err(|e| anyhow!(e))?;
            rec.log("population_scope",&rerun::TextDocument::new(json!({"candidates":envelope.count,"population":c.population.len(),"selection":"finite objective fitness; not an acoustic qualification or general constraint-feasibility proof","smoothing":display_smoothing::ALGORITHM_VERSION,"mode":"raw","bands":"pointwise candidate spread, not confidence"}).to_string()))?;
        }
        self.curve(
            "candidate_filter",
            filter
                .as_slice()
                .ok_or_else(|| anyhow!("noncontiguous filter response"))?,
            [170, 100, 205],
            false,
        )?;
        self.curve(
            "predicted_corrected",
            corrected
                .as_slice()
                .ok_or_else(|| anyhow!("noncontiguous corrected response"))?,
            [45, 110, 215],
            false,
        )?;
        if let Some(terminal) = &c.terminal
            && terminal.finalized
        {
            let params = terminal
                .final_params
                .as_ref()
                .ok_or_else(|| anyhow!("terminal missing final parameters"))?;
            ensure!(
                params.len() == c.best_params.len() && params.iter().all(|v| v.is_finite()),
                "invalid final report parameters"
            );
            let final_filter = context(&self.objective).peq_spl(params);
            let final_corrected =
                &final_filter + &*self.objective.target - &*self.objective.deviation;
            self.curve(
                "final_report_corrected",
                &final_corrected.to_vec(),
                [35, 145, 115],
                false,
            )?;
        }
        // File-only flush provides backpressure/error visibility without network
        // I/O in the callback. Viewer traffic remains in the SDK's bounded queue.
        self.file
            .as_ref()
            .ok_or_else(|| anyhow!("recording already finalized"))?
            .flush_with_timeout(Duration::from_secs(5))?;
        self.callbacks = callback_index;
        self.first_generation.get_or_insert(c.generation);
        self.last_generation = Some(c.generation);
        self.terminal_observed = c.terminal.as_ref().is_some_and(|t| t.finalized);
        Ok(())
    }

    pub(super) fn finish(&mut self, optimizer_returned_ok: bool) -> Result<()> {
        if self.temp.is_none() {
            return Ok(());
        }
        let outcome = json!({"optimizer_returned_ok":optimizer_returned_ok,"callbacks_logged":self.callbacks,
            "first_generation":self.first_generation,"last_generation":self.last_generation,
            "generation_barriers_expected":self.first_generation.zip(self.last_generation).map(|(a,b)|b-a+1),
            "terminal_finalized_observed":self.terminal_observed,
            "recording_complete":optimizer_returned_ok && self.terminal_observed,
            "completeness_scope":"complete callback prefix flushed to local file; CLI separately checks final outcome flush/publication",
            "recording":"direct callback observations; errors retain only successfully flushed prefix"});
        let mut failure = None;
        for rec in self.recordings() {
            if let Err(e) = rec.log(
                "run/outcome",
                &rerun::TextDocument::new(outcome.to_string()),
            ) {
                failure.get_or_insert(anyhow!(e));
            }
        }
        if let Some(viewer) = self.viewer.take() {
            if let Err(e) = viewer.flush_with_timeout(Duration::from_secs(2)) {
                eprintln!("Rerun viewer delivery incomplete; local recording remains primary: {e}");
                if let Some(file) = self.file.as_ref()
                    && let Err(log_error) = file.log(
                        "run/viewer_warning",
                        &rerun::TextDocument::new(format!("delivery unverified: {e}")),
                    )
                {
                    failure.get_or_insert(anyhow!(log_error));
                }
            }
            drop(viewer);
        }
        if let Some(file) = self.file.take() {
            if let Err(e) = file.flush_with_timeout(Duration::from_secs(5)) {
                failure.get_or_insert(anyhow!(e));
            }
            drop(file);
        }
        let temp = self
            .temp
            .take()
            .ok_or_else(|| anyhow!("missing recording staging file"))?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(&self.output)?;
        if let Some(error) = failure {
            return Err(error.context(format!("recording retained at {}", self.output.display())));
        }
        Ok(())
    }
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        if let Err(error) = self.finish(false) {
            eprintln!("Rerun partial-recording finalization failed: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoeq::optim::ObjectiveDataBuilder;
    use ndarray::array;

    fn data() -> ObjectiveData {
        ObjectiveDataBuilder::new(
            array![100.0, 1000.0, 10000.0],
            array![0.0, 0.0, 0.0],
            array![2.0, -6.0, 1.0],
            48000.0,
            autoeq::PeqModel::Pk,
            autoeq::LossType::SpeakerFlat,
        )
        .freq_range(20.0, 20000.0)
        .max_db(6.0)
        .min_db(-12.0)
        .build()
        .unwrap()
    }
    // Serialization fixture only; direct real-solver parity is exercised offline.
    fn checkpoint(generation: usize, finalized: bool) -> DECheckpoint {
        serde_json::from_value(json!({"checkpoint_version":1,"run_identity":"test-run",
            "implementation_identity":"test","solver_source_identity":"sha256:source",
            "target_identity":"test-target","build_identity":"sha256:build",
            "configuration_fingerprint":"sha256:config","seed":42,"generation":generation,
            "evaluations":generation*4,"population":[[3.0,1.0,-6.0]],
            "population_fitness":[{"kind":"finite","value":5.0}],"best_index":0,
            "best_params":[3.0,1.0,-6.0],"best_fitness":{"kind":"finite","value":5.0},
            "rng_state":[1,2,3],"adaptive":null,"archive":null,
            "terminal":if finalized {json!({"success":true,"message":"test fixture",
                "finalized":true,"final_params":[3.0,1.0,-6.0],
                "final_fitness":{"kind":"finite","value":5.0},"polish_evaluations":0})}
                else {serde_json::Value::Null}}))
        .unwrap()
    }
    #[test]
    fn rerun_response_uses_existing_peq_and_correct_deviation_sign() {
        let d = data();
        let filter = context(&d).peq_spl(&[3.0, 1.0, -6.0]);
        assert!((filter[1] + 6.0).abs() < 1e-9);
        let baseline = &*d.target - &*d.deviation;
        assert_eq!(baseline.to_vec(), vec![-2.0, 6.0, -1.0]);
        let corrected = &filter + &baseline;
        assert!(corrected[1].abs() < 1e-9);
        let neutral = context(&d).peq_spl(&[3.0, 1.0, 0.0]);
        assert!(neutral.iter().all(|v| v.abs() < 1e-9));
    }
    #[test]
    fn rerun_rejects_invalid_data_modes_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.rrd");
        std::fs::write(&path, b"original").unwrap();
        assert!(Telemetry::new(&path, None, data(), "test-run", "sha256:measurement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        let mut d = data();
        d.target = std::sync::Arc::new(array![f64::NAN, 0.0, 0.0]);
        assert!(validate_data(&d).is_err());
        let mut d = data();
        d.freqs = std::sync::Arc::new(array![100.0, 100.0, 1000.0]);
        assert!(validate_data(&d).is_err());
        let mut args = autoeq::cli::Args::speaker_defaults();
        args.rerun_recording = Some(path);
        args.algo = "autoeq:de".into();
        assert!(super::super::validate_checkpoint_mode(&args).is_err());
        args.seed = Some(42);
        assert!(super::super::validate_checkpoint_mode(&args).is_ok());
        args.refine = true;
        assert!(super::super::validate_checkpoint_mode(&args).is_err());
        args.refine = false;
        args.algo = "autoeq:cobyla".into();
        assert!(super::super::validate_checkpoint_mode(&args).is_err());
        args.algo = "autoeq:de".into();
        args.peq_model = autoeq::PeqModel::Free;
        assert!(super::super::validate_checkpoint_mode(&args).is_err());
    }
    #[test]
    fn rerun_records_all_barriers_and_same_generation_terminal_without_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("direct.rrd");
        let mut t = Telemetry::new(&path, None, data(), "test-run", "sha256:measurement").unwrap();
        let (rec, storage) = rerun::RecordingStreamBuilder::new("test").memory().unwrap();
        t.file = Some(rec);
        t.record(&checkpoint(0, false)).unwrap();
        t.record(&checkpoint(1, false)).unwrap();
        t.record(&checkpoint(1, true)).unwrap();
        assert_eq!(t.callbacks, 3);
        assert_eq!(t.first_generation, Some(0));
        assert_eq!(t.last_generation, Some(1));
        assert!(t.terminal_observed);
        let debug = format!("{:?}", storage.take());
        assert!(debug.contains("frequency_response/predicted_corrected"));
        assert!(debug.contains("arrays/candidate_filter_db"));
        assert!(debug.contains("callback_index"));
        assert!(debug.contains("generation"));
        assert!(t.record(&checkpoint(3, false)).is_err());
        t.finish(false).unwrap();
    }
    #[test]
    fn rerun_failure_retains_partial_recording_and_rejects_nonfinite_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.rrd");
        let mut t = Telemetry::new(&path, None, data(), "test-run", "sha256:measurement").unwrap();
        t.record(&checkpoint(0, false)).unwrap();
        let mut bad = checkpoint(1, false);
        bad.best_params[0] = f64::INFINITY;
        assert!(t.record(&bad).is_err());
        t.finish(false).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        assert!(Telemetry::new(&path, None, data(), "test-run", "sha256:measurement").is_err());
    }
}
