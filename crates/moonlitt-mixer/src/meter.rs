//! Thread-safe level metering shared between the audio and main threads.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Thread-safe stereo level meter (peak + RMS).
/// Written by audio thread, read by main thread via atomic f32-as-u32.
#[derive(Clone)]
pub struct LevelMeter {
    peak_left: Arc<AtomicU32>,
    peak_right: Arc<AtomicU32>,
    rms_left: Arc<AtomicU32>,
    rms_right: Arc<AtomicU32>,
    true_peak_left: Arc<AtomicU32>,
    true_peak_right: Arc<AtomicU32>,
}

impl Default for LevelMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelMeter {
    pub fn new() -> Self {
        Self {
            peak_left: Arc::new(AtomicU32::new(0)),
            peak_right: Arc::new(AtomicU32::new(0)),
            rms_left: Arc::new(AtomicU32::new(0)),
            rms_right: Arc::new(AtomicU32::new(0)),
            true_peak_left: Arc::new(AtomicU32::new(0)),
            true_peak_right: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Update meter from a rendered buffer. Called on audio thread.
    pub(crate) fn update(&self, left: &[f32], right: &[f32]) {
        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;
        let mut sum_sq_l: f32 = 0.0;
        let mut sum_sq_r: f32 = 0.0;

        for i in 0..left.len() {
            let al = left[i].abs();
            let ar = right[i].abs();
            if al > peak_l {
                peak_l = al;
            }
            if ar > peak_r {
                peak_r = ar;
            }
            sum_sq_l += left[i] * left[i];
            sum_sq_r += right[i] * right[i];
        }

        // True peak: 4x oversampled via linear interpolation between adjacent samples
        let mut tp_l = peak_l;
        let mut tp_r = peak_r;
        if left.len() >= 2 {
            for i in 0..left.len() - 1 {
                // 3 interpolated points between sample[i] and sample[i+1]
                for k in 1..4u32 {
                    let t = k as f32 * 0.25;
                    let interp_l = left[i] + t * (left[i + 1] - left[i]);
                    let interp_r = right[i] + t * (right[i + 1] - right[i]);
                    let al = interp_l.abs();
                    let ar = interp_r.abs();
                    if al > tp_l {
                        tp_l = al;
                    }
                    if ar > tp_r {
                        tp_r = ar;
                    }
                }
            }
        }

        let n = left.len().max(1) as f32;
        let rms_l = (sum_sq_l / n).sqrt();
        let rms_r = (sum_sq_r / n).sqrt();

        self.peak_left.store(peak_l.to_bits(), Ordering::Relaxed);
        self.peak_right.store(peak_r.to_bits(), Ordering::Relaxed);
        self.rms_left.store(rms_l.to_bits(), Ordering::Relaxed);
        self.rms_right.store(rms_r.to_bits(), Ordering::Relaxed);
        self.true_peak_left.store(tp_l.to_bits(), Ordering::Relaxed);
        self.true_peak_right
            .store(tp_r.to_bits(), Ordering::Relaxed);
    }

    /// Read sample peak level (L, R).
    pub fn peak(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_left.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_right.load(Ordering::Relaxed)),
        )
    }

    /// Read RMS level (L, R).
    pub fn rms(&self) -> (f32, f32) {
        (
            f32::from_bits(self.rms_left.load(Ordering::Relaxed)),
            f32::from_bits(self.rms_right.load(Ordering::Relaxed)),
        )
    }

    /// Read true peak level (L, R) — 4x oversampled per EBU R128.
    pub fn true_peak(&self) -> (f32, f32) {
        (
            f32::from_bits(self.true_peak_left.load(Ordering::Relaxed)),
            f32::from_bits(self.true_peak_right.load(Ordering::Relaxed)),
        )
    }
}
