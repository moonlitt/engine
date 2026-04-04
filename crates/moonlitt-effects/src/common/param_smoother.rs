/// Exponential one-pole parameter smoother.
///
/// Smooths abrupt parameter changes over a configurable ramp time to
/// avoid audible zipper noise. The time constant equals `ramp_ms`;
/// after one time constant the value reaches ~63.2 % of the way to
/// the target.
pub struct ParamSmoother {
    current: f64,
    target: f64,
    coeff: f64,
    threshold: f64,
}

impl ParamSmoother {
    /// Create a new smoother starting at `initial`.
    ///
    /// * `sample_rate` — host sample rate in Hz.
    /// * `ramp_ms` — smoothing time constant in milliseconds.
    pub fn new(initial: f64, sample_rate: f64, ramp_ms: f64) -> Self {
        let samples = ramp_ms * 0.001 * sample_rate;
        let coeff = if samples > 0.0 {
            (-1.0 / samples).exp()
        } else {
            0.0
        };
        Self {
            current: initial,
            target: initial,
            coeff,
            threshold: 1e-8,
        }
    }

    /// Set a new target value. The smoother will ramp towards it.
    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    /// Advance one sample and return the smoothed value.
    #[inline]
    pub fn next(&mut self) -> f64 {
        if (self.current - self.target).abs() < self.threshold {
            self.current = self.target;
        } else {
            self.current = self.coeff * self.current + (1.0 - self.coeff) * self.target;
        }
        self.current
    }

    /// Peek at the current value without advancing.
    pub fn next_value(&self) -> f64 {
        self.current
    }

    /// Returns `true` when the smoother has converged to its target.
    pub fn is_settled(&self) -> bool {
        (self.current - self.target).abs() < self.threshold
    }

    /// Jump immediately to `value`, bypassing the ramp.
    pub fn reset(&mut self, value: f64) {
        self.current = value;
        self.target = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoother_reaches_target() {
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        for _ in 0..4410 {
            s.next();
        }
        assert!((s.next() - 1.0).abs() < 0.001);
    }

    #[test]
    fn smoother_starts_at_initial() {
        let s = ParamSmoother::new(5.0, 44100.0, 10.0);
        assert_eq!(s.next_value(), 5.0);
    }

    #[test]
    fn smoother_settled_when_at_target() {
        let s = ParamSmoother::new(1.0, 44100.0, 10.0);
        assert!(s.is_settled());
    }

    #[test]
    fn smoother_not_settled_after_target_change() {
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        assert!(!s.is_settled());
    }

    #[test]
    fn smoother_reset_jumps_immediately() {
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        s.reset(5.0);
        assert_eq!(s.next_value(), 5.0);
        assert!(s.is_settled());
    }

    #[test]
    fn smoother_ramp_timing() {
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        let samples_10ms = (44100.0 * 0.01) as usize;
        for _ in 0..samples_10ms {
            s.next();
        }
        let val = s.next();
        assert!(
            (val - 0.632).abs() < 0.05,
            "After 1 TC, value={val}, expected ~0.632"
        );
    }
}
