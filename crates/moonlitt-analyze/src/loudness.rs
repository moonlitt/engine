//! EBU R128 loudness measurement via the `ebur128` crate.
//!
//! Reports integrated, short-term max, momentary max, and loudness range (LRA).
//! Uses K-weighting + gating per BS.1770-5.

use crate::report::LoudnessStats;
use ebur128::{EbuR128, Mode};

pub fn measure(left: &[f32], right: &[f32], sample_rate: u32) -> Result<LoudnessStats, ebur128::Error> {
    let frames = left.len().min(right.len());
    let mode = Mode::I | Mode::M | Mode::S | Mode::LRA | Mode::HISTOGRAM;
    let mut meter = EbuR128::new(2, sample_rate, mode)?;

    // ebur128 expects planar f32 channels. Feed a fixed window so we can
    // sample momentary/short-term maxima as we go.
    const WINDOW: usize = 4096;
    let mut momentary_max = f64::NEG_INFINITY;
    let mut short_term_max = f64::NEG_INFINITY;

    let mut i = 0;
    while i < frames {
        let end = (i + WINDOW).min(frames);
        meter.add_frames_planar_f32(&[&left[i..end], &right[i..end]])?;

        if let Ok(m) = meter.loudness_momentary() {
            if m.is_finite() && m > momentary_max { momentary_max = m; }
        }
        if let Ok(s) = meter.loudness_shortterm() {
            if s.is_finite() && s > short_term_max { short_term_max = s; }
        }

        i = end;
    }

    let integrated = meter.loudness_global().unwrap_or(f64::NEG_INFINITY);
    let lra = meter.loudness_range().unwrap_or(0.0);

    Ok(LoudnessStats {
        integrated_lufs: clean(integrated),
        short_term_max_lufs: clean(short_term_max),
        momentary_max_lufs: clean(momentary_max),
        lra_lu: lra,
    })
}

/// ebur128 returns `-inf` for silence. Floor at -200 LUFS so reports
/// round-trip through JSON cleanly (matches the dBFS floor used in `peak`).
const MIN_LUFS: f64 = -200.0;

fn clean(v: f64) -> f64 {
    if v.is_finite() { v.max(MIN_LUFS) } else { MIN_LUFS }
}
