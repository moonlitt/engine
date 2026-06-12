//! Stereo delay with tempo sync, ping-pong, and filtered feedback.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. Uses `FractionalDelayLine` for sinc-interpolated
//! delay reads, and `Biquad` LP/HP filters in the feedback path.
//!
//! ## Algorithm
//!
//! ```text
//! input_L → write to delay_L → read at delay_time → [LP+HP filter] → × feedback → write back
//! input_R → write to delay_R → read at delay_time → [LP+HP filter] → × feedback → write back
//!
//! Ping-pong: cross-feed (L delayed → R feedback, R delayed → L feedback)
//! Output: dry × input + wet × delayed
//! ```
//!
//! When `sync_mode = 1`: `delay_ms = NoteValue::from_index(sync_note).to_ms(bpm)`

use std::f64::consts::PI;

use super::delay_line::FractionalDelayLine;
use super::lfo::NoteValue;
use crate::common::{flush_denormal, ParamSmoother};
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

/// Maximum delay time in milliseconds per channel.
const MAX_DELAY_MS: f64 = 5000.0;

/// Sinc kernel width (8-point Kaiser-windowed).
const SINC_POINTS: usize = 8;

/// Smoothing ramp time in milliseconds for parameter changes.
const SMOOTH_MS: f64 = 20.0;

// ---------------------------------------------------------------------------
// FeedbackFilter — lightweight biquad for LP/HP in the feedback path
// ---------------------------------------------------------------------------

