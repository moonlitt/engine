//! Sinc interpolator with configurable quality (point count).
//!
//! Uses pre-computed windowed sinc tables for efficient lookup.
//! The table stores sinc(x) × window(x) for fractional positions at fixed resolution.

use crate::window;

/// Interpolation quality level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// Linear interpolation (2 points). Lowest CPU, noticeable aliasing.
    Linear,
    /// 4th-order sinc (8 points). Good for real-time, matches FluidSynth default.
    Sinc8,
    /// 16-point sinc. Good quality, matches sfizz's low-quality setting.
    Sinc16,
    /// 36-point sinc. High quality.
    Sinc36,
    /// 48-point sinc. Very high quality.
    Sinc48,
    /// 72-point sinc. Maximum quality, matches sfizz's highest setting.
    Sinc72,
}

impl Quality {
    /// Number of sinc kernel points (taps).
    pub fn num_points(self) -> usize {
        match self {
            Quality::Linear => 2,
            Quality::Sinc8 => 8,
            Quality::Sinc16 => 16,
            Quality::Sinc36 => 36,
            Quality::Sinc48 => 48,
            Quality::Sinc72 => 72,
        }
    }

    /// Half the kernel size (points on each side of center).
    pub fn half_len(self) -> usize {
        self.num_points() / 2
    }
}

/// Number of fractional steps in the sinc table.
/// Higher = more precision in the fractional position lookup.
/// 256 steps gives 1/256 fractional precision — more than enough for audio.
const TABLE_STEPS: usize = 256;

/// Pre-computed windowed sinc table.
/// Layout: `table[frac_index * num_points + tap]`
/// where frac_index = 0..TABLE_STEPS, tap = 0..num_points
pub struct SincInterpolator {
    table: Vec<f32>,
    num_points: usize,
    half_len: usize,
    quality: Quality,
}

impl SincInterpolator {
    /// Create a new interpolator with the given quality.
    /// Pre-computes the sinc table (one-time cost).
    pub fn new(quality: Quality) -> Self {
        if matches!(quality, Quality::Linear) {
            return Self {
                table: Vec::new(),
                num_points: 2,
                half_len: 1,
                quality,
            };
        }

        let num_points = quality.num_points();
        let half_len = quality.half_len();
        let beta = kaiser_beta(num_points);

        let mut table = vec![0.0f32; TABLE_STEPS * num_points];

        for frac_idx in 0..TABLE_STEPS {
            let frac = frac_idx as f64 / TABLE_STEPS as f64;

            for tap in 0..num_points {
                let n = tap as f64 - half_len as f64 + 1.0 - frac;
                let s = window::sinc(n);
                let w = window::kaiser(n, half_len as f64, beta);
                table[frac_idx * num_points + tap] = (s * w) as f32;
            }
        }

        Self {
            table,
            num_points,
            half_len,
            quality,
        }
    }

    /// Interpolate a sample at position `index + frac` in the sample buffer.
    ///
    /// - `samples`: the audio sample buffer
    /// - `index`: integer sample position
    /// - `frac`: fractional position (0.0 to 1.0)
    ///
    /// Returns the interpolated sample value.
    ///
    /// The caller must ensure `index >= half_len - 1` and
    /// `index + half_len < samples.len()` to avoid out-of-bounds access.
    pub fn interpolate(&self, samples: &[f32], index: usize, frac: f32) -> f32 {
        if self.quality == Quality::Linear {
            return Self::linear_interp(samples, index, frac);
        }

        let frac_idx = (frac * TABLE_STEPS as f32) as usize;
        let frac_idx = frac_idx.min(TABLE_STEPS - 1);

        let kernel = &self.table[frac_idx * self.num_points..][..self.num_points];
        let start = index + 1 - self.half_len;

        let mut sum = 0.0f32;
        for (i, &k) in kernel.iter().enumerate() {
            sum += samples[start + i] * k;
        }
        sum
    }

    /// Interpolate with bounds checking — clamps to edge samples if near boundary.
    pub fn interpolate_safe(&self, samples: &[f32], index: usize, frac: f32) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        if self.quality == Quality::Linear {
            return Self::linear_interp_safe(samples, index, frac);
        }

        let frac_idx = (frac * TABLE_STEPS as f32) as usize;
        let frac_idx = frac_idx.min(TABLE_STEPS - 1);
        let kernel = &self.table[frac_idx * self.num_points..][..self.num_points];

        let half = self.half_len;
        let mut sum = 0.0f32;
        for (i, &k) in kernel.iter().enumerate() {
            let sample_idx = (index + 1) as isize - half as isize + i as isize;
            let clamped = sample_idx.clamp(0, samples.len() as isize - 1) as usize;
            sum += samples[clamped] * k;
        }
        sum
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    pub fn num_points(&self) -> usize {
        self.num_points
    }

    fn linear_interp(samples: &[f32], index: usize, frac: f32) -> f32 {
        samples[index] * (1.0 - frac) + samples[index + 1] * frac
    }

    fn linear_interp_safe(samples: &[f32], index: usize, frac: f32) -> f32 {
        let a = if index < samples.len() { samples[index] } else { 0.0 };
        let b = if index + 1 < samples.len() { samples[index + 1] } else { a };
        a * (1.0 - frac) + b * frac
    }
}

/// Choose Kaiser beta based on kernel size.
/// Higher beta = narrower main lobe = better stopband rejection.
fn kaiser_beta(num_points: usize) -> f64 {
    match num_points {
        0..=8 => 5.0,
        9..=16 => 6.5,
        17..=36 => 7.5,
        37..=48 => 8.5,
        _ => 9.5, // 72+: maximum stopband rejection
    }
}
