//! Lowpass Resonant Biquad Filter
//!
//! Implements the standard 2-pole lowpass from Robert Bristow-Johnson's
//! "Audio EQ Cookbook" (musicdsp.org). This is the same filter type
//! specified by the SF2 standard for initialFilterFc / initialFilterQ.
//!
//! Transfer function: H(s) = 1 / (s² + s/Q + 1)
//!
//! Direct Form II Transposed implementation for numerical stability.

use std::f64::consts::PI;

pub struct LowpassFilter {
    // Coefficients (normalized)
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    // State (Direct Form II Transposed)
    z1: f32,
    z2: f32,
    // Config
    sample_rate: u32,
}

impl LowpassFilter {
    pub fn new(sample_rate: u32) -> Self {
        let mut f = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
            sample_rate,
        };
        f.set_params(20000.0, 0.0);
        f
    }

    /// Set filter parameters.
    /// - `cutoff_hz`: cutoff frequency in Hz (20..20000)
    /// - `q_db`: resonance in decibels (0..96). 0 = no resonance.
    pub fn set_params(&mut self, cutoff_hz: f32, q_db: f32) {
        let (b0, b1, b2, a1, a2) = self.coefficients(cutoff_hz, q_db);
        self.b0 = b0;
        self.b1 = b1;
        self.b2 = b2;
        self.a1 = a1;
        self.a2 = a2;
    }

    /// Calculate biquad coefficients for given cutoff and Q.
    /// Returns (b0, b1, b2, a1, a2) normalized by a0.
    ///
    /// Audio EQ Cookbook lowpass:
    ///   w0 = 2π × fc / fs
    ///   alpha = sin(w0) / (2Q)
    ///   b0 = (1 - cos(w0)) / 2
    ///   b1 = 1 - cos(w0)
    ///   b2 = (1 - cos(w0)) / 2
    ///   a0 = 1 + alpha
    ///   a1 = -2 × cos(w0)
    ///   a2 = 1 - alpha
    pub fn coefficients(&self, cutoff_hz: f32, q_db: f32) -> (f32, f32, f32, f32, f32) {
        let fc = (cutoff_hz as f64).clamp(20.0, self.sample_rate as f64 * 0.499);
        let fs = self.sample_rate as f64;

        // Convert Q from dB to linear
        // SF2 Q is in centibels, but our API uses dB for simplicity
        let q_linear = if q_db <= 0.0 {
            std::f64::consts::FRAC_1_SQRT_2 // ~0.707, Butterworth (no peak)
        } else {
            10.0f64.powf(q_db as f64 / 20.0)
        };

        let w0 = 2.0 * PI * fc / fs;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q_linear);

        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        // Normalize by a0
        (
            (b0 / a0) as f32,
            (b1 / a0) as f32,
            (b2 / a0) as f32,
            (a1 / a0) as f32,
            (a2 / a0) as f32,
        )
    }

    /// Process one sample through the filter.
    /// Direct Form II Transposed:
    ///   y[n] = b0*x[n] + z1
    ///   z1 = b1*x[n] - a1*y[n] + z2
    ///   z2 = b2*x[n] - a2*y[n]
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        output
    }

    /// Reset filter state (clear delay line).
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}
