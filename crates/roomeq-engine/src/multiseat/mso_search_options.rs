use super::consts::{MSO_ALLPASS_Q_MAX, MSO_ALLPASS_Q_MIN};
use super::mso::mso_params_per_optimized_sub;
use super::multi_seat_measurements::MultiSeatMeasurements;
use super::types::MsoSolution;
use roomeq_model::MultiSeatConfig;

#[derive(Debug, Clone, Copy)]
pub(super) struct MsoSearchOptions {
    pub(super) optimize_polarity: bool,
    pub(super) allpass_filters_per_sub: usize,
    pub(super) allpass_min_freq: f64,
    pub(super) allpass_max_freq: f64,
    pub(super) allpass_min_q: f64,
    pub(super) allpass_max_q: f64,
}

impl MsoSearchOptions {
    pub(super) fn from_config(config: &MultiSeatConfig, min_freq: f64, max_freq: f64) -> Self {
        let allpass_min_freq = min_freq.max(20.0);
        let usable_allpass_max = max_freq.min(200.0);
        let allpass_filters_per_sub = if allpass_min_freq < usable_allpass_max {
            config.allpass_filters_per_sub
        } else {
            if config.allpass_filters_per_sub > 0 {
                log::warn!(
                    "MSO all-pass filters disabled because [{min_freq:.1}, {max_freq:.1}] Hz has no overlap with the supported 20-200 Hz all-pass band"
                );
            }
            0
        };
        let allpass_max_freq = usable_allpass_max.max(allpass_min_freq);
        Self {
            optimize_polarity: config.optimize_polarity,
            allpass_filters_per_sub,
            allpass_min_freq,
            allpass_max_freq,
            allpass_min_q: MSO_ALLPASS_Q_MIN,
            allpass_max_q: MSO_ALLPASS_Q_MAX,
        }
    }
    /// Bound the common all-pass pole bandwidth before discarding source
    /// metadata during complex-grid interpolation. Timing qualification is
    /// independent of this numerical resolution constraint.
    pub(super) fn with_measurement_resolution(
        mut self,
        measurements: &MultiSeatMeasurements,
        sample_rate: f64,
    ) -> Self {
        if self.allpass_filters_per_sub == 0 {
            return self;
        }
        let cap = autoeq_core::resolution::MeasurementResolution::combine(
            measurements.measurements.iter().flatten(),
        )
        .and_then(|support| {
            let width =
                support.max_width_octaves([self.allpass_min_freq, self.allpass_max_freq])?;
            support.max_peak_q(self.allpass_max_freq, sample_rate, width)
        });
        match cap.filter(|q| *q >= self.allpass_min_q) {
            Some(q) => {
                self.allpass_max_q = self.allpass_max_q.min(q);
                log::info!(
                    "Multi-seat all-pass numerical Q support: [{:.4}, {:.4}]",
                    self.allpass_min_q,
                    self.allpass_max_q
                );
            }
            None => {
                self.allpass_filters_per_sub = 0;
                log::warn!(
                    "Multi-seat all-pass filters disabled: retained sampling/smoothing support cannot satisfy the Q floor; gain/delay controls remain available"
                );
            }
        }
        self
    }
}

