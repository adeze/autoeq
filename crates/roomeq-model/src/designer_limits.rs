//! Shared offline Designer limits used by native and headless command handlers.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignerLimits {
    /// Lower correction-band boundary, Hz.
    pub min_freq: f64,
    /// Upper correction-band boundary, Hz, strictly below native Nyquist.
    pub max_freq: f64,
    /// Signed minimum filter gain, dB.
    pub min_db: f64,
    /// Signed maximum filter gain, dB.
    pub max_db: f64,
}

impl DesignerLimits {
    /// Validate an atomic edit against the same limits as the native Room EQ UI.
    pub fn updated(
        self,
        min_freq: Option<f64>,
        max_freq: Option<f64>,
        min_db: Option<f64>,
        max_db: Option<f64>,
        sample_rate: f64,
    ) -> Result<Self, String> {
        let next = Self {
            min_freq: min_freq.unwrap_or(self.min_freq),
            max_freq: max_freq.unwrap_or(self.max_freq),
            min_db: min_db.unwrap_or(self.min_db),
            max_db: max_db.unwrap_or(self.max_db),
        };
        let Self {
            min_freq: low,
            max_freq: high,
            min_db: cut,
            max_db: boost,
        } = next;
        if !sample_rate.is_finite()
            || sample_rate <= 0.0
            || ![low, high, cut, boost].iter().all(|v| v.is_finite())
            || !(20.0..=20_000.0).contains(&low)
            || !(20.0..=20_000.0).contains(&high)
            || low >= high
            || high >= sample_rate / 2.0
            || !(-25.0..=25.0).contains(&cut)
            || !(-25.0..=25.0).contains(&boost)
            || cut > boost
        {
            return Err("Invalid Designer limits: frequencies must be 20–20000 Hz, ordered below Nyquist; gains must be ordered within -25–25 dB".into());
        }
        if next == self {
            return Err("Designer update contains no changes".into());
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::DesignerLimits;
    #[test]
    fn shared_limits_preserve_native_validation() {
        let limits = DesignerLimits {
            min_freq: 20.,
            max_freq: 300.,
            min_db: -12.,
            max_db: 0.,
        };
        assert_eq!(
            limits
                .updated(None, Some(400.), None, None, 48000.)
                .unwrap()
                .max_freq,
            400.
        );
        assert!(limits.updated(None, None, None, None, 48000.).is_err());
        for rate in [0., f64::NAN, 800.] {
            assert!(limits.updated(None, Some(400.), None, None, rate).is_err());
        }
        assert!(
            limits
                .updated(Some(500.), None, None, None, 48000.)
                .is_err()
        );
    }
}
