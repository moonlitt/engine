//! Peak / true peak / RMS measurement.
//!
//! - **Sample peak** is the largest absolute sample value.
//! - **True peak** estimates the post-DAC inter-sample peak via 4× linear
//!   interpolation. Conservative — a polyphase FIR would be more accurate but
//!   adds complexity not needed for regression testing.
//! - **RMS** is the root mean square over the full buffer.
//!
//! All values reported in dBFS / dBTP. Silence returns `f64::NEG_INFINITY`.

use crate::report::PeakStats;

const MIN_DBFS: f64 = -200.0;

/// Measure sample peak and true peak across a stereo buffer.
pub fn measure(left: &[f32], right: &[f32]) -> PeakStats {
    let (sp_l, tp_l) = measure_channel(left);
    let (sp_r, tp_r) = measure_channel(right);
    PeakStats {
        sample_peak_l_dbfs: linear_to_dbfs(sp_l),
        sample_peak_r_dbfs: linear_to_dbfs(sp_r),
        true_peak_l_dbtp: linear_to_dbfs(tp_l),
        true_peak_r_dbtp: linear_to_dbfs(tp_r),
    }
}

fn measure_channel(samples: &[f32]) -> (f32, f32) {
    let mut sp: f32 = 0.0;
    let mut tp: f32 = 0.0;
    for i in 0..samples.len() {
        let a = samples[i].abs();
        if a > sp { sp = a; }
        if a > tp { tp = a; }
        // 4× linear interpolation between this and next sample.
        if i + 1 < samples.len() {
            let next = samples[i + 1];
            for k in 1..4u32 {
                let t = k as f32 * 0.25;
                let interp = samples[i] + t * (next - samples[i]);
                let ia = interp.abs();
                if ia > tp { tp = ia; }
            }
        }
    }
    (sp, tp)
}

/// RMS in dBFS for a single channel.
pub fn rms_dbfs(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return f64::NEG_INFINITY;
    }
    let mut sum_sq: f64 = 0.0;
    for &s in samples {
        let s = s as f64;
        sum_sq += s * s;
    }
    let rms = (sum_sq / samples.len() as f64).sqrt();
    linear_to_dbfs(rms as f32)
}

fn linear_to_dbfs(linear: f32) -> f64 {
    if linear <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let db = 20.0 * (linear as f64).log10();
    if db < MIN_DBFS {
        f64::NEG_INFINITY
    } else {
        db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_sine_reports_zero_dbfs() {
        let len = 4410;
        let buf: Vec<f32> = (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44100.0).sin())
            .collect();
        let stats = measure(&buf, &buf);
        assert!(stats.sample_peak_l_dbfs > -0.1);
        assert!(stats.sample_peak_l_dbfs <= 0.0);
    }

    #[test]
    fn silence_reports_neg_infinity() {
        let buf = vec![0.0f32; 1000];
        let stats = measure(&buf, &buf);
        assert_eq!(stats.sample_peak_l_dbfs, f64::NEG_INFINITY);
        assert_eq!(rms_dbfs(&buf), f64::NEG_INFINITY);
    }
}
