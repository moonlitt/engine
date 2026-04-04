//! N-stage allpass phaser with LFO-modulated frequency sweep.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. A cascade of first-order allpass filters with
//! exponentially swept cutoff frequencies creates moving notches in the
//! frequency response. When summed with the dry signal, this produces the
//! classic phaser comb-filter sweep.
//!
//! ## Algorithm
//!
//! ```text
//! For each channel (L has phase_offset=0, R has phase_offset=stereo_phase/360):
//!     lfo_val = lfo.next(rate)                        // -1..1
//!     lfo_uni = lfo_val * 0.5 + 0.5                   // 0..1
//!     freq = min_freq * (max_freq / min_freq).powf(lfo_uni * depth)
//!
//!     signal = input + feedback_buf * feedback
//!     for stage in 0..active_stages:
//!         signal = allpass[stage].process(signal, freq)
//!
//!     feedback_buf = flush_denormal(signal)
//!     output = input + signal                          // additive mix
//! ```

use std::f64::consts::PI;

use super::lfo::{Lfo, NoteValue};
use crate::common::{flush_denormal_f64, ParamSmoother};
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

/// Maximum number of allpass stages per channel.
const MAX_STAGES: usize = 12;

/// Smoothing ramp time in milliseconds for parameter changes.
const SMOOTH_MS: f64 = 5.0;

// ---------------------------------------------------------------------------
// First-order allpass stage
// ---------------------------------------------------------------------------

/// A single first-order allpass filter stage.
///
/// Transfer function: H(z) = (-a + z^-1) / (1 - a * z^-1)
/// where a = (tan(pi * f / sr) - 1) / (tan(pi * f / sr) + 1).
#[derive(Debug, Clone)]
struct AllpassStage {
    /// Single delay element.
    z: f64,
}

impl AllpassStage {
    fn new() -> Self {
        Self { z: 0.0 }
    }

    /// Process one sample through the allpass at the given centre frequency.
    #[inline]
    fn process(&mut self, x: f64, freq: f64, sample_rate: f64) -> f64 {
        let tan_val = (PI * freq / sample_rate).tan();
        let a = (tan_val - 1.0) / (tan_val + 1.0);
        let y = -a * x + self.z;
        self.z = a * y + x;
        y
    }

    fn reset(&mut self) {
        self.z = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Sync mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    Free,
    Sync,
}

// ---------------------------------------------------------------------------
// Phaser
// ---------------------------------------------------------------------------

/// N-stage allpass phaser with LFO-driven exponential frequency sweep.
///
/// The allpass cascade creates phase shifts that, when summed with the
/// original signal, produce a series of notches in the frequency response.
/// The LFO sweeps these notches up and down the spectrum. Feedback
/// deepens the notches for a more pronounced effect.
pub struct Phaser {
    sample_rate: u32,

    // Parameters
    rate_hz: f64,
    depth: f64,
    stages: u32,
    feedback: f64,
    min_freq: f64,
    max_freq: f64,
    stereo_phase: f64,
    sync_mode: SyncMode,
    sync_note: u32,
    bpm: f64,
    bypass: bool,

    // Per-channel state
    stages_l: Vec<AllpassStage>,
    stages_r: Vec<AllpassStage>,
    lfo_l: Lfo,
    lfo_r: Lfo,
    feedback_buf_l: f64,
    feedback_buf_r: f64,

    // Smoothers
    rate_smoother: ParamSmoother,
    depth_smoother: ParamSmoother,
    feedback_smoother: ParamSmoother,
    min_freq_smoother: ParamSmoother,
    max_freq_smoother: ParamSmoother,
}

impl Phaser {
    /// Create a new phaser with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        let mut lfo_r = Lfo::new(sample_rate);
        lfo_r.set_phase(90.0 / 360.0); // default stereo_phase = 90 degrees

        let stages_l: Vec<AllpassStage> = (0..MAX_STAGES).map(|_| AllpassStage::new()).collect();
        let stages_r: Vec<AllpassStage> = (0..MAX_STAGES).map(|_| AllpassStage::new()).collect();

