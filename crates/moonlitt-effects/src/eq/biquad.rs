//! Single second-order IIR filter section (biquad).
//!
//! Coefficient formulas follow the Robert Bristow-Johnson Audio EQ Cookbook
//! exactly. All arithmetic is f64 for precision; only the final audio I/O
//! touches f32.

use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Filter type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterType {
    Peak,
    LowShelf,
    HighShelf,
    Lowpass,
    Highpass,
    Notch,
}

impl FilterType {
    /// Map an integer (used in param IDs) to a filter type.
    pub fn from_index(i: u32) -> Self {
        match i {
            0 => Self::Peak,
            1 => Self::LowShelf,
            2 => Self::HighShelf,
            3 => Self::Lowpass,
            4 => Self::Highpass,
            5 => Self::Notch,
            _ => Self::Peak,
        }
    }

    pub fn to_index(self) -> u32 {
        match self {
            Self::Peak => 0,
            Self::LowShelf => 1,
            Self::HighShelf => 2,
            Self::Lowpass => 3,
            Self::Highpass => 4,
            Self::Notch => 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Coefficients
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct BiquadCoeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64, // a0 is normalized to 1.0
}

impl BiquadCoeffs {
    /// Unity gain, no filtering (pass-through).
    pub fn passthrough() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    /// Compute biquad coefficients from the Audio EQ Cookbook.
    ///
    /// * `filter_type` - the type of filter
    /// * `sample_rate` - sample rate in Hz
    /// * `freq`        - center/corner frequency in Hz
    /// * `gain_db`     - gain in dB (used by Peak, LowShelf, HighShelf)
    /// * `q`           - quality factor
    pub fn design(
        filter_type: FilterType,
        sample_rate: f64,
        freq: f64,
        gain_db: f64,
        q: f64,
    ) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match filter_type {
            FilterType::Peak => {
                // A = 10^(dBgain/40)  — note /40 for peaking EQ
                let a_lin = 10.0_f64.powf(gain_db / 40.0);
                (
                    1.0 + alpha * a_lin,        // b0
                    -2.0 * cos_w0,              // b1
                    1.0 - alpha * a_lin,        // b2
                    1.0 + alpha / a_lin,        // a0
                    -2.0 * cos_w0,              // a1
                    1.0 - alpha / a_lin,        // a2
                )
            }

            FilterType::LowShelf => {
                let a_lin = 10.0_f64.powf(gain_db / 40.0);
                let two_sqrt_a_alpha = 2.0 * a_lin.sqrt() * alpha;
                (
                    a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cos_w0 + two_sqrt_a_alpha),
                    2.0 * a_lin * ((a_lin - 1.0) - (a_lin + 1.0) * cos_w0),
                    a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cos_w0 - two_sqrt_a_alpha),
                    (a_lin + 1.0) + (a_lin - 1.0) * cos_w0 + two_sqrt_a_alpha,
                    -2.0 * ((a_lin - 1.0) + (a_lin + 1.0) * cos_w0),
                    (a_lin + 1.0) + (a_lin - 1.0) * cos_w0 - two_sqrt_a_alpha,
                )
            }

            FilterType::HighShelf => {
                let a_lin = 10.0_f64.powf(gain_db / 40.0);
                let two_sqrt_a_alpha = 2.0 * a_lin.sqrt() * alpha;
                (
                    a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cos_w0 + two_sqrt_a_alpha),
                    -2.0 * a_lin * ((a_lin - 1.0) + (a_lin + 1.0) * cos_w0),
                    a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cos_w0 - two_sqrt_a_alpha),
                    (a_lin + 1.0) - (a_lin - 1.0) * cos_w0 + two_sqrt_a_alpha,
                    2.0 * ((a_lin - 1.0) - (a_lin + 1.0) * cos_w0),
                    (a_lin + 1.0) - (a_lin - 1.0) * cos_w0 - two_sqrt_a_alpha,
                )
            }

            FilterType::Lowpass => (
                (1.0 - cos_w0) / 2.0,
                1.0 - cos_w0,
                (1.0 - cos_w0) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),

            FilterType::Highpass => (
                (1.0 + cos_w0) / 2.0,
                -(1.0 + cos_w0),
                (1.0 + cos_w0) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),

            FilterType::Notch => (
                1.0,
                -2.0 * cos_w0,
                1.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),
        };

        // Normalize by a0
        let inv_a0 = 1.0 / a0;
        Self {
            b0: b0 * inv_a0,
            b1: b1 * inv_a0,
            b2: b2 * inv_a0,
            a1: a1 * inv_a0,
            a2: a2 * inv_a0,
        }
    }
}

// ---------------------------------------------------------------------------
// Biquad state (Direct Form II Transposed)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Biquad {
    coeffs: BiquadCoeffs,
    z1: f64,
    z2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::new()
    }
}

impl Biquad {
    /// Create a new biquad with passthrough coefficients.
    pub fn new() -> Self {
        Self {
            coeffs: BiquadCoeffs::passthrough(),
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Replace the coefficients (typically after a parameter change).
    pub fn set_coeffs(&mut self, coeffs: BiquadCoeffs) {
        self.coeffs = coeffs;
    }

    /// Reset internal state to zero.
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Process a single sample (Direct Form II Transposed).
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let c = &self.coeffs;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_coeffs_leave_signal_unchanged() {
        let mut bq = Biquad::new();
        for i in 0..100 {
            let x = (i as f64) * 0.01;
            let y = bq.process(x);
            assert!(
                (y - x).abs() < 1e-15,
                "passthrough must not alter the signal"
            );
        }
    }

    #[test]
    fn reset_clears_state() {
        let mut bq = Biquad::new();
        bq.set_coeffs(BiquadCoeffs::design(
            FilterType::Peak,
            44100.0,
            1000.0,
            6.0,
            1.0,
        ));
        // Feed some signal
        for _ in 0..100 {
            bq.process(1.0);
        }
        bq.reset();
        assert_eq!(bq.z1, 0.0);
        assert_eq!(bq.z2, 0.0);
    }
}
