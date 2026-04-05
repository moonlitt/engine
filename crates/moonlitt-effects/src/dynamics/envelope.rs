//! Exponential envelope follower for level detection.
//!
//! Uses separate attack and release coefficients for smooth
//! gain-reduction tracking. All arithmetic in f64.

/// Exponential envelope follower with independent attack/release.
#[derive(Clone)]
pub struct EnvelopeFollower {
    sample_rate: f64,
    attack_coeff: f64,
    release_coeff: f64,
    level: f64,
}

impl EnvelopeFollower {
    /// Create a new envelope follower with default 10ms attack, 100ms release.
    pub fn new(sample_rate: f64) -> Self {
        let mut ef = Self {
            sample_rate,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            level: 0.0,
        };
        ef.set_attack_ms(10.0);
        ef.set_release_ms(100.0);
        ef
    }

    /// Set attack time in milliseconds.
    ///
    /// Coefficient: `exp(-1 / (ms * 0.001 * sample_rate))`
    pub fn set_attack_ms(&mut self, ms: f64) {
        let samples = ms * 0.001 * self.sample_rate;
        self.attack_coeff = if samples > 0.0 {
            (-1.0 / samples).exp()
        } else {
            0.0
        };
    }

    /// Set release time in milliseconds.
    ///
    /// Coefficient: `exp(-1 / (ms * 0.001 * sample_rate))`
    pub fn set_release_ms(&mut self, ms: f64) {
        let samples = ms * 0.001 * self.sample_rate;
        self.release_coeff = if samples > 0.0 {
            (-1.0 / samples).exp()
        } else {
            0.0
        };
    }

    /// Process a single input level and return the smoothed envelope level.
    ///
    /// Uses attack coefficient when input exceeds current level (onset),
    /// release coefficient when input falls below (decay).
    #[inline]
    pub fn process(&mut self, input_level: f64) -> f64 {
        if input_level > self.level {
            self.level = self.attack_coeff * self.level + (1.0 - self.attack_coeff) * input_level;
        } else {
            self.level =
                self.release_coeff * self.level + (1.0 - self.release_coeff) * input_level;
        }
        self.level
    }

    /// Reset the envelope to zero.
    pub fn reset(&mut self) {
        self.level = 0.0;
    }

    /// Current envelope level.
    pub fn level(&self) -> f64 {
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attack_from_zero() {
        let sr = 44100.0;
        let mut env = EnvelopeFollower::new(sr);
        env.set_attack_ms(10.0);
        env.set_release_ms(100.0);

        // Feed constant level 1.0 and check convergence
        let mut level = 0.0;
        for _ in 0..44100 {
            level = env.process(1.0);
        }
        // After 1 second, should be very close to 1.0
        assert!(
            (level - 1.0).abs() < 1e-10,
            "envelope should converge to 1.0, got {}",
            level
        );
    }

    #[test]
    fn test_release_from_peak() {
        let sr = 44100.0;
        let mut env = EnvelopeFollower::new(sr);
        env.set_attack_ms(0.1);
        env.set_release_ms(100.0);

        // Quickly ramp up
        for _ in 0..4410 {
            env.process(1.0);
        }
        assert!((env.level() - 1.0).abs() < 1e-6);

        // Release: feed 0.0 for 3 seconds (30 time constants at 100ms)
        for _ in 0..(44100 * 3) {
            env.process(0.0);
        }
        assert!(
            env.level() < 1e-4,
            "envelope should decay to ~0, got {}",
            env.level()
        );
    }

    #[test]
    fn test_time_constant() {
        let sr = 44100.0;
        let mut env = EnvelopeFollower::new(sr);
        env.set_attack_ms(10.0);

        // One time constant = 10ms = 441 samples
        // After 1 time constant, should reach ~63.2% of target
        let attack_samples = (10.0 * 0.001 * sr) as usize;
        for _ in 0..attack_samples {
            env.process(1.0);
        }

        let expected = 1.0 - (-1.0_f64).exp(); // 1 - e^(-1) ≈ 0.6321
        let error = (env.level() - expected).abs();
        assert!(
            error < 0.01,
            "after 1 time constant, level should be ~{:.4}, got {:.4}",
            expected,
            env.level()
        );
    }
}
