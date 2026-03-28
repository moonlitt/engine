//! TPDF (Triangular Probability Density Function) dithering.
//!
//! Applied at the master bus output stage to eliminate quantization
//! distortion when converting from float to the DAC's bit depth.
//!
//! TPDF = difference of two uniform random variables → triangular PDF.
//! This completely eliminates quantization-correlated distortion at
//! the cost of a flat noise floor at -24 × target_bits dB.

/// TPDF dither state. One instance per channel.
pub struct Dither {
    /// LCG state for fast, deterministic PRNG.
    state: u32,
    /// Dither amplitude = 1.0 / 2^target_bits.
    amplitude: f32,
}

impl Dither {
    /// Create a new dither with the given target bit depth and seed.
    pub fn new(target_bits: u32, seed: u32) -> Self {
        Self {
            state: seed,
            amplitude: 1.0 / (1u64 << target_bits) as f32,
        }
    }

    /// Create a 24-bit dither (macOS CoreAudio default).
    pub fn new_24bit(seed: u32) -> Self {
        Self::new(24, seed)
    }

    /// Generate next uniform random in [0, 1).
    #[inline]
    fn next_uniform(&mut self) -> f32 {
        // LCG: fast, sufficient for dither (not crypto)
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        // Upper bits have better statistical properties
        (self.state >> 8) as f32 / 16777216.0 // 2^24
    }

    /// Apply TPDF dither to a single sample.
    #[inline]
    pub fn process(&mut self, sample: f32) -> f32 {
        let r1 = self.next_uniform();
        let r2 = self.next_uniform();
        sample + (r1 - r2) * self.amplitude
    }

    /// Apply TPDF dither to a buffer in-place.
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for s in buffer.iter_mut() {
            *s = self.process(*s);
        }
    }
}

/// Stereo dither pair with independent seeds per channel.
pub struct StereoDither {
    pub left: Dither,
    pub right: Dither,
}

impl StereoDither {
    pub fn new_24bit() -> Self {
        Self {
            // Different seeds for uncorrelated L/R noise
            left: Dither::new_24bit(0xDEADBEEF),
            right: Dither::new_24bit(0xCAFEBABE),
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.left.process_buffer(left);
        self.right.process_buffer(right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dither_output_differs_from_input() {
        // Use 16-bit dither (larger amplitude) so f32 can represent the difference
        let mut d = Dither::new(16, 42);
        let input = 0.0f32;
        let output = d.process(input);
        // Dither adds noise, so output should differ
        assert_ne!(input, output);
    }

    #[test]
    fn test_dither_amplitude_bounded() {
        let mut d = Dither::new_24bit(42);
        let input = 0.0f32;
        for _ in 0..10000 {
            let output = d.process(input);
            // TPDF range is [-amplitude, +amplitude], amplitude = 1/2^24 ≈ 6e-8
            assert!(output.abs() < 2.0 * d.amplitude,
                "dither output {} exceeds expected range", output);
        }
    }

    #[test]
    fn test_dither_mean_near_zero() {
        let mut d = Dither::new_24bit(42);
        let n = 100_000;
        let sum: f64 = (0..n).map(|_| d.process(0.0) as f64).sum();
        let mean = sum / n as f64;
        // Mean of TPDF should be ~0 (unbiased)
        assert!(mean.abs() < 1e-6, "dither mean {} is not near zero", mean);
    }

    #[test]
    fn test_stereo_dither_uncorrelated() {
        let mut sd = StereoDither::new_24bit();
        let mut left = vec![0.0f32; 1000];
        let mut right = vec![0.0f32; 1000];
        sd.process(&mut left, &mut right);
        // L and R should differ (different seeds)
        assert_ne!(left, right);
    }
}