/// A minimal 2nd-order IIR filter (Direct Form II Transposed) used
/// exclusively for the feedback path LP and HP filters.
///
/// Inlined here so the delay module doesn't depend on the `parametric-eq`
/// feature gate.
#[derive(Debug, Clone)]
struct FeedbackFilter {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl FeedbackFilter {
    /// Passthrough (identity) filter.
    fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Design a 2nd-order Butterworth lowpass filter.
    fn design_lowpass(sample_rate: f64, freq: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        let inv_a0 = 1.0 / a0;
        Self {
            b0: b0 * inv_a0,
            b1: b1 * inv_a0,
            b2: b2 * inv_a0,
            a1: a1 * inv_a0,
            a2: a2 * inv_a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Design a 2nd-order Butterworth highpass filter.
    fn design_highpass(sample_rate: f64, freq: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        let inv_a0 = 1.0 / a0;
        Self {
            b0: b0 * inv_a0,
            b1: b1 * inv_a0,
            b2: b2 * inv_a0,
            a1: a1 * inv_a0,
            a2: a2 * inv_a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Process a single sample (Direct Form II Transposed).
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Update coefficients from another filter, preserving internal state
    /// to avoid clicks during smooth parameter changes.
    fn update_coeffs(&mut self, other: &FeedbackFilter) {
        self.b0 = other.b0;
        self.b1 = other.b1;
        self.b2 = other.b2;
        self.a1 = other.a1;
        self.a2 = other.a2;
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ---------------------------------------------------------------------------
// NoteValue display helper
// ---------------------------------------------------------------------------

/// Return a human-readable label for a `NoteValue` index.
fn note_value_label(index: u32) -> &'static str {
    match index {
        0 => "1/32",
        1 => "1/16T",
        2 => "1/16",
        3 => "1/16.",
        4 => "1/8T",
        5 => "1/8",
        6 => "1/8.",
        7 => "1/4T",
        8 => "1/4",
        9 => "1/4.",
        10 => "1/2T",
        11 => "1/2",
        12 => "1/2.",
        13 => "1/1T",
        14 => "1/1",
        15 => "2 bars",
        _ => "4 bars",
    }
}

// ---------------------------------------------------------------------------
// StereoDelay
// ---------------------------------------------------------------------------

/// Stereo delay with tempo sync, ping-pong mode, and filtered feedback.
///
/// 12 parameters: time L/R, sync mode/notes, BPM, feedback, ping-pong,
/// filter LP/HP, dry/wet, bypass.
pub struct StereoDelay {
    sample_rate: u32,
    sr_f64: f64,

    // Parameters (raw values)
    time_left_ms: f64,
    time_right_ms: f64,
    sync_mode: bool,
    sync_note_left: u32,
    sync_note_right: u32,
    bpm: f64,
    feedback: f64,
    ping_pong: bool,
    filter_lp_freq: f64,
    filter_hp_freq: f64,
    dry_wet: f64,
    bypass: bool,

    // DSP state
    delay_l: FractionalDelayLine,
    delay_r: FractionalDelayLine,
    fb_lp: [FeedbackFilter; 2],
    fb_hp: [FeedbackFilter; 2],

    // Parameter smoothers
    smooth_time_l: ParamSmoother,
    smooth_time_r: ParamSmoother,
    smooth_feedback: ParamSmoother,
    smooth_lp: ParamSmoother,
    smooth_hp: ParamSmoother,
    smooth_dry_wet: ParamSmoother,

    // Feedback taps (persisted between process calls for ping-pong)
    fb_sample_l: f32,
    fb_sample_r: f32,
}

impl StereoDelay {
    /// Create a new stereo delay with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        let mut delay = Self {
            sample_rate,
            sr_f64: sr,

            time_left_ms: 500.0,
            time_right_ms: 500.0,
            sync_mode: false,
            sync_note_left: 8,
            sync_note_right: 8,
            bpm: 120.0,
            feedback: 0.3,
            ping_pong: false,
            filter_lp_freq: 8000.0,
            filter_hp_freq: 20.0,
            dry_wet: 0.3,
            bypass: false,

            delay_l: FractionalDelayLine::new(MAX_DELAY_MS, sample_rate, SINC_POINTS),
            delay_r: FractionalDelayLine::new(MAX_DELAY_MS, sample_rate, SINC_POINTS),
            fb_lp: [FeedbackFilter::new(), FeedbackFilter::new()],
            fb_hp: [FeedbackFilter::new(), FeedbackFilter::new()],

            smooth_time_l: ParamSmoother::new(500.0, sr, SMOOTH_MS),
            smooth_time_r: ParamSmoother::new(500.0, sr, SMOOTH_MS),
            smooth_feedback: ParamSmoother::new(0.3, sr, SMOOTH_MS),
            smooth_lp: ParamSmoother::new(8000.0, sr, SMOOTH_MS),
            smooth_hp: ParamSmoother::new(20.0, sr, SMOOTH_MS),
            smooth_dry_wet: ParamSmoother::new(0.3, sr, SMOOTH_MS),

            fb_sample_l: 0.0,
            fb_sample_r: 0.0,
        };

        delay.update_filters();
        delay
    }

    /// Compute the effective delay time in milliseconds for a channel,
    /// respecting sync mode.
    fn effective_delay_ms(&self, free_ms: f64, sync_note: u32) -> f64 {
        if self.sync_mode {
            NoteValue::from_index(sync_note).to_ms(self.bpm)
        } else {
            free_ms
        }
    }

    /// Recompute feedback filter coefficients.
    fn update_filters(&mut self) {
        let lp = FeedbackFilter::design_lowpass(self.sr_f64, self.filter_lp_freq);
        let hp = FeedbackFilter::design_highpass(self.sr_f64, self.filter_hp_freq);
        self.fb_lp[0] = lp.clone();
        self.fb_lp[1] = lp;
        self.fb_hp[0] = hp.clone();
        self.fb_hp[1] = hp;
    }

    /// Update smoother targets from current parameter values.
    fn sync_smoothers(&mut self) {
        let time_l = self.effective_delay_ms(self.time_left_ms, self.sync_note_left);
        let time_r = self.effective_delay_ms(self.time_right_ms, self.sync_note_right);
        self.smooth_time_l.set_target(time_l);
        self.smooth_time_r.set_target(time_r);
        self.smooth_feedback.set_target(self.feedback);
        self.smooth_lp.set_target(self.filter_lp_freq);
        self.smooth_hp.set_target(self.filter_hp_freq);
        self.smooth_dry_wet.set_target(self.dry_wet);
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for StereoDelay {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Stereo Delay",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
        self.fb_lp[0].reset();
        self.fb_lp[1].reset();
        self.fb_hp[0].reset();
        self.fb_hp[1].reset();
        self.fb_sample_l = 0.0;
        self.fb_sample_r = 0.0;
    }

    // -- MIDI: no-op for a delay effect --
    fn note_on(&mut self, _channel: u8, _note: u8, _velocity: u8) {}
    fn note_off(&mut self, _channel: u8, _note: u8) {}
    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: i16) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn all_notes_off(&mut self) {}

    // -- Audio: generator render is a no-op (this is an effect) --
    fn render(&mut self, _left: &mut [f32], _right: &mut [f32]) {}

    fn process_effect(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let len = in_l.len();

        // Bypass: bit-exact copy
        if self.bypass {
            out_l[..len].copy_from_slice(&in_l[..len]);
            out_r[..len].copy_from_slice(&in_r[..len]);
            return;
        }

        // Track whether filters need updating (per-block, not per-sample)
        let mut prev_lp = self.smooth_lp.next_value();
        let mut prev_hp = self.smooth_hp.next_value();

        for i in 0..len {
            // Advance smoothers
            let delay_ms_l = self.smooth_time_l.next();
            let delay_ms_r = self.smooth_time_r.next();
            let feedback = self.smooth_feedback.next() as f32;
            let dry_wet = self.smooth_dry_wet.next() as f32;
            let current_lp = self.smooth_lp.next();
            let current_hp = self.smooth_hp.next();

            // Update filter coefficients when frequency changes significantly
            if (current_lp - prev_lp).abs() > 0.1 || (current_hp - prev_hp).abs() > 0.01 {
                let lp_filt = FeedbackFilter::design_lowpass(self.sr_f64, current_lp);
                let hp_filt = FeedbackFilter::design_highpass(self.sr_f64, current_hp);
                // Preserve filter state (z1, z2) to avoid clicks
                self.fb_lp[0].update_coeffs(&lp_filt);
                self.fb_lp[1].update_coeffs(&lp_filt);
                self.fb_hp[0].update_coeffs(&hp_filt);
                self.fb_hp[1].update_coeffs(&hp_filt);
                prev_lp = current_lp;
                prev_hp = current_hp;
            }

            // Convert delay time from ms to samples
            let delay_samples_l = delay_ms_l * 0.001 * self.sr_f64;
            let delay_samples_r = delay_ms_r * 0.001 * self.sr_f64;

            // Write input + feedback into delay lines
            let write_l = in_l[i] + flush_denormal(self.fb_sample_l);
            let write_r = in_r[i] + flush_denormal(self.fb_sample_r);
            self.delay_l.write(write_l);
            self.delay_r.write(write_r);

            // Read delayed output
            let delayed_l = self.delay_l.read(delay_samples_l);
            let delayed_r = self.delay_r.read(delay_samples_r);

            // Filter the delayed signal (feedback path)
            let filtered_l = flush_denormal(
                self.fb_hp[0].process(self.fb_lp[0].process(delayed_l as f64)) as f32,
            );
            let filtered_r = flush_denormal(
                self.fb_hp[1].process(self.fb_lp[1].process(delayed_r as f64)) as f32,
            );

            // Compute feedback samples (with optional ping-pong cross-feed)
            if self.ping_pong {
                // Cross-feed: L delayed → R feedback, R delayed → L feedback
                self.fb_sample_l = filtered_r * feedback;
                self.fb_sample_r = filtered_l * feedback;
            } else {
                self.fb_sample_l = filtered_l * feedback;
                self.fb_sample_r = filtered_r * feedback;
            }

            // Mix dry/wet
            let dry = 1.0 - dry_wet;
            out_l[i] = in_l[i] * dry + delayed_l * dry_wet;
            out_r[i] = in_r[i] * dry + delayed_r * dry_wet;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Delay does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: time_left_ms    (1..5000)
    // 1: time_right_ms   (1..5000)
    // 2: sync_mode       (0/1)
    // 3: sync_note_left  (0..16, stepped)
    // 4: sync_note_right (0..16, stepped)
    // 5: bpm             (20..300)
    // 6: feedback        (0..0.95)
    // 7: ping_pong       (0/1)
    // 8: filter_lp       (200..20000)
    // 9: filter_hp       (20..2000)
    // 10: dry_wet        (0..1)
    // 11: bypass         (0/1)

    fn param_count(&self) -> u32 {
        12
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Time L".into(),
                group: "Delay".into(),
                min: 1.0,
                max: 5000.0,
                default: 500.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Time R".into(),
                group: "Delay".into(),
                min: 1.0,
                max: 5000.0,
                default: 500.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Sync Mode".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Sync Note L".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 16.0,
                default: 8.0,
                step_count: 16,
                flags: ParamFlags::STEPPED,
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Sync Note R".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 16.0,
                default: 8.0,
                step_count: 16,
                flags: ParamFlags::STEPPED,
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "BPM".into(),
                group: "Sync".into(),
                min: 20.0,
                max: 300.0,
                default: 120.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Feedback".into(),
                group: "Delay".into(),
                min: 0.0,
                max: 0.95,
                default: 0.3,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            7 => Some(ParamInfo {
                id: 7,
                name: "Ping-Pong".into(),
                group: "Delay".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            8 => Some(ParamInfo {
                id: 8,
                name: "Filter LP".into(),
                group: "Filter".into(),
                min: 200.0,
                max: 20000.0,
                default: 8000.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            9 => Some(ParamInfo {
                id: 9,
                name: "Filter HP".into(),
                group: "Filter".into(),
                min: 20.0,
                max: 2000.0,
                default: 20.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            10 => Some(ParamInfo {
                id: 10,
                name: "Dry/Wet".into(),
                group: "Mix".into(),
                min: 0.0,
                max: 1.0,
                default: 0.3,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            11 => Some(ParamInfo {
                id: 11,
                name: "Bypass".into(),
                group: "Global".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            _ => None,
        }
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            0 => Some(self.time_left_ms),
            1 => Some(self.time_right_ms),
            2 => Some(if self.sync_mode { 1.0 } else { 0.0 }),
            3 => Some(self.sync_note_left as f64),
            4 => Some(self.sync_note_right as f64),
            5 => Some(self.bpm),
            6 => Some(self.feedback),
            7 => Some(if self.ping_pong { 1.0 } else { 0.0 }),
            8 => Some(self.filter_lp_freq),
            9 => Some(self.filter_hp_freq),
            10 => Some(self.dry_wet),
            11 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.time_left_ms = value.clamp(1.0, 5000.0);
                self.sync_smoothers();
            }
            1 => {
                self.time_right_ms = value.clamp(1.0, 5000.0);
                self.sync_smoothers();
            }
            2 => {
                self.sync_mode = value >= 0.5;
                self.sync_smoothers();
            }
            3 => {
                self.sync_note_left = (value.round() as u32).min(16);
                self.sync_smoothers();
            }
            4 => {
                self.sync_note_right = (value.round() as u32).min(16);
                self.sync_smoothers();
            }
            5 => {
                self.bpm = value.clamp(20.0, 300.0);
                self.sync_smoothers();
            }
            6 => {
                self.feedback = value.clamp(0.0, 0.95);
                self.smooth_feedback.set_target(self.feedback);
            }
            7 => {
                self.ping_pong = value >= 0.5;
            }
            8 => {
                self.filter_lp_freq = value.clamp(200.0, 20000.0);
                self.smooth_lp.set_target(self.filter_lp_freq);
                self.update_filters();
            }
            9 => {
                self.filter_hp_freq = value.clamp(20.0, 2000.0);
                self.smooth_hp.set_target(self.filter_hp_freq);
                self.update_filters();
            }
            10 => {
                self.dry_wet = value.clamp(0.0, 1.0);
                self.smooth_dry_wet.set_target(self.dry_wet);
            }
            11 => {
                self.bypass = value >= 0.5;
            }
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.0} ms", value)),
            1 => Some(format!("{:.0} ms", value)),
            2 => Some(if value >= 0.5 {
                "Sync".into()
            } else {
                "Free".into()
            }),
            3 => Some(note_value_label(value.round() as u32).into()),
            4 => Some(note_value_label(value.round() as u32).into()),
            5 => Some(format!("{:.1} BPM", value)),
            6 => Some(format!("{:.0}%", value * 100.0)),
            7 => Some(if value >= 0.5 {
                "On".into()
            } else {
                "Off".into()
            }),
            8 => Some(format!("{:.0} Hz", value)),
            9 => Some(format!("{:.0} Hz", value)),
            10 => Some(format!("{:.0}%", value * 100.0)),
            11 => Some(if value >= 0.5 {
                "On".into()
            } else {
                "Off".into()
            }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_is_bitexact() {
        let mut delay = StereoDelay::new(44100);
        delay.set_param(11, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        delay.process_effect(&input, &silent, &mut out_l, &mut out_r);

        for i in 0..512 {
            assert_eq!(
                out_l[i].to_bits(),
                input[i].to_bits(),
                "bypass left sample {} not bit-exact",
                i
            );
            assert_eq!(
                out_r[i].to_bits(),
                silent[i].to_bits(),
                "bypass right sample {} not bit-exact",
                i
            );
        }
    }

    #[test]
    fn test_param_round_trip() {
        let mut delay = StereoDelay::new(44100);

        // time_left_ms
        delay.set_param(0, 250.0);
        assert_eq!(delay.get_param(0), Some(250.0));

        // time_right_ms
        delay.set_param(1, 750.0);
        assert_eq!(delay.get_param(1), Some(750.0));

        // sync_mode
        delay.set_param(2, 1.0);
        assert_eq!(delay.get_param(2), Some(1.0));

        // sync_note_left
        delay.set_param(3, 5.0);
        assert_eq!(delay.get_param(3), Some(5.0));

        // sync_note_right
        delay.set_param(4, 11.0);
        assert_eq!(delay.get_param(4), Some(11.0));

        // bpm
        delay.set_param(5, 140.0);
        assert_eq!(delay.get_param(5), Some(140.0));

        // feedback
        delay.set_param(6, 0.6);
        assert_eq!(delay.get_param(6), Some(0.6));

        // ping_pong
        delay.set_param(7, 1.0);
        assert_eq!(delay.get_param(7), Some(1.0));

        // filter_lp
        delay.set_param(8, 5000.0);
        assert_eq!(delay.get_param(8), Some(5000.0));

        // filter_hp
        delay.set_param(9, 100.0);
        assert_eq!(delay.get_param(9), Some(100.0));

        // dry_wet
        delay.set_param(10, 0.7);
        assert_eq!(delay.get_param(10), Some(0.7));

        // bypass
        delay.set_param(11, 1.0);
        assert_eq!(delay.get_param(11), Some(1.0));

        // Clamping
        delay.set_param(0, 0.0); // below min (1.0)
        assert_eq!(delay.get_param(0), Some(1.0));

        delay.set_param(6, 2.0); // above max (0.95)
        assert_eq!(delay.get_param(6), Some(0.95));

        // Invalid param
        assert_eq!(delay.get_param(99), None);
        assert!(delay.param_info(12).is_none());

        // Param count
        assert_eq!(delay.param_count(), 12);
    }

    /// Process a block of silence to warm up smoothers and let them converge.
    fn warm_up(delay: &mut StereoDelay, num_samples: usize) {
        let silence = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        delay.process_effect(&silence, &silence, &mut out_l, &mut out_r);
        // Clear delay lines after warmup so no residual signal remains
        delay.delay_l.clear();
        delay.delay_r.clear();
        delay.fb_sample_l = 0.0;
        delay.fb_sample_r = 0.0;
    }

    #[test]
    fn test_delay_time_correct() {
        let sr = 44100u32;
        let delay_ms = 10.0; // 10ms = 441 samples

        let mut delay = StereoDelay::new(sr);
        delay.set_param(0, delay_ms); // time_left = 10ms
        delay.set_param(1, delay_ms); // time_right = 10ms
        delay.set_param(6, 0.0); // feedback = 0 (no repeats)
        delay.set_param(10, 1.0); // wet = 100%

        // Warm up to let smoothers converge (1s = 50 time constants)
        warm_up(&mut delay, 44100);

        let delay_samples = (delay_ms * 0.001 * sr as f64).round() as usize;
        let total = delay_samples + 100; // extra tail

        // Create impulse at sample 0
        let mut input_l = vec![0.0f32; total];
        let input_r = vec![0.0f32; total];
        input_l[0] = 1.0;

        let mut out_l = vec![0.0f32; total];
        let mut out_r = vec![0.0f32; total];

        delay.process_effect(&input_l, &input_r, &mut out_l, &mut out_r);

        // Find peak position in output
        let peak_pos = out_l
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        // Allow 1 sample tolerance for sinc interpolation
        let diff = (peak_pos as i64 - delay_samples as i64).unsigned_abs();
        assert!(
            diff <= 1,
            "impulse should appear at sample {}, found peak at {} (diff={})",
            delay_samples,
            peak_pos,
            diff
        );
    }

    #[test]
    fn test_feedback_decays() {
        let sr = 44100u32;
        let delay_ms = 20.0; // 20ms = 882 samples
        let feedback_val = 0.5;

        let mut delay = StereoDelay::new(sr);
        delay.set_param(0, delay_ms);
        delay.set_param(1, delay_ms);
        delay.set_param(6, feedback_val);
        delay.set_param(8, 20000.0); // LP at max (near passthrough)
        delay.set_param(9, 20.0); // HP at min (near passthrough)
        delay.set_param(10, 1.0); // wet = 100%

        // Warm up to let smoothers converge (1s = 50 time constants)
        warm_up(&mut delay, 44100);

        let delay_samples = (delay_ms * 0.001 * sr as f64).round() as usize;
        let total = delay_samples * 5; // enough for 4 repeats

        let mut input_l = vec![0.0f32; total];
        let input_r = vec![0.0f32; total];
        input_l[0] = 1.0;

        let mut out_l = vec![0.0f32; total];
        let mut out_r = vec![0.0f32; total];

        delay.process_effect(&input_l, &input_r, &mut out_l, &mut out_r);

        // Measure peak amplitude at each repeat
        let mut peaks = Vec::new();
        for rep in 1..=3 {
            let center = delay_samples * rep;
            let window_start = center.saturating_sub(2);
            let window_end = (center + 3).min(total);
            let peak = out_l[window_start..window_end]
                .iter()
                .map(|s| s.abs())
                .fold(0.0f32, f32::max);
            peaks.push(peak);
        }

        // Each repeat should be roughly feedback x previous
        for i in 1..peaks.len() {
            let ratio = peaks[i] / peaks[i - 1];
            assert!(
                (ratio - feedback_val as f32).abs() < 0.15,
                "repeat {}: ratio {:.3} should be ~{:.1} (peaks: {:.4}, {:.4})",
                i + 1,
                ratio,
                feedback_val,
                peaks[i - 1],
                peaks[i]
            );
        }
    }

    #[test]
    fn test_ping_pong_alternates() {
        let sr = 44100u32;
        let delay_ms = 20.0; // 20ms = 882 samples

        let mut delay = StereoDelay::new(sr);
        delay.set_param(0, delay_ms);
        delay.set_param(1, delay_ms);
        delay.set_param(6, 0.9); // high feedback
        delay.set_param(7, 1.0); // ping-pong on
        delay.set_param(8, 20000.0); // LP wide open
        delay.set_param(9, 20.0); // HP at minimum
        delay.set_param(10, 1.0); // wet = 100%

        // Warm up to let smoothers converge (1s = 50 time constants)
        warm_up(&mut delay, 44100);

        let delay_samples = (delay_ms * 0.001 * sr as f64).round() as usize;
        let total = delay_samples * 5;

        // Impulse in L only
        let mut input_l = vec![0.0f32; total];
        let input_r = vec![0.0f32; total];
        input_l[0] = 1.0;

        let mut out_l = vec![0.0f32; total];
        let mut out_r = vec![0.0f32; total];

        delay.process_effect(&input_l, &input_r, &mut out_l, &mut out_r);

        // First repeat: impulse was in L, so delayed output appears in L
        // Second repeat (cross-fed): should appear in R
        let first_repeat_center = delay_samples;
        let second_repeat_center = delay_samples * 2;

        // Measure peaks around each repeat
        let peak_l_1 = out_l
            [first_repeat_center.saturating_sub(2)..(first_repeat_center + 3).min(total)]
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        let peak_r_1 = out_r
            [first_repeat_center.saturating_sub(2)..(first_repeat_center + 3).min(total)]
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);

        let peak_r_2 = out_r
            [second_repeat_center.saturating_sub(2)..(second_repeat_center + 3).min(total)]
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);

        // First repeat: L should have signal, R should be near zero
        assert!(
            peak_l_1 > 0.1,
            "first repeat: L should have signal, got {:.4}",
            peak_l_1
        );
        assert!(
            peak_r_1 < 0.01,
            "first repeat: R should be quiet, got {:.4}",
            peak_r_1
        );

        // Second repeat: R should have signal from ping-pong cross-feed
        assert!(
            peak_r_2 > 0.05,
            "second repeat: R should have ping-pong signal, got {:.4}",
            peak_r_2
        );
    }

    #[test]
    fn test_param_info_complete() {
        let delay = StereoDelay::new(44100);

        assert_eq!(delay.param_count(), 12);

        for i in 0..12 {
            let info = delay.param_info(i);
            assert!(info.is_some(), "param_info({}) should return Some", i);
            let info = info.unwrap();
            assert_eq!(info.id, i);
            assert!(
                !info.name.is_empty(),
                "param {} name should not be empty",
                i
            );
            assert!(
                !info.group.is_empty(),
                "param {} group should not be empty",
                i
            );
            assert!(info.min <= info.default, "param {} min <= default", i);
            assert!(info.default <= info.max, "param {} default <= max", i);
        }

        // No 13th parameter
        assert!(delay.param_info(12).is_none());
    }
}
