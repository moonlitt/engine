//! Anomaly detection: NaN / Inf / denormals / DC offset / silent regions.
//!
//! These metrics catch real engine bugs that LUFS or peak alone won't.
//! Examples: a denormal explosion in a feedback path, a stuck DC offset
//! leaking from a filter, or unexpected dropouts mid-render.

use crate::report::Anomalies;

const SILENCE_WINDOW_MS: f64 = 100.0;
const SILENCE_THRESHOLD_DBFS: f64 = -60.0;

pub fn scan(left: &[f32], right: &[f32], sample_rate: u32) -> Anomalies {
    let frames = left.len().min(right.len());
    let mut a = Anomalies::default();

    let mut sum_l: f64 = 0.0;
    let mut sum_r: f64 = 0.0;

    for i in 0..frames {
        let l = left[i];
        let r = right[i];

        if l.is_nan() { a.nan_count += 1; }
        if r.is_nan() { a.nan_count += 1; }
        if l.is_infinite() { a.inf_count += 1; }
        if r.is_infinite() { a.inf_count += 1; }

        // Subnormal floats (denormals) — typically a DSP feedback bug.
        // f32::MIN_POSITIVE is the smallest normal value; anything smaller
        // and non-zero is denormal.
        if l != 0.0 && l.is_finite() && l.abs() < f32::MIN_POSITIVE {
            a.denormal_count += 1;
        }
        if r != 0.0 && r.is_finite() && r.abs() < f32::MIN_POSITIVE {
            a.denormal_count += 1;
        }

        if l.is_finite() { sum_l += l as f64; }
        if r.is_finite() { sum_r += r as f64; }
    }

    if frames > 0 {
        a.dc_offset_l = sum_l / frames as f64;
        a.dc_offset_r = sum_r / frames as f64;
    }

    a.silence_segments = detect_silence(left, right, sample_rate);

    a
}

/// Find regions where the per-window RMS (averaged across L/R) stays below
/// the threshold for at least one window's worth of time. Adjacent silent
/// windows are merged into a single segment.
fn detect_silence(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<(f64, f64)> {
    let frames = left.len().min(right.len());
    let win = ((SILENCE_WINDOW_MS / 1000.0) * sample_rate as f64).round() as usize;
    if win == 0 || frames < win {
        return Vec::new();
    }

    let threshold_linear = 10f64.powf(SILENCE_THRESHOLD_DBFS / 20.0);
    let mut segments: Vec<(f64, f64)> = Vec::new();
    let mut current_start: Option<usize> = None;

    let mut i = 0;
    while i + win <= frames {
        let mut sum_sq: f64 = 0.0;
        for k in 0..win {
            let l = left[i + k] as f64;
            let r = right[i + k] as f64;
            sum_sq += (l * l + r * r) * 0.5;
        }
        let rms = (sum_sq / win as f64).sqrt();
        let is_silent = rms < threshold_linear;

        match (is_silent, current_start) {
            (true, None) => current_start = Some(i),
            (false, Some(start)) => {
                segments.push((
                    start as f64 / sample_rate as f64,
                    (i + win) as f64 / sample_rate as f64,
                ));
                current_start = None;
            }
            _ => {}
        }
        i += win;
    }

    if let Some(start) = current_start {
        segments.push((
            start as f64 / sample_rate as f64,
            frames as f64 / sample_rate as f64,
        ));
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_silence_in_middle() {
        let sr = 44100;
        let mut left = vec![0.0f32; sr as usize * 3];
        let mut right = vec![0.0f32; sr as usize * 3];
        // Loud first second
        for i in 0..sr as usize {
            let s = (i as f32 * 0.001).sin() * 0.5;
            left[i] = s;
            right[i] = s;
        }
        // Silence 1-2s
        // Loud 2-3s
        for i in (sr as usize * 2)..(sr as usize * 3) {
            let s = (i as f32 * 0.001).sin() * 0.5;
            left[i] = s;
            right[i] = s;
        }
        let a = scan(&left, &right, sr);
        assert_eq!(a.silence_segments.len(), 1);
        let (start, end) = a.silence_segments[0];
        assert!(start > 0.9 && start < 1.2, "start={start}");
        assert!(end > 1.9 && end < 2.2, "end={end}");
    }

    #[test]
    fn flags_nan_and_inf() {
        let left = vec![0.0, f32::NAN, 0.5, f32::INFINITY, -0.3];
        let right = vec![0.0; 5];
        let a = scan(&left, &right, 44100);
        assert_eq!(a.nan_count, 1);
        assert_eq!(a.inf_count, 1);
    }
}
