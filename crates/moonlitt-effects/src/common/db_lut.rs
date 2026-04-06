//! Pre-computed dB to linear gain lookup table with linear interpolation.
//!
//! Replaces expensive `10.0_f64.powf(db / 20.0)` (~100 cycles) with a table
//! lookup + lerp (~5 cycles). 4096 entries over 144 dB range gives < 0.001 dB
//! error -- far below the 24-bit audio noise floor.
//!
//! The table uses f64 to match the f64 internal arithmetic of the dynamics
//! processors, preserving bit-exact passthrough when gain is 0 dB.

/// Pre-computed dB to linear gain lookup table.
pub struct DbLut {
    table: Vec<f64>,
    db_min: f64,
    db_max: f64,
    inv_step: f64,
}

impl DbLut {
    /// Create a new LUT covering -120 dB to +24 dB with 4096 entries.
    pub fn new() -> Self {
        let db_min = -120.0f64;
        let db_max = 24.0f64;
        let num_entries = 65536;
        let step = (db_max - db_min) / (num_entries - 1) as f64;
        let inv_step = 1.0 / step;
        let table: Vec<f64> = (0..num_entries)
            .map(|i| {
                let db = db_min + i as f64 * step;
                10.0_f64.powf(db / 20.0)
            })
            .collect();
        Self {
            table,
            db_min,
            db_max,
            inv_step,
        }
    }

    /// Convert dB to linear gain using table lookup with linear interpolation.
    ///
    /// Values outside [-120, +24] are clamped.
    /// Returns exactly 1.0 for 0.0 dB input (bit-exact passthrough).
    #[inline]
    pub fn db_to_linear(&self, db: f64) -> f64 {
        // Fast path: 0 dB = unity gain, must be bit-exact for passthrough tests
        if db == 0.0 {
            return 1.0;
        }
        let db = db.clamp(self.db_min, self.db_max);
        let pos = (db - self.db_min) * self.inv_step;
        let idx = pos as usize;
        let frac = pos - idx as f64;
        if idx + 1 < self.table.len() {
            self.table[idx] * (1.0 - frac) + self.table[idx + 1] * frac
        } else {
            self.table[self.table.len() - 1]
        }
    }

    /// Convert dB (f32) to linear gain (f32).
    ///
    /// Convenience wrapper for f32-only callers (e.g., benchmarks).
    #[inline]
    pub fn db_to_linear_f32(&self, db: f32) -> f32 {
        self.db_to_linear(db as f64) as f32
    }
}

impl Default for DbLut {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_lut_precision() {
        let lut = DbLut::new();
        // Test across the full range: LUT error should be < 0.01 dB
        for db_i in -1200..240 {
            let db = db_i as f64 / 10.0;
            let lut_val = lut.db_to_linear(db);
            let powf_val = 10.0_f64.powf(db / 20.0);

            // Convert both back to dB and compare
            if powf_val > 1e-10 && lut_val > 1e-10 {
                let lut_db = 20.0 * lut_val.log10();
                let powf_db = 20.0 * powf_val.log10();
                let error_db = (lut_db - powf_db).abs();
                assert!(
                    error_db < 0.01,
                    "LUT error {error_db:.6} dB at {db:.1} dB (lut={lut_val}, powf={powf_val})"
                );
            }
        }
    }

    #[test]
    fn db_lut_boundary() {
        let lut = DbLut::new();

        // -120 dB -> near zero
        let val = lut.db_to_linear(-120.0);
        assert!(val < 1e-5, "-120 dB should be near zero, got {val}");

        // 0 dB -> exactly 1.0
        let val = lut.db_to_linear(0.0);
        assert_eq!(val, 1.0, "0 dB should be exactly 1.0, got {val}");

        // +24 dB -> ~15.85
        let expected = 10.0_f64.powf(24.0 / 20.0);
        let val = lut.db_to_linear(24.0);
        assert!(
            (val - expected).abs() < 0.001,
            "+24 dB should be ~{expected}, got {val}"
        );
    }

    #[test]
    fn db_lut_f32_wrapper() {
        let lut = DbLut::new();
        let val = lut.db_to_linear_f32(-6.0);
        let expected = 10.0_f32.powf(-6.0 / 20.0);
        assert!(
            (val - expected).abs() < 0.001,
            "-6 dB f32: expected {expected}, got {val}"
        );
    }
}
