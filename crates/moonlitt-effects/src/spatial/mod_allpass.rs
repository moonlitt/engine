//! Modulated Allpass Filter
//!
//! Used in Dattorro plate reverb tank (Dattorro 1997, Figure 1, tank section).
//! LFO modulates the delay line read position to eliminate metallic coloration.
//!
//! Reference:
//! Jon Dattorro, "Effect Design Part 1: Reverberator and Other Filters"
//! JAES Vol. 45 No. 9, 1997.
//! https://ccrma.stanford.edu/~dattorro/EffectDesignPart1.pdf

/// A modulated allpass filter with LFO-driven read-position modulation.
///
/// The delay line length is fixed at allocation time; the LFO sweeps the
/// fractional read pointer around the nominal tap, producing chorus-like
/// detuning inside the reverb tank.
pub struct ModAllpass {
    buffer: Vec<f32>,
    index: usize,
    size: usize,
    feedback: f32,
}

impl ModAllpass {
    /// Create a new modulated allpass with the given nominal delay in samples.
    /// Allocates +16 headroom for modulation excursion.
    pub fn new(size: usize, feedback: f32) -> Self {
        let alloc = size + 16;
        Self {
            buffer: vec![0.0; alloc],
            index: 0,
            size,
            feedback,
        }
    }

    /// Set the feedback coefficient (decay diffusion).
    #[inline]
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback;
    }

    /// Process one sample with modulated read position.
    ///
    /// `mod_offset` is the LFO modulation in fractional samples, added to
    /// the nominal delay length. Linear interpolation between integer neighbors.
    ///
    /// Allpass topology:
    ///   read_pos = index - (size + mod_offset)   (wrapped)
    ///   delayed  = linear_interp(buffer, read_pos)
    ///   output   = -feedback * input + delayed
    ///   buffer[index] = input + feedback * delayed
    #[inline]
    pub fn process(&mut self, input: f32, mod_offset: f32) -> f32 {
        let buf_len = self.buffer.len();

        // Fractional read position
        let read_offset = self.size as f32 + mod_offset;
        let read_pos = self.index as f32 - read_offset;

        // Wrap to positive range and split into integer + fraction
        let read_pos_wrapped = read_pos.rem_euclid(buf_len as f32);
        let idx0 = read_pos_wrapped as usize;
        let frac = read_pos_wrapped - idx0 as f32;
        let idx1 = if idx0 + 1 >= buf_len { 0 } else { idx0 + 1 };

        // Linear interpolation
        let delayed = self.buffer[idx0] * (1.0 - frac) + self.buffer[idx1] * frac;

        // Canonical Schroeder allpass — feed back the OUTPUT so DC gain is 1.
        // Feeding back `delayed` (form 1) gives DC gain (1-g+g²)/(1-g) > 1,
        // which lets the reverb tank accumulate DC across feedback cycles.
        let output = -self.feedback * input + delayed;
        self.buffer[self.index] = input + self.feedback * output;

        // Advance write pointer
        self.index += 1;
        if self.index >= buf_len {
            self.index = 0;
        }

        output
    }

    /// Read from the delay buffer at an absolute offset from the write head.
    /// Used for output taps.
    #[inline]
    pub fn read_at(&self, offset: usize) -> f32 {
        let buf_len = self.buffer.len();
        let pos = if self.index >= offset {
            self.index - offset
        } else {
            buf_len - (offset - self.index)
        };
        self.buffer[pos % buf_len]
    }

    /// Clear the internal delay buffer.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}
