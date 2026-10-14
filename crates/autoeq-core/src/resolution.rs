//! Numerical feature-width support retained independently of interpolation grids.
use crate::Curve;
use std::sync::Arc;

/// Original sample grids supporting a derived frequency response.
///
/// This bounds numerical sampling support, not physical acquisition accuracy.
/// The local width requires two original sampling intervals. Explicit smoothing
/// widths may increase that lower bound, but interpolation cannot reduce it.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementResolution {
    grids: Vec<Vec<f64>>,
    smoothing: Vec<Vec<(f64, f64)>>,
}

impl MeasurementResolution {
    /// Represent source data without usable numerical resolution support.
    ///
    /// Derived curves retain this marker instead of treating interpolated
    /// samples as a new acquisition. Every width and Q query returns `None`.
    pub fn unsupported() -> Self {
        Self {
            grids: Vec::new(),
            smoothing: Vec::new(),
        }
    }

    /// Retain the positive-frequency portion of one original sampling grid.
    ///
    /// Returns `None` for fewer than two positive samples, nonfinite values,
    /// negative frequencies, or a non-increasing grid. A DC sample is allowed.
    pub fn from_grid(grid: &[f64]) -> Option<Self> {
        if grid.iter().any(|value| !value.is_finite() || *value < 0.0)
            || grid.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return None;
        }
        let positive: Vec<_> = grid.iter().copied().filter(|value| *value > 0.0).collect();
        (positive.len() >= 2).then_some(Self {
            grids: vec![positive],
            smoothing: Vec::new(),
        })
    }

    /// Combine original support without treating averaged samples as new data.
    ///
    /// Invalid inputs retain an unsupported marker; empty input returns `None`.
    /// Identical grids are stored once; all distinct source grids remain visible.
    pub fn combine<'a>(curves: impl IntoIterator<Item = &'a Curve>) -> Option<Arc<Self>> {
        let mut grids = Vec::new();
        let mut smoothing = Vec::new();
        for curve in curves {
            let support = curve.retained_resolution();
            if support.grids.is_empty() {
                return Some(Arc::new(Self::unsupported()));
            }
            for grid in &support.grids {
                if !grids.contains(grid) {
                    grids.push(grid.clone());
                }
            }
            for profile in &support.smoothing {
                if !smoothing.contains(profile) {
                    smoothing.push(profile.clone());
                }
            }
        }
        (!grids.is_empty()).then(|| Arc::new(Self { grids, smoothing }))
    }

    /// Retain the widths actually applied by a magnitude smoother.
    ///
    /// Each knot is `(frequency_hz, width_octaves)`. Returns `None` for invalid
    /// knots. Success preserves every original grid and previous smoothing floor.
    pub fn with_smoothing(&self, widths: Vec<(f64, f64)>) -> Option<Arc<Self>> {
        if widths.is_empty()
            || widths
                .iter()
                .any(|(f, w)| !f.is_finite() || *f <= 0.0 || !w.is_finite() || *w <= 0.0)
            || widths.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        {
            return None;
        }
        let mut updated = self.clone();
        if !updated.smoothing.contains(&widths) {
            updated.smoothing.push(widths);
        }
        Some(Arc::new(updated))
    }

    /// Return the frequency interval covered by every contributing source.
    ///
    /// Unsupported or disjoint source grids return `None`; edge extrapolation
    /// cannot enlarge this interval.
    pub fn support_band_hz(&self) -> Option<[f64; 2]> {
        let minimum = self
            .grids
            .iter()
            .filter_map(|grid| grid.first().copied())
            .reduce(f64::max)?;
        let maximum = self
            .grids
            .iter()
            .filter_map(|grid| grid.last().copied())
            .reduce(f64::min)?;
        (maximum > minimum).then_some([minimum, maximum])
    }

    /// Return a conservative local feature width in octaves.
    ///
    /// Uses the larger adjacent original-grid spacing, requiring two intervals.
    /// The coarsest source wins. Returns `None` outside any contributing
    /// source's support or for invalid frequencies; no edge extrapolation.
    pub fn width_octaves(&self, frequency: f64) -> Option<f64> {
        if !frequency.is_finite() || frequency <= 0.0 {
            return None;
        }
        let sampling = self
            .grids
            .iter()
            .map(|grid| {
                if frequency < grid[0] || frequency > *grid.last()? {
                    return None;
                }
                let upper = grid
                    .partition_point(|sample| *sample < frequency)
                    .min(grid.len() - 1);
                let lower = upper.saturating_sub(1);
                let first = lower.saturating_sub(1);
                let last = (upper + 1).min(grid.len() - 1);
                // Two independent intervals are an engineering sampling floor,
                // not a calibrated estimate of an instrument's resolving power.
                Some(
                    2.0 * grid[first..=last]
                        .windows(2)
                        .map(|pair| (pair[1] / pair[0]).log2())
                        .fold(0.0_f64, f64::max),
                )
            })
            .try_fold(0.0_f64, |width, next| next.map(|next| width.max(next)))?;
        if sampling <= 0.0 {
            return None;
        }
        let smoothing = self
            .smoothing
            .iter()
            .filter_map(|profile| {
                if frequency < profile[0].0 || frequency > profile.last()?.0 {
                    return None;
                }
                let upper = profile
                    .partition_point(|(f, _)| *f < frequency)
                    .min(profile.len() - 1);
                Some(profile[upper].1.max(profile[upper.saturating_sub(1)].1))
            })
            .fold(0.0_f64, f64::max);
        Some(sampling.max(smoothing))
    }

    /// Return the widest retained feature floor over a supported band.
    ///
    /// Inspects original-grid and smoothing-profile knots as well as both band
    /// edges, so a derived grid cannot hide an intermediate coarse region.
    /// Returns `None` for invalid, unsupported, or nonfinite-width bands.
    pub fn max_width_octaves(&self, band: [f64; 2]) -> Option<f64> {
        let [low, high] = band;
        let [source_low, source_high] = self.support_band_hz()?;
        if !low.is_finite()
            || !high.is_finite()
            || low <= 0.0
            || high <= low
            || low < source_low
            || high > source_high
        {
            return None;
        }
        let maximum = self
            .grids
            .iter()
            .flat_map(|grid| grid.iter().copied())
            .chain(
                self.smoothing
                    .iter()
                    .flat_map(|profile| profile.iter().map(|(frequency, _)| *frequency)),
            )
            .filter(|frequency| (low..=high).contains(frequency))
            .chain([low, high])
            .try_fold(0.0_f64, |width, frequency| {
                self.width_octaves(frequency).map(|next| width.max(next))
            })?;
        (maximum.is_finite() && maximum > 0.0).then_some(maximum)
    }

    /// Return a digital peak-filter Q ceiling for the warranted bandwidth.
    ///
    /// Uses the Audio EQ Cookbook bandwidth relation with bilinear warping.
    /// Smoothing width is in octaves and can only broaden the sampling floor.
    /// Returns `None` outside original support or for invalid inputs/Nyquist.
    pub fn max_peak_q(
        &self,
        frequency: f64,
        sample_rate: f64,
        smoothing_octaves: f64,
    ) -> Option<f64> {
        if !sample_rate.is_finite()
            || sample_rate <= 0.0
            || frequency >= sample_rate / 2.0
            || !smoothing_octaves.is_finite()
            || smoothing_octaves < 0.0
        {
            return None;
        }
        let bandwidth = self.width_octaves(frequency)?.max(smoothing_octaves);
        let omega = std::f64::consts::TAU * frequency / sample_rate;
        let q =
            1.0 / (2.0 * (std::f64::consts::LN_2 / 2.0 * bandwidth * omega / omega.sin()).sinh());
        (q.is_finite() && q > 0.0).then_some(q)
    }
}

