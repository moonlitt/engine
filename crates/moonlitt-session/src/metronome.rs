//! Audio-thread metronome generator.
//!
//! Generates a short 1 kHz sine burst with exponential decay at every beat.
//! Mixed in *after* the main mixer so it never affects rendered audio
//! levels reported by track meters — only the master meter sees it.
//!
//! The struct is owned by the audio thread; the UI flips `enabled` via
//! a shared `Arc<AtomicBool>` so toggling is lock-free and real-time-safe.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const CLICK_FREQ_HZ: f32 = 1000.0;
const CLICK_AMPLITUDE: f32 = 0.25;
const CLICK_DURATION_MS: f32 = 20.0;
/// Below this BPM we treat the value as nonsense and skip clicking.
/// Stops a runaway loop if someone sets bpm=0.
const MIN_BPM: f64 = 1.0;

pub struct Metronome {
    enabled: Arc<AtomicBool>,
    sample_rate: u32,
    /// Cached beat length in samples — refreshed when `tempo` is read.
    samples_per_beat: f64,
    /// Floating-point phase counter to avoid integer truncation drift
    /// over long playback (1 ms over 10 minutes adds up otherwise).
    samples_since_beat: f64,
    /// Samples of the current click still to emit. 0 = not clicking.
    click_remaining: u32,
    click_total_samples: u32,
}

impl Metronome {
    pub fn new(sample_rate: u32) -> Self {
        let click_total = ((sample_rate as f32) * (CLICK_DURATION_MS / 1000.0)) as u32;
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            sample_rate,
            samples_per_beat: sample_rate as f64 * 60.0 / 120.0,
            samples_since_beat: 0.0,
            click_remaining: 0,
            click_total_samples: click_total.max(1),
        }
    }

    /// Shared toggle handle. The UI side stores this Arc and flips it
    /// from any thread.
    pub fn enabled_handle(&self) -> Arc<AtomicBool> {
        self.enabled.clone()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Mix metronome clicks into the master output buffers. Called once
    /// per audio chunk, after the main mixer has rendered, with the
    /// current tempo in BPM (so tempo automation in the MIDI file or a
    /// UI change picks up on the next chunk without lag).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], bpm: f64) {
        if !self.is_enabled() || bpm < MIN_BPM {
            // Reset phase so re-enabling fires cleanly on next beat.
            self.samples_since_beat = 0.0;
            self.click_remaining = 0;
            return;
        }
        self.samples_per_beat = self.sample_rate as f64 * 60.0 / bpm;

        let n = left.len().min(right.len());
        for i in 0..n {
            self.samples_since_beat += 1.0;
            if self.samples_since_beat >= self.samples_per_beat {
                self.samples_since_beat -= self.samples_per_beat;
                self.click_remaining = self.click_total_samples;
            }
            if self.click_remaining > 0 {
                let elapsed_samples = self.click_total_samples - self.click_remaining;
                let t = elapsed_samples as f32 / self.sample_rate as f32;
                let progress = elapsed_samples as f32 / self.click_total_samples as f32;
                // Exponential decay envelope — squared makes the tail
                // softer than linear, closer to a real click than a beep.
                let env = (1.0 - progress).powi(2);
                // cos (not sin) so the first sample of the click is at
                // peak amplitude — a real click has an abrupt onset, and
                // sin(0) = 0 would emit a silent first sample.
                let s = (std::f32::consts::TAU * CLICK_FREQ_HZ * t).cos() * env * CLICK_AMPLITUDE;
                left[i] += s;
                right[i] += s;
                self.click_remaining -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_metronome_is_silent() {
        let mut m = Metronome::new(48_000);
        let mut l = vec![0.0_f32; 256];
        let mut r = vec![0.0_f32; 256];
        m.process(&mut l, &mut r, 120.0);
        assert!(l.iter().all(|s| *s == 0.0));
        assert!(r.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn enabling_after_silence_does_not_immediately_click() {
        let mut m = Metronome::new(48_000);
        let mut l = vec![0.0_f32; 4];
        let mut r = vec![0.0_f32; 4];
        // Phase advances while disabled — but we reset it on the silent path.
        m.process(&mut l, &mut r, 120.0);
        m.enabled_handle().store(true, Ordering::Relaxed);
        m.process(&mut l, &mut r, 120.0);
        // Click_remaining only becomes non-zero when samples_since_beat
        // crosses samples_per_beat. After only 4 samples enabled, no click yet.
        assert_eq!(m.click_remaining, 0);
    }

    #[test]
    fn click_fires_at_beat_boundary() {
        // At 60 BPM and SR 48 kHz, exactly one click per second. Render
        // 1.1 s so the click crosses a full beat and we capture its
        // envelope, not just the single sample at the boundary.
        let sr = 48_000;
        let mut m = Metronome::new(sr);
        m.enabled_handle().store(true, Ordering::Relaxed);

        let chunk = (sr as f32 * 1.1) as usize;
        let mut l = vec![0.0_f32; chunk];
        let mut r = vec![0.0_f32; chunk];
        m.process(&mut l, &mut r, 60.0);

        let peak = l.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(
            peak > 0.05,
            "expected a click peak above noise floor; peak={peak}"
        );
        // Right channel should match L exactly — clicks are centered mono.
        assert_eq!(l, r);
    }

    #[test]
    fn click_count_matches_bpm() {
        // 120 BPM = 2 clicks/s; render 5 s and count zero-crossings of
        // the envelope (== beats), approximately.
        let sr = 48_000;
        let mut m = Metronome::new(sr);
        m.enabled_handle().store(true, Ordering::Relaxed);

        let seconds = 5.0_f32;
        let n = (sr as f32 * seconds) as usize;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        m.process(&mut l, &mut r, 120.0);

        // Count clicks by looking for rising edges after long silence —
        // the 1 kHz carrier inside each 20 ms click crosses zero many
        // times, so a naive "any sample > threshold" counts oscillations,
        // not clicks. Anything > 2 000 samples (≈ 42 ms) of consecutive
        // silence is well past one click but well under one beat at 120 BPM.
        let click_duration_samples = (sr as f32 * 0.020) as usize;
        let silence_threshold = click_duration_samples * 2;
        let mut clicks = 0;
        let mut silent_run = silence_threshold + 1;
        for s in &l {
            if s.abs() > 0.01 {
                if silent_run >= silence_threshold {
                    clicks += 1;
                }
                silent_run = 0;
            } else {
                silent_run += 1;
            }
        }
        // At 120 BPM over 5 s we expect ~10 beats. Allow ±1 for boundary effects.
        assert!(
            (9..=11).contains(&clicks),
            "expected ~10 clicks at 120 BPM over 5 s, got {clicks}"
        );
    }

    #[test]
    fn invalid_bpm_silences_output() {
        let mut m = Metronome::new(48_000);
        m.enabled_handle().store(true, Ordering::Relaxed);
        let mut l = vec![0.0_f32; 256];
        let mut r = vec![0.0_f32; 256];
        m.process(&mut l, &mut r, 0.0);
        assert!(l.iter().all(|s| *s == 0.0));
    }
}
