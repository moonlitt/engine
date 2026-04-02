/// Allpass filter with fixed feedback of 0.5 (standard Freeverb).
///
/// Used in series after the comb filters to add diffusion
/// without changing the frequency response magnitude.
pub struct AllpassFilter {
    buffer: Vec<f32>,
    index: usize,
}

const FEEDBACK: f32 = 0.5;

impl AllpassFilter {
    /// Create a new allpass filter with the given delay length in samples.
    pub fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
        }
    }

    /// Process one sample.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        let output = -input + buffered;
        self.buffer[self.index] = input + buffered * FEEDBACK;
        self.index += 1;
        if self.index >= self.buffer.len() {
            self.index = 0;
        }
        output
    }

    /// Clear internal state.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allpass_initial_output() {
        let mut ap = AllpassFilter::new(4);
        // First output should be -input (buffered is 0).
        let out = ap.process(1.0);
        assert!((out - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_allpass_output_decays_to_zero() {
        // After feeding an impulse and letting the filter ring,
        // the output should decay to zero (no unbounded growth).
        let mut ap = AllpassFilter::new(8);
        let _ = ap.process(1.0);
        for _ in 0..500 {
            let _ = ap.process(0.0);
        }
        let final_out = ap.process(0.0);
        assert!(
            final_out.abs() < 1e-6,
            "Allpass should decay to zero, got {final_out}"
        );
    }
}