impl Curve {
    /// Preserve numerical support or an explicit unsupported-data marker.
    ///
    /// Transformations must use this method so invalid original samples cannot
    /// acquire resolution merely by being replaced with a valid output grid.
    pub fn retained_resolution(&self) -> Arc<MeasurementResolution> {
        self.resolution_warrant()
            .unwrap_or_else(|| Arc::new(MeasurementResolution::unsupported()))
    }

    /// Return original support, deriving it only for an unannotated input grid.
    ///
    /// Invalid or insufficient input grids yield `None` rather than invented support.
    pub fn resolution_warrant(&self) -> Option<Arc<MeasurementResolution>> {
        self.resolution
            .clone()
            .or_else(|| MeasurementResolution::from_grid(self.freq.as_slice()?).map(Arc::new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array1;

    #[test]
    fn interpolation_does_not_create_narrower_feature_support() {
        let coarse = Curve {
            freq: Array1::from_vec(vec![20.0, 40.0, 80.0, 160.0]),
            spl: Array1::zeros(4),
            ..Default::default()
        };
        let dense = crate::interpolate_log_space(&Array1::linspace(20.0, 160.0, 2048), &coarse);
        let original = coarse.resolution_warrant().unwrap();
        let support = dense.resolution_warrant().unwrap();
        assert_eq!(*original, *support);
        assert_eq!(support.width_octaves(60.0), Some(2.0));
        assert!(support.max_peak_q(60.0, 48000.0, 0.0).unwrap() < 0.7);
        assert_eq!(support.width_octaves(200.0), None);
    }

    #[test]
    fn band_width_query_includes_smoothing_knots_hidden_by_resampling() {
        let grid: Vec<_> = (20..=200).map(f64::from).collect();
        let support = MeasurementResolution::from_grid(&grid)
            .unwrap()
            .with_smoothing(vec![(20.0, 0.1), (80.0, 2.0), (120.0, 0.1), (200.0, 0.1)])
            .unwrap();
        assert!(support.width_octaves(20.0).unwrap() < 0.2);
        assert!(support.width_octaves(200.0).unwrap() < 0.2);
        assert_eq!(support.max_width_octaves([20.0, 200.0]), Some(2.0));
        assert_eq!(support.max_width_octaves([19.0, 200.0]), None);
        assert_eq!(support.max_width_octaves([20.0, f64::NAN]), None);
    }

    #[test]
    fn dense_bass_and_smoothed_treble_have_different_warranted_q() {
        let grid: Vec<_> = (0..=960)
            .map(|i| 20.0 * 2.0_f64.powf(i as f64 / 96.0))
            .collect();
        let support = MeasurementResolution::from_grid(&grid).unwrap();
        assert!(support.max_peak_q(40.0, 48000.0, 1.0 / 48.0).unwrap() > 6.0);
        assert!(support.max_peak_q(20000.0, 48000.0, 1.0 / 6.0).unwrap() < 2.0);
        assert_eq!(support.max_peak_q(24000.0, 48000.0, 0.0), None);
    }

    #[test]
    fn invalid_input_does_not_acquire_support_after_interpolation() {
        let empty = Curve::default();
        let derived = crate::interpolate(&Array1::linspace(20.0, 200.0, 64), &empty);
        assert!(derived.resolution.is_some());
        assert_eq!(derived.retained_resolution().width_octaves(100.0), None);
        let smoothed = crate::smooth_one_over_n_octave(&derived, 12);
        assert_eq!(smoothed.retained_resolution().width_octaves(100.0), None);
    }

    #[test]
    fn combined_support_keeps_coarse_grid_and_requires_every_source() {
        let coarse = Curve {
            freq: Array1::from_vec(vec![20.0, 40.0, 80.0, 160.0]),
            spl: Array1::zeros(4),
            ..Default::default()
        };
        let fine = Curve {
            freq: Array1::linspace(40.0, 160.0, 64),
            spl: Array1::zeros(64),
            ..Default::default()
        };
        let support = MeasurementResolution::combine([&coarse, &fine]).unwrap();
        assert_eq!(support.width_octaves(80.0), Some(2.0));
        assert_eq!(support.width_octaves(30.0), None);
        let invalid = Curve::default();
        assert_eq!(
            MeasurementResolution::combine([&fine, &invalid])
                .unwrap()
                .width_octaves(80.0),
            None
        );
    }

    #[test]
    fn later_smoothing_and_resampling_cannot_shrink_an_applied_width() {
        let source = Curve {
            freq: Array1::linspace(100.0, 1000.0, 1024),
            spl: Array1::zeros(1024),
            ..Default::default()
        };
        let broad = crate::smooth_one_over_n_octave(&source, 2);
        let fine = crate::smooth_one_over_n_octave(&broad, 48);
        let derived = crate::interpolate_log_space(&Array1::linspace(200.0, 900.0, 2048), &fine);
        assert_eq!(
            derived.retained_resolution().width_octaves(500.0),
            Some(0.5)
        );
        assert_eq!(source.retained_resolution().width_octaves(2000.0), None);
    }

    #[test]
    fn correction_basis_has_full_width_features_and_smooth_band_edges() {
        let source = Curve {
            freq: Array1::logspace(10.0, 20.0_f64.log10(), 2000.0_f64.log10(), 2048),
            spl: Array1::zeros(2048),
            ..Default::default()
        };
        let proposal = Array1::from_elem(2048, 6.0);
        let basis = crate::curve_transforms::ResolutionMagnitudeBasis::new(
            &source,
            &proposal,
            0.5,
            [20.0, 2000.0],
        )
        .unwrap();
        assert_eq!(basis.at(20.0), 0.0);
        assert_eq!(basis.at(2000.0), 0.0);
        assert_eq!(basis.at(19.0), 0.0);
        assert_eq!(basis.at(2100.0), 0.0);
        assert!((basis.at(20.0 * 2.0_f64.powf(0.5)) - 6.0).abs() < 1e-10);
        assert!((basis.at(20.0 * 2.0_f64.powf(0.25)) - 3.0).abs() < 1e-10);
        // At an edge, a tiny frequency advance yields only quadratic onset,
        // rather than jumping by the six-dB interior correction in one bin.
        assert!(basis.at(20.0 * 2.0_f64.powf(0.001)) < 0.0001);
        let dense = basis.evaluate(&Array1::logspace(
            10.0,
            19.0_f64.log10(),
            2100.0_f64.log10(),
            65536,
        ));
        assert!(
            dense
                .iter()
                .all(|gain| *gain >= 0.0 && *gain <= 6.0 + 1e-10)
        );
        let narrow = crate::curve_transforms::ResolutionMagnitudeBasis::new(
            &source,
            &proposal,
            0.5,
            [20.0, 30.0],
        )
        .unwrap();
        assert_eq!(narrow.at(25.0), 0.0);
    }

    #[test]
    fn correction_smoothing_cannot_follow_a_spike_on_an_artificially_dense_grid() {
        let original = Curve {
            freq: Array1::from_vec(vec![20.0, 40.0, 80.0, 160.0]),
            spl: Array1::zeros(4),
            phase: Some(Array1::from_elem(4, 37.0)),
            ..Default::default()
        };
        let dense = crate::interpolate_log_space(&Array1::linspace(20.0, 160.0, 2048), &original);
        let mut correction = Array1::zeros(2048);
        let phase_before = dense.phase.clone();
        correction[878] = 10.0;
        let limited = crate::curve_transforms::smooth_correction_to_resolution(
            &dense,
            &correction,
            0.0,
            [20.0, 160.0],
        )
        .unwrap();
        assert!(limited.iter().all(|value| *value >= 0.0 && *value < 0.1));
        assert!(dense.spl.iter().all(|value| *value == 0.0));
        assert_eq!(dense.phase, phase_before);
        let too_narrow = crate::curve_transforms::smooth_correction_to_resolution(
            &dense,
            &correction,
            0.0,
            [60.0, 100.0],
        )
        .unwrap();
        assert!(too_narrow.iter().all(|value| *value == 0.0));
    }

    #[test]
    fn correction_smoothing_refuses_extrapolated_or_invalid_support() {
        let original = Curve {
            freq: Array1::from_vec(vec![20.0, 40.0, 80.0, 160.0]),
            spl: Array1::zeros(4),
            ..Default::default()
        };
        let extrapolated =
            crate::interpolate_log_space(&Array1::linspace(20.0, 200.0, 64), &original);
        assert!(
            crate::curve_transforms::smooth_correction_to_resolution(
                &extrapolated,
                &Array1::zeros(64),
                0.0,
                [20.0, 200.0]
            )
            .is_err()
        );
        assert!(
            crate::curve_transforms::smooth_correction_to_resolution(
                &original,
                &Array1::zeros(4),
                f64::NAN,
                [20.0, 160.0]
            )
            .is_err()
        );
    }
}
