//! Canonical scalar FIR transfer evaluation shared by playback and reports.

// Rust guideline compliant 2026-02-21

use num_complex::Complex64;
use std::f64::consts::PI;

/// Evaluate the complete FIR polynomial at one frequency.
///
/// This preserves the engine's scalar contract: empty taps represent unity,
/// and evaluation frequencies may exceed Nyquist. Callers validate sample
/// rates and finite inputs at their boundary. Non-finite inputs can produce
/// non-finite output. No interpolation, truncation, or magnitude floor applies.
///
/// # Examples
/// ```
/// use autoeq_core::fir_response::fir_response_at;
/// let response = fir_response_at(&[1.0, -1.0], 0.0, 48_000.0);
/// assert_eq!(response.norm(), 0.0);
/// ```
#[must_use]
pub fn fir_response_at(taps: &[f64], frequency_hz: f64, sample_rate: f64) -> Complex64 {
    if taps.is_empty() {
        return Complex64::new(1.0, 0.0);
    }
    // Horner's method evaluates the same finite DTFT as summing tap phasors,
    // without computing sine/cosine once per tap. No grid interpolation or
    // truncation is involved, including at narrow electrical peak anchors.
    let z = Complex64::from_polar(1.0, -2.0 * PI * frequency_hz / sample_rate);
    taps.iter()
        .rev()
        .fold(Complex64::default(), |acc, tap| acc * z + tap)
}

#[cfg(test)]
mod tests {
    use super::fir_response_at;
    use num_complex::Complex64;

    #[test]
    fn preserves_signed_cancellation_latency_and_periodic_scalar_contract() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            assert_eq!(fir_response_at(&[], 0.0, rate), Complex64::new(1.0, 0.0));
            assert_eq!(
                fir_response_at(&[1.0, -1.0], 0.0, rate),
                Complex64::default()
            );
            let delayed = fir_response_at(&[0.0, 1.0], rate / 4.0, rate);
            assert!((delayed - Complex64::new(0.0, -1.0)).norm() < 1e-14);
            assert!(fir_response_at(&[1.0, 1.0], rate / 2.0, rate).norm() < 1e-14);
            assert!(
                (fir_response_at(&[0.0, 1.0], rate, rate) - Complex64::new(1.0, 0.0)).norm()
                    < 1e-14
            );
        }
    }
}