pub(super) fn decode_mso_params(
    params: &[f64],
    num_subs: usize,
    options: MsoSearchOptions,
) -> MsoSolution {
    let mut gains = vec![0.0; num_subs];
    let mut delays = vec![0.0; num_subs];
    let mut polarities = vec![false; num_subs];
    let mut allpass_filters = vec![Vec::new(); num_subs];
    let per_sub = mso_params_per_optimized_sub(options);

    for sub_idx in 1..num_subs {
        let mut offset = (sub_idx - 1) * per_sub;
        gains[sub_idx] = params[offset];
        offset += 1;
        delays[sub_idx] = params[offset];
        offset += 1;

        if options.optimize_polarity {
            polarities[sub_idx] = params[offset] >= 0.5;
            offset += 1;
        }

        for _ in 0..options.allpass_filters_per_sub {
            let freq = params[offset];
            let q = params[offset + 1];
            allpass_filters[sub_idx].push((freq, q));
            offset += 2;
        }
    }

    (gains, delays, polarities, allpass_filters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sparse_matrix() -> MultiSeatMeasurements {
        let curve = crate::Curve {
            freq: ndarray::Array1::from_vec(vec![20.0, 40.0, 80.0, 160.0, 200.0]),
            spl: ndarray::Array1::zeros(5),
            phase: Some(ndarray::Array1::zeros(5)),
            ..Default::default()
        };
        MultiSeatMeasurements::new(vec![vec![curve.clone(); 2], vec![curve; 2]]).unwrap()
    }

    #[test]
    fn all_seat_original_grids_bound_search_and_decoded_allpass_parameters() {
        let matrix = sparse_matrix();
        let config = MultiSeatConfig {
            allpass_filters_per_sub: 2,
            ..Default::default()
        };
        let options = MsoSearchOptions::from_config(&config, 20.0, 200.0)
            .with_measurement_resolution(&matrix, 48_000.0);
        assert_eq!(options.allpass_filters_per_sub, 2);
        assert!(options.allpass_max_q < 0.67);
        let (_, upper) = super::super::mso::mso_bounds(2, options);
        let (_, _, _, filters) = decode_mso_params(&upper, 2, options);
        assert_eq!(filters[1].len(), 2);
        for &(frequency, q) in &filters[1] {
            for curve in matrix.measurements.iter().flatten() {
                assert!(
                    q <= curve
                        .retained_resolution()
                        .max_peak_q(frequency, 48_000.0, 0.0)
                        .unwrap()
                );
            }
        }
        let mut dense = matrix.clone();
        let grid = ndarray::Array1::linspace(20.0, 200.0, 1000);
        for curve in dense.measurements.iter_mut().flatten() {
            *curve = autoeq_core::interpolate_response(&grid, curve);
        }
        let densified = MsoSearchOptions::from_config(&config, 20.0, 200.0)
            .with_measurement_resolution(&dense, 48_000.0);
        assert_eq!(options.allpass_max_q, densified.allpass_max_q);
    }

    #[test]
    fn unsupported_smoothing_at_one_seat_removes_allpass_dimensions() {
        let mut matrix = sparse_matrix();
        let curve = &mut matrix.measurements[1][1];
        curve.resolution = curve
            .retained_resolution()
            .with_smoothing(vec![(20.0, 4.0), (200.0, 4.0)]);
        let config = MultiSeatConfig {
            allpass_filters_per_sub: 2,
            optimize_polarity: true,
            ..Default::default()
        };
        let options = MsoSearchOptions::from_config(&config, 20.0, 200.0)
            .with_measurement_resolution(&matrix, 48_000.0);
        assert_eq!(options.allpass_filters_per_sub, 0);
        assert!(options.optimize_polarity);
        let (lower, _) = super::super::mso::mso_bounds(2, options);
        assert_eq!(lower.len(), 3, "gain, delay and polarity remain available");
    }

    #[test]
    fn allpass_search_is_disabled_when_band_starts_above_200_hz() {
        let config = MultiSeatConfig {
            allpass_filters_per_sub: 3,
            ..MultiSeatConfig::default()
        };

        let unsupported = MsoSearchOptions::from_config(&config, 250.0, 2_000.0);
        assert_eq!(unsupported.allpass_filters_per_sub, 0);
        assert_eq!(unsupported.allpass_min_freq, 250.0);
        assert_eq!(unsupported.allpass_max_freq, 250.0);

        let supported = MsoSearchOptions::from_config(&config, 20.0, 120.0);
        assert_eq!(supported.allpass_filters_per_sub, 3);
        assert_eq!(supported.allpass_max_freq, 120.0);
    }
}