        Self {
            sample_rate,

            rate_hz: 0.4,
            depth: 0.6,
            stages: 4,
            feedback: 0.3,
            min_freq: 100.0,
            max_freq: 5000.0,
            stereo_phase: 90.0,
            sync_mode: SyncMode::Free,
            sync_note: 8, // Quarter
            bpm: 120.0,
            bypass: false,

            stages_l,
            stages_r,
            lfo_l: Lfo::new(sample_rate),
            lfo_r,
            feedback_buf_l: 0.0,
            feedback_buf_r: 0.0,

            rate_smoother: ParamSmoother::new(0.4, sr, SMOOTH_MS),
            depth_smoother: ParamSmoother::new(0.6, sr, SMOOTH_MS),
            feedback_smoother: ParamSmoother::new(0.3, sr, SMOOTH_MS),
            min_freq_smoother: ParamSmoother::new(100.0, sr, SMOOTH_MS),
            max_freq_smoother: ParamSmoother::new(5000.0, sr, SMOOTH_MS),
        }
    }

    /// Compute the LFO frequency for the current sample, considering sync mode.
    #[inline]
    fn lfo_freq(&self, rate: f64) -> f64 {
        match self.sync_mode {
            SyncMode::Free => rate,
            SyncMode::Sync => NoteValue::from_index(self.sync_note).to_hz(self.bpm),
        }
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Phaser {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Phaser",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        for stage in &mut self.stages_l {
            stage.reset();
        }
        for stage in &mut self.stages_r {
            stage.reset();
        }
        self.lfo_l.reset_phase();
        self.lfo_r.reset_phase();
        self.lfo_r.set_phase(self.stereo_phase / 360.0);
        self.feedback_buf_l = 0.0;
        self.feedback_buf_r = 0.0;
        self.rate_smoother.reset(self.rate_hz);
        self.depth_smoother.reset(self.depth);
        self.feedback_smoother.reset(self.feedback);
        self.min_freq_smoother.reset(self.min_freq);
        self.max_freq_smoother.reset(self.max_freq);
    }

