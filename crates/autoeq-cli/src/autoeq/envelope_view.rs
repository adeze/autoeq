//! Display-only geometry shared by direct telemetry and the small offline viewer.
use super::{
    display_smoothing::{Mode, smooth},
    response_envelope::Envelope,
};

pub struct Chart {
    pub min_hz: f64,
    pub max_hz: f64,
    pub min_db: f64,
    pub max_db: f64,
}
impl Chart {
    fn point(&self, hz: f64, db: f64) -> [f32; 2] {
        [
            (12. * (hz.log10() - self.min_hz.log10()) / (self.max_hz.log10() - self.min_hz.log10()))
                as f32,
            (6. * (self.max_db - db) / (self.max_db - self.min_db)) as f32,
        ]
    }
    fn validate(&self) -> Result<(), String> {
        if ![
            self.min_hz,
            self.max_hz,
            self.min_db,
            self.max_db,
            self.max_db - self.min_db,
        ]
        .iter()
        .all(|v| v.is_finite())
            || self.min_hz <= 0.
            || self.max_hz <= self.min_hz
            || self.max_db <= self.min_db
        {
            return Err("Invalid common chart domain".into());
        }
        Ok(())
    }
    #[allow(dead_code)] // CLI telemetry supplies its own axes; standalone views use these.
    pub fn axes(&self, rec: &rerun::RecordingStream, root: &str) -> Result<(), String> {
        self.validate()?;
        let mut lines = Vec::new();
        let mut points = Vec::new();
        let mut labels = Vec::new();
        for hz in [
            20., 50., 100., 200., 500., 1000., 2000., 5000., 10000., 20000.,
        ] {
            if hz < self.min_hz || hz > self.max_hz {
                continue;
            }
            let x = self.point(hz, 0.)[0];
            lines.push(vec![[x, 0.], [x, 6.]]);
            points.push([x, 6.3]);
            labels.push(format!("{hz:.0} Hz"));
        }
        let step = ((self.max_db - self.min_db) / 6. / 5.).ceil().max(1.) * 5.;
        let mut db = (self.min_db / step).ceil() * step;
        while db <= self.max_db {
            let y = self.point(self.min_hz, db)[1];
            lines.push(vec![[0., y], [12., y]]);
            points.push([-0.5, y]);
            labels.push(format!("{db:.0} dB"));
            db += step;
        }
        rec.log_static(
            format!("{root}/axes"),
            &rerun::LineStrips2D::new(lines).with_colors([0x68708070u32]),
        )
        .map_err(|e| e.to_string())?;
        rec.log_static(
            format!("{root}/ticks"),
            &rerun::Points2D::new(points)
                .with_labels(labels)
                .with_show_labels(true)
                .with_radii([0.]),
        )
        .map_err(|e| e.to_string())
    }
    fn curve(
        &self,
        rec: &rerun::RecordingStream,
        path: &str,
        hz: &[f64],
        db: &[f64],
        color: [u8; 4],
    ) -> Result<(), String> {
        let points = hz
            .iter()
            .zip(db)
            .map(|(&h, &d)| self.point(h, d))
            .collect::<Vec<_>>();
        if points.iter().flatten().any(|x| !x.is_finite()) {
            return Err("Curve exceeds finite display coordinates".into());
        }
        rec.log(
            path,
            &rerun::LineStrips2D::new([points]).with_colors([color]),
        )
        .map_err(|e| e.to_string())
    }
    // Display intervals cover log-frequency cells; raw f64 statistics remain in the recording.
    fn band(
        &self,
        rec: &rerun::RecordingStream,
        path: &str,
        hz: &[f64],
        lo: &[f64],
        hi: &[f64],
        color: [u8; 4],
    ) -> Result<(), String> {
        let lines = hz
            .iter()
            .zip(lo)
            .zip(hi)
            .map(|((&h, &l), &u)| vec![self.point(h, l), self.point(h, u)])
            .collect::<Vec<_>>();
        let xs = hz.iter().map(|&h| self.point(h, 0.)[0]).collect::<Vec<_>>();
        let radii = (0..xs.len())
            .map(|i| {
                let left = if i > 0 {
                    xs[i] - xs[i - 1]
                } else {
                    xs[1] - xs[0]
                };
                let right = if i + 1 < xs.len() {
                    xs[i + 1] - xs[i]
                } else {
                    left
                };
                rerun::Radius::new_scene_units(0.25 * (left + right))
            })
            .collect::<Vec<_>>();
        if lines.iter().flatten().flatten().any(|x| !x.is_finite()) {
            return Err("Band exceeds finite display coordinates".into());
        }
        rec.log(
            path,
            &rerun::LineStrips2D::new(lines)
                .with_colors([color])
                .with_radii(radii),
        )
        .map_err(|e| e.to_string())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn log(
        &self,
        rec: &rerun::RecordingStream,
        root: &str,
        hz: &[f64],
        baseline: &[f64],
        target: &[f64],
        best: &[f64],
        envelope: &Envelope,
        mode: Mode,
        color: [u8; 3],
    ) -> Result<(), String> {
        self.validate()?;
        if hz.len() < 2
            || [&envelope.min, &envelope.max, &envelope.p10, &envelope.p90]
                .iter()
                .any(|v| v.len() != hz.len() || v.iter().any(|x| !x.is_finite()))
        {
            return Err(
                "Envelope chart requires paired finite arrays and at least two frequencies".into(),
            );
        }
        let baseline = smooth(hz, baseline, mode).map_err(|e| e.to_string())?;
        let target = smooth(hz, target, mode).map_err(|e| e.to_string())?;
        let best = smooth(hz, best, mode).map_err(|e| e.to_string())?;
        self.band(
            rec,
            &format!("{root}/population_min_max"),
            hz,
            &envelope.min,
            &envelope.max,
            [color[0], color[1], color[2], 28],
        )?;
        self.band(
            rec,
            &format!("{root}/population_p10_p90"),
            hz,
            &envelope.p10,
            &envelope.p90,
            [color[0], color[1], color[2], 65],
        )?;
        for (name, bound) in [("minimum", &envelope.min), ("maximum", &envelope.max)] {
            self.curve(
                rec,
                &format!("{root}/{name}"),
                hz,
                bound,
                [color[0], color[1], color[2], 50],
            )?;
        }
        self.curve(
            rec,
            &format!("{root}/baseline"),
            hz,
            &baseline,
            [color[0], color[1], color[2], 85],
        )?;
        self.curve(
            rec,
            &format!("{root}/target"),
            hz,
            &target,
            [220, 175, 65, 255],
        )?;
        self.curve(
            rec,
            &format!("{root}/best"),
            hz,
            &best,
            [color[0], color[1], color[2], 255],
        )?;
        for (name, values) in [
            ("min", &envelope.min),
            ("max", &envelope.max),
            ("p10", &envelope.p10),
            ("p90", &envelope.p90),
        ] {
            rec.log(
                format!("{root}/arrays/{name}_db"),
                &rerun::Tensor::new(rerun::encodings::TensorData::new(
                    vec![values.len() as u64],
                    rerun::encodings::TensorBuffer::F64(values.clone().into()),
                )),
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_finite_axes_and_invalid_domain() {
        let chart = Chart {
            min_hz: 1e-300,
            max_hz: 1e300,
            min_db: -100.,
            max_db: 100.,
        };
        assert!(chart.validate().is_ok());
        assert_eq!(chart.point(1., 0.), [6., 3.]);
        assert!(
            Chart {
                min_hz: 0.,
                ..chart
            }
            .validate()
            .is_err()
        );
        assert!(
            Chart {
                min_db: -f64::MAX,
                max_db: f64::MAX,
                ..chart
            }
            .validate()
            .is_err()
        );
    }
}
