/// Lowpass feedback comb filter (Freeverb variant).
///
/// The feedback path includes a one-pole lowpass that simulates
/// high-frequency absorption in a reverberant space.
pub struct CombFilter {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
    damp1: f32,
    damp2: f32,
    prev: f32,
}

impl CombFilter {
    /// Create a new comb filter with the given delay length in samples.
    pub fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
            feedback: 0.0,
            damp1: 0.0,
            damp2: 1.0,
            prev: 0.0,
        }
    }

    /// Set the damping coefficient. `damp1` controls how much of the
    /// previous filtered sample feeds back; `damp2 = 1 - damp1`.
    pub fn set_damp(&mut self, value: f32) {
        self.damp1 = value;
        self.damp2 = 1.0 - value;
    }

    /// Set the feedback gain.
    pub fn set_feedback(&mut self, value: f32) {
        self.feedback = value;
    }

    /// Process one sample through the comb filter.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.prev = output * self.damp2 + self.prev * self.damp1;
        self.buffer[self.index] = input + self.prev * self.feedback;
        self.index += 1;
        if self.index >= self.buffer.len() {
            self.index = 0;
        }
        output
    }

    /// Clear internal state.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.prev = 0.0;
        self.index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comb_initial_output_is_zero() {
        let mut comb = CombFilter::new(8);
        comb.set_feedback(0.8);
        // First 8 outputs should be zero (reading from zeroed buffer).
        for _ in 0..8 {
            let out = comb.process(1.0);
            assert_eq!(out, 0.0);
        }
    }

    #[test]
    fn test_comb_echoes_input() {
        let mut comb = CombFilter::new(4);
        comb.set_feedback(0.0);
        comb.set_damp(0.0); // no damping
        // Feed impulse at sample 0.
        let _ = comb.process(1.0);
        for _ in 1..4 {
            let _ = comb.process(0.0);
        }
        // After 4 samples, we should see the impulse come back.
        let out = comb.process(0.0);
        assert!((out - 1.0).abs() < 1e-6);
    }
}