    // -- MIDI: no-op for a phaser effect --
    fn note_on(&mut self, _channel: u8, _note: u8, _velocity: u8) {}
    fn note_off(&mut self, _channel: u8, _note: u8) {}
    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: i16) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn all_notes_off(&mut self) {}

    // -- Audio: generator render is a no-op (this is an effect) --
    fn render(&mut self, _left: &mut [f32], _right: &mut [f32]) {}

    fn process_effect(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        let len = in_l.len();

        // Bypass: bit-exact copy
        if self.bypass {
            out_l[..len].copy_from_slice(&in_l[..len]);
            out_r[..len].copy_from_slice(&in_r[..len]);
            return;
        }

        let sr = self.sample_rate as f64;
        let active_stages = self.stages as usize;

        for n in 0..len {
            let rate = self.rate_smoother.next();
            let depth = self.depth_smoother.next();
            let feedback = self.feedback_smoother.next();
            let min_f = self.min_freq_smoother.next();
            let max_f = self.max_freq_smoother.next();

            let freq = self.lfo_freq(rate);

            // Ensure max_freq > min_freq for the exponential mapping
            let ratio = (max_f / min_f).max(1.0);

            // --- Left channel ---
            let lfo_val_l = self.lfo_l.next(freq);
            let lfo_uni_l = lfo_val_l * 0.5 + 0.5; // map to 0..1
            let sweep_freq_l = min_f * ratio.powf(lfo_uni_l * depth);

            let mut signal_l = in_l[n] as f64 + self.feedback_buf_l * feedback;
            for stage in self.stages_l[..active_stages].iter_mut() {
                signal_l = stage.process(signal_l, sweep_freq_l, sr);
            }
            self.feedback_buf_l = flush_denormal_f64(signal_l);

            // --- Right channel ---
            let lfo_val_r = self.lfo_r.next(freq);
            let lfo_uni_r = lfo_val_r * 0.5 + 0.5;
            let sweep_freq_r = min_f * ratio.powf(lfo_uni_r * depth);

            let mut signal_r = in_r[n] as f64 + self.feedback_buf_r * feedback;
            for stage in self.stages_r[..active_stages].iter_mut() {
                signal_r = stage.process(signal_r, sweep_freq_r, sr);
            }
            self.feedback_buf_r = flush_denormal_f64(signal_r);

            // Phaser uses additive mix: output = input + wet
            out_l[n] = (in_l[n] as f64 + signal_l) as f32;
            out_r[n] = (in_r[n] as f64 + signal_r) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Phaser does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0:  rate_hz       (0.05..10)
    // 1:  depth         (0..1)
    // 2:  stages        (2..12, stepped)
    // 3:  feedback      (-0.95..0.95)
    // 4:  min_freq      (20..5000)
    // 5:  max_freq      (200..20000)
    // 6:  stereo_phase  (0..180)
    // 7:  sync_mode     (0/1, stepped)
    // 8:  sync_note     (0..16, stepped)
    // 9:  bpm           (20..300)
    // 10: bypass        (0/1, stepped)

    fn param_count(&self) -> u32 {
        11
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Rate".into(),
                group: "Modulation".into(),
                min: 0.05,
                max: 10.0,
                default: 0.4,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Depth".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 1.0,
                default: 0.6,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Stages".into(),
                group: "Modulation".into(),
                min: 2.0,
                max: 12.0,
                default: 4.0,
                step_count: 5,
                flags: ParamFlags::STEPPED,
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Feedback".into(),
                group: "Modulation".into(),
                min: -0.95,
                max: 0.95,
                default: 0.3,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Min Freq".into(),
                group: "Frequency".into(),
                min: 20.0,
                max: 5000.0,
                default: 100.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Max Freq".into(),
                group: "Frequency".into(),
                min: 200.0,
                max: 20000.0,
                default: 5000.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Stereo Phase".into(),
                group: "Stereo".into(),
                min: 0.0,
                max: 180.0,
                default: 90.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            7 => Some(ParamInfo {
                id: 7,
                name: "Sync Mode".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            8 => Some(ParamInfo {
                id: 8,
                name: "Sync Note".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 16.0,
                default: 8.0,
                step_count: 16,
                flags: ParamFlags::STEPPED,
            }),
            9 => Some(ParamInfo {
                id: 9,
                name: "BPM".into(),
                group: "Sync".into(),
                min: 20.0,
                max: 300.0,
                default: 120.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            10 => Some(ParamInfo {
                id: 10,
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
            0 => Some(self.rate_hz),
            1 => Some(self.depth),
            2 => Some(self.stages as f64),
            3 => Some(self.feedback),
            4 => Some(self.min_freq),
            5 => Some(self.max_freq),
            6 => Some(self.stereo_phase),
            7 => Some(match self.sync_mode {
                SyncMode::Free => 0.0,
                SyncMode::Sync => 1.0,
            }),
            8 => Some(self.sync_note as f64),
            9 => Some(self.bpm),
            10 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.rate_hz = value.clamp(0.05, 10.0);
                self.rate_smoother.set_target(self.rate_hz);
            }
            1 => {
                self.depth = value.clamp(0.0, 1.0);
                self.depth_smoother.set_target(self.depth);
            }
            2 => {
                // Round to nearest even in {2,4,6,8,10,12} or just clamp to integer range
                let v = (value.round() as u32).clamp(2, MAX_STAGES as u32);
                self.stages = v;
            }
            3 => {
                self.feedback = value.clamp(-0.95, 0.95);
                self.feedback_smoother.set_target(self.feedback);
            }
            4 => {
                self.min_freq = value.clamp(20.0, 5000.0);
                self.min_freq_smoother.set_target(self.min_freq);
            }
            5 => {
                self.max_freq = value.clamp(200.0, 20000.0);
                self.max_freq_smoother.set_target(self.max_freq);
            }
            6 => {
                self.stereo_phase = value.clamp(0.0, 180.0);
                self.lfo_r.set_phase(self.stereo_phase / 360.0);
            }
            7 => {
                self.sync_mode = if value >= 0.5 {
                    SyncMode::Sync
                } else {
                    SyncMode::Free
                };
            }
            8 => {
                self.sync_note = (value.round() as u32).min(16);
            }
            9 => {
                self.bpm = value.clamp(20.0, 300.0);
            }
            10 => {
                self.bypass = value >= 0.5;
            }
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.2} Hz", value)),
            1 => Some(format!("{:.0}%", value * 100.0)),
            2 => Some(format!("{}", value.round() as u32)),
            3 => Some(format!("{:.0}%", value * 100.0)),
            4 => Some(format!("{:.0} Hz", value)),
            5 => Some(format!("{:.0} Hz", value)),
            6 => Some(format!("{:.0}\u{00b0}", value)),
            7 => Some(if value >= 0.5 { "Sync" } else { "Free" }.into()),
            8 => Some(
                match NoteValue::from_index(value.round() as u32) {
                    NoteValue::ThirtySecond => "1/32",
                    NoteValue::SixteenthTriplet => "1/16T",
                    NoteValue::Sixteenth => "1/16",
                    NoteValue::DottedSixteenth => "1/16.",
                    NoteValue::EighthTriplet => "1/8T",
                    NoteValue::Eighth => "1/8",
                    NoteValue::DottedEighth => "1/8.",
                    NoteValue::QuarterTriplet => "1/4T",
                    NoteValue::Quarter => "1/4",
                    NoteValue::DottedQuarter => "1/4.",
                    NoteValue::HalfTriplet => "1/2T",
                    NoteValue::Half => "1/2",
                    NoteValue::DottedHalf => "1/2.",
                    NoteValue::WholeTriplet => "1/1T",
                    NoteValue::Whole => "1/1",
                    NoteValue::TwoBar => "2 Bar",
                    NoteValue::FourBar => "4 Bar",
                }
                .into(),
            ),
            9 => Some(format!("{:.1} BPM", value)),
            10 => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
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
        let mut phaser = Phaser::new(44100);
        phaser.set_param(10, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        phaser.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
        let mut phaser = Phaser::new(44100);

        // rate_hz
        phaser.set_param(0, 2.5);
        assert_eq!(phaser.get_param(0), Some(2.5));

        // depth
        phaser.set_param(1, 0.8);
        assert_eq!(phaser.get_param(1), Some(0.8));

        // stages
        phaser.set_param(2, 6.0);
        assert_eq!(phaser.get_param(2), Some(6.0));

        // feedback
        phaser.set_param(3, -0.5);
        assert_eq!(phaser.get_param(3), Some(-0.5));

        // min_freq
        phaser.set_param(4, 200.0);
        assert_eq!(phaser.get_param(4), Some(200.0));

        // max_freq
        phaser.set_param(5, 10000.0);
        assert_eq!(phaser.get_param(5), Some(10000.0));

        // stereo_phase
        phaser.set_param(6, 120.0);
        assert_eq!(phaser.get_param(6), Some(120.0));

        // sync_mode
        phaser.set_param(7, 1.0);
        assert_eq!(phaser.get_param(7), Some(1.0));

        // sync_note
        phaser.set_param(8, 5.0);
        assert_eq!(phaser.get_param(8), Some(5.0));

        // bpm
        phaser.set_param(9, 140.0);
        assert_eq!(phaser.get_param(9), Some(140.0));

        // bypass
        phaser.set_param(10, 1.0);
        assert_eq!(phaser.get_param(10), Some(1.0));

        // Clamping
        phaser.set_param(0, -5.0);
        assert_eq!(phaser.get_param(0), Some(0.05));

        phaser.set_param(0, 100.0);
        assert_eq!(phaser.get_param(0), Some(10.0));

        phaser.set_param(1, -1.0);
        assert_eq!(phaser.get_param(1), Some(0.0));

        phaser.set_param(1, 5.0);
        assert_eq!(phaser.get_param(1), Some(1.0));

        phaser.set_param(2, 0.0);
        assert_eq!(phaser.get_param(2), Some(2.0));

        phaser.set_param(2, 100.0);
        assert_eq!(phaser.get_param(2), Some(12.0));

        phaser.set_param(3, -2.0);
        assert_eq!(phaser.get_param(3), Some(-0.95));

        phaser.set_param(3, 2.0);
        assert_eq!(phaser.get_param(3), Some(0.95));

        phaser.set_param(4, 1.0);
        assert_eq!(phaser.get_param(4), Some(20.0));

        phaser.set_param(5, 50000.0);
        assert_eq!(phaser.get_param(5), Some(20000.0));

        // Invalid param
        assert_eq!(phaser.get_param(99), None);
        assert!(phaser.param_info(11).is_none());
    }

    #[test]
    fn test_more_stages_more_notches() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        // Generate white noise input (deterministic seed)
        let mut rng: u64 = 0xDEAD_BEEF_CAFE_1234;
        let input: Vec<f32> = (0..num_samples)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                (rng as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0
            })
            .collect();

        // --- 4 stages ---
        let mut phaser_4 = Phaser::new(sr);
        phaser_4.set_param(2, 4.0); // 4 stages
        phaser_4.set_param(0, 0.5); // rate
        phaser_4.set_param(1, 1.0); // full depth
        phaser_4.set_param(3, 0.5); // feedback
        // Jump smoothers
        phaser_4.rate_smoother.reset(0.5);
        phaser_4.depth_smoother.reset(1.0);
        phaser_4.feedback_smoother.reset(0.5);

        let mut out_4_l = vec![0.0f32; num_samples];
        let mut out_4_r = vec![0.0f32; num_samples];
        phaser_4.process_effect(&input, &input, &mut out_4_l, &mut out_4_r);

        // --- 8 stages ---
        let mut phaser_8 = Phaser::new(sr);
        phaser_8.set_param(2, 8.0); // 8 stages
        phaser_8.set_param(0, 0.5);
        phaser_8.set_param(1, 1.0);
        phaser_8.set_param(3, 0.5);
        phaser_8.rate_smoother.reset(0.5);
        phaser_8.depth_smoother.reset(1.0);
        phaser_8.feedback_smoother.reset(0.5);

        let mut out_8_l = vec![0.0f32; num_samples];
        let mut out_8_r = vec![0.0f32; num_samples];
        phaser_8.process_effect(&input, &input, &mut out_8_l, &mut out_8_r);

        // Verify that 4-stage and 8-stage produce different output
        let skip = 2048;
        let diff_energy: f64 = out_4_l[skip..]
            .iter()
            .zip(out_8_l[skip..].iter())
            .map(|(&a, &b)| ((a - b) as f64).powi(2))
            .sum::<f64>()
            / (num_samples - skip) as f64;

        assert!(
            diff_energy > 1e-6,
            "4-stage and 8-stage should produce different output, diff_energy={diff_energy:.10}"
        );
    }

    #[test]
    fn test_feedback_deepens_notches() {
        let sr = 44100u32;
        let num_samples = sr as usize;

        // Generate white noise
        let mut rng: u64 = 0xCAFE_BABE_1234_5678;
        let input: Vec<f32> = (0..num_samples)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                (rng as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0
            })
            .collect();

        // --- Zero feedback ---
        let mut phaser_0fb = Phaser::new(sr);
        phaser_0fb.set_param(0, 0.05); // very slow LFO (near-static sweep)
        phaser_0fb.set_param(1, 0.5);
        phaser_0fb.set_param(3, 0.0); // zero feedback
        phaser_0fb.rate_smoother.reset(0.05);
        phaser_0fb.depth_smoother.reset(0.5);
        phaser_0fb.feedback_smoother.reset(0.0);

        let mut out_0fb_l = vec![0.0f32; num_samples];
        let mut out_0fb_r = vec![0.0f32; num_samples];
        phaser_0fb.process_effect(&input, &input, &mut out_0fb_l, &mut out_0fb_r);

        // --- High feedback ---
        let mut phaser_hfb = Phaser::new(sr);
        phaser_hfb.set_param(0, 0.05);
        phaser_hfb.set_param(1, 0.5);
        phaser_hfb.set_param(3, 0.9); // high feedback
        phaser_hfb.rate_smoother.reset(0.05);
        phaser_hfb.depth_smoother.reset(0.5);
        phaser_hfb.feedback_smoother.reset(0.9);

        let mut out_hfb_l = vec![0.0f32; num_samples];
        let mut out_hfb_r = vec![0.0f32; num_samples];
        phaser_hfb.process_effect(&input, &input, &mut out_hfb_l, &mut out_hfb_r);

        // Higher feedback should produce output that differs more from the
        // no-feedback case — the notches are deeper and more resonant.
        let skip = 2048;
        let diff_energy: f64 = out_0fb_l[skip..]
            .iter()
            .zip(out_hfb_l[skip..].iter())
            .map(|(&a, &b)| ((a - b) as f64).powi(2))
            .sum::<f64>()
            / (num_samples - skip) as f64;

        assert!(
            diff_energy > 1e-4,
            "High feedback should differ significantly from zero feedback, diff_energy={diff_energy:.10}"
        );
    }

    #[test]
    fn test_frequency_sweep_range() {
        let sr = 44100u32;
        let num_samples = sr as usize;

        // Generate a 1 kHz sine
        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                ((i as f64 / sr as f64) * 1000.0 * std::f64::consts::TAU).sin() as f32
            })
            .collect();

        // --- Low range (100-500 Hz) ---
        let mut phaser_low = Phaser::new(sr);
        phaser_low.set_param(4, 100.0);
        phaser_low.set_param(5, 500.0);
        phaser_low.set_param(1, 1.0);
        phaser_low.min_freq_smoother.reset(100.0);
        phaser_low.max_freq_smoother.reset(500.0);
        phaser_low.depth_smoother.reset(1.0);

        let mut out_low_l = vec![0.0f32; num_samples];
        let mut out_low_r = vec![0.0f32; num_samples];
        phaser_low.process_effect(&input, &input, &mut out_low_l, &mut out_low_r);

        // --- High range (5000-20000 Hz) ---
        let mut phaser_high = Phaser::new(sr);
        phaser_high.set_param(4, 5000.0);
        phaser_high.set_param(5, 20000.0);
        phaser_high.set_param(1, 1.0);
        phaser_high.min_freq_smoother.reset(5000.0);
        phaser_high.max_freq_smoother.reset(20000.0);
        phaser_high.depth_smoother.reset(1.0);

        let mut out_high_l = vec![0.0f32; num_samples];
        let mut out_high_r = vec![0.0f32; num_samples];
        phaser_high.process_effect(&input, &input, &mut out_high_l, &mut out_high_r);

        // The two frequency ranges should produce different output
        let skip = 2048;
        let diff_energy: f64 = out_low_l[skip..]
            .iter()
            .zip(out_high_l[skip..].iter())
            .map(|(&a, &b)| ((a - b) as f64).powi(2))
            .sum::<f64>()
            / (num_samples - skip) as f64;

        assert!(
            diff_energy > 1e-6,
            "Different freq ranges should produce different output, diff_energy={diff_energy:.10}"
        );
    }

    #[test]
    fn test_param_info_complete() {
        let phaser = Phaser::new(44100);
        assert_eq!(phaser.param_count(), 11);

        for i in 0..11 {
            let info = phaser.param_info(i);
            assert!(
                info.is_some(),
                "param_info({}) should return Some",
                i
            );
            let info = info.unwrap();
            assert_eq!(info.id, i);
            assert!(!info.name.is_empty(), "param {} name should not be empty", i);
            assert!(
                !info.group.is_empty(),
                "param {} group should not be empty",
                i
            );
        }

        // No param beyond 10
        assert!(phaser.param_info(11).is_none());
    }
}
