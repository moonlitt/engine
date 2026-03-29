//! Modulated Allpass Filter
//!
//! Used in Dattorro plate reverb tank (Dattorro 1997, §4).
//! LFO modulates the delay line read position to eliminate metallic coloration.
//! https://ccrma.stanford.edu/~dattorro/EffectDesignPart1.pdf

/// A modulated allpass filter with LFO-driven read-position modulation.
///
/// The delay line length is fixed at allocation time; the LFO sweeps the
/// fractional read pointer around the nominal tap, producing chorus-like
/// detuning inside the reverb tank.
pub struct ModAllpass {
    buffer: Vec<f32>,
    write_pos: usize,
    gain: f32,
}

impl ModAllpass {
    /// Create a new modulated allpass with the given maximum delay in samples.
    pub fn new(max_delay: usize, gain: f32) -> Self {
        Self {
            buffer: vec![0.0; max_delay.max(1)],
            write_pos: 0,
            gain,
        }
    }

    /// Clear the internal delay buffer.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }
}
