//! Flanger with through-zero feedback and soft saturation.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. Uses a short modulated delay line with sinc
//! interpolation. Negative feedback values enable through-zero mode,
//! producing the characteristic "jet sweep" sound by flipping the polarity
//! of the feedback signal.
//!
//! ## Algorithm
//!
//! ```text
//! For each channel (L has lfo_phase=0, R has lfo_phase=stereo_phase/360):
//!     modulated_delay = delay_ms + lfo.next(rate) * depth * delay_ms
//!     delayed = delay_line.read(modulated_delay * sr / 1000)
//!     feedback_sample = flush_denormal(tanh(delayed * |feedback|))
//!     if feedback < 0: feedback_sample = -feedback_sample  // through-zero
//!     delay_line.write(input + feedback_sample)
//!     wet = delayed
//!     output = (1-mix) * input + mix * wet
//! ```

use super::delay_line::FractionalDelayLine;
use super::lfo::{Lfo, LfoShape, NoteValue};
use crate::common::{flush_denormal, ParamSmoother};
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

/// Maximum delay line length in milliseconds.
///
/// Must exceed delay_ms max (10 ms) plus modulation excursion (~10 ms at
/// full depth) with headroom for the sinc kernel.
const MAX_DELAY_MS: f64 = 25.0;

/// Sinc kernel width (8-point Kaiser-windowed).
const SINC_POINTS: usize = 8;

/// Smoothing ramp time in milliseconds for parameter changes.
const SMOOTH_MS: f64 = 5.0;

// ---------------------------------------------------------------------------
// Sync mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    Free,
    Sync,
}

// ---------------------------------------------------------------------------
// Flanger
// ---------------------------------------------------------------------------

/// Flanger effect with through-zero feedback and tanh soft saturation.
///
/// Short modulated delay (0.1-10 ms) with feedback creates comb-filter
/// sweeps. Negative feedback values flip the wet signal polarity,
/// producing through-zero flanging (the classic "jet engine" effect).
/// The tanh saturator in the feedback path prevents harsh metallic
/// artifacts at high feedback settings.
pub struct Flanger {
    sample_rate: u32,

    // Parameters
    rate_hz: f64,
    depth: f64,
    delay_ms: f64,
    feedback: f64,
    stereo_phase: f64,
    lfo_shape: LfoShape,
    dry_wet: f64,
    bypass: bool,
    sync_mode: SyncMode,
    sync_note: u32,
    bpm: f64,

    // Per-channel state
    delay_line_l: FractionalDelayLine,
    delay_line_r: FractionalDelayLine,
    lfo_l: Lfo,
    lfo_r: Lfo,

    // Smoothers
    rate_smoother: ParamSmoother,
    depth_smoother: ParamSmoother,
    delay_smoother: ParamSmoother,
    feedback_smoother: ParamSmoother,
    mix_smoother: ParamSmoother,
}

impl Flanger {
    /// Create a new flanger with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        let mut lfo_r = Lfo::new(sample_rate);
        lfo_r.set_phase(90.0 / 360.0); // default stereo_phase = 90 degrees

        Self {
            sample_rate,

            rate_hz: 0.5,
            depth: 0.7,
            delay_ms: 2.0,
            feedback: 0.5,
            stereo_phase: 90.0,
            lfo_shape: LfoShape::Sine,
            dry_wet: 0.5,
            bypass: false,
            sync_mode: SyncMode::Free,
            sync_note: 8, // Quarter
            bpm: 120.0,

            delay_line_l: FractionalDelayLine::new(MAX_DELAY_MS, sample_rate, SINC_POINTS),
            delay_line_r: FractionalDelayLine::new(MAX_DELAY_MS, sample_rate, SINC_POINTS),
            lfo_l: Lfo::new(sample_rate),
            lfo_r,

            rate_smoother: ParamSmoother::new(0.5, sr, SMOOTH_MS),
            depth_smoother: ParamSmoother::new(0.7, sr, SMOOTH_MS),
            delay_smoother: ParamSmoother::new(2.0, sr, SMOOTH_MS),
            feedback_smoother: ParamSmoother::new(0.5, sr, SMOOTH_MS),
            mix_smoother: ParamSmoother::new(0.5, sr, SMOOTH_MS),
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

    /// Soft saturation via hyperbolic tangent.
    #[inline]
    fn soft_saturate(x: f64) -> f64 {
        x.tanh()
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Flanger {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Flanger",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.delay_line_l.clear();
        self.delay_line_r.clear();
        self.lfo_l.reset_phase();
        self.lfo_r.reset_phase();
        self.lfo_r.set_phase(self.stereo_phase / 360.0);
        self.rate_smoother.reset(self.rate_hz);
        self.depth_smoother.reset(self.depth);
        self.delay_smoother.reset(self.delay_ms);
        self.feedback_smoother.reset(self.feedback);
        self.mix_smoother.reset(self.dry_wet);
    }

    // -- MIDI: no-op for a flanger effect --
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

        for n in 0..len {
            let rate = self.rate_smoother.next();
            let depth = self.depth_smoother.next();
            let base_delay = self.delay_smoother.next();
            let feedback = self.feedback_smoother.next();
            let mix = self.mix_smoother.next();

            let freq = self.lfo_freq(rate);
            let fb_abs = feedback.abs();
            let fb_sign = if feedback < 0.0 { -1.0 } else { 1.0 };

            // --- Left channel ---
            let lfo_val_l = self.lfo_l.next(freq);
            let mod_delay_l = base_delay + lfo_val_l * depth * base_delay;
            let delay_samples_l = (mod_delay_l * sr / 1000.0).max(1.0);
            let delayed_l = self.delay_line_l.read(delay_samples_l) as f64;

            let fb_sample_l =
                flush_denormal(Self::soft_saturate(delayed_l * fb_abs) as f32) as f64
                    * fb_sign;
            self.delay_line_l
                .write((in_l[n] as f64 + fb_sample_l) as f32);

            // --- Right channel ---
            let lfo_val_r = self.lfo_r.next(freq);
            let mod_delay_r = base_delay + lfo_val_r * depth * base_delay;
            let delay_samples_r = (mod_delay_r * sr / 1000.0).max(1.0);
            let delayed_r = self.delay_line_r.read(delay_samples_r) as f64;

            let fb_sample_r =
                flush_denormal(Self::soft_saturate(delayed_r * fb_abs) as f32) as f64
                    * fb_sign;
            self.delay_line_r
                .write((in_r[n] as f64 + fb_sample_r) as f32);

            // --- Mix ---
            let dry = 1.0 - mix;
            out_l[n] = (dry * in_l[n] as f64 + mix * delayed_l) as f32;
            out_r[n] = (dry * in_r[n] as f64 + mix * delayed_r) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Flanger does not have a volume control.
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
    // 2:  delay_ms      (0.1..10)
    // 3:  feedback      (-0.95..0.95)
    // 4:  stereo_phase  (0..180)
    // 5:  lfo_shape     (0..4, stepped)
    // 6:  dry_wet       (0..1)
    // 7:  bypass        (0/1, stepped)
    // 8:  sync_mode     (0/1, stepped)
    // 9:  sync_note     (0..16, stepped)
    // 10: bpm           (20..300)

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
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Depth".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 1.0,
                default: 0.7,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Delay".into(),
                group: "Modulation".into(),
                min: 0.1,
                max: 10.0,
                default: 2.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Feedback".into(),
                group: "Modulation".into(),
                min: -0.95,
                max: 0.95,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Stereo Phase".into(),
                group: "Stereo".into(),
                min: 0.0,
                max: 180.0,
                default: 90.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "LFO Shape".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 4.0,
                default: 0.0,
                step_count: 4,
                flags: ParamFlags::STEPPED,
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Dry/Wet".into(),
                group: "Mix".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            7 => Some(ParamInfo {
                id: 7,
                name: "Bypass".into(),
                group: "Global".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            8 => Some(ParamInfo {
                id: 8,
                name: "Sync Mode".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            9 => Some(ParamInfo {
                id: 9,
                name: "Sync Note".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 16.0,
                default: 8.0,
                step_count: 16,
                flags: ParamFlags::STEPPED,
            }),
            10 => Some(ParamInfo {
                id: 10,
                name: "BPM".into(),
                group: "Sync".into(),
                min: 20.0,
                max: 300.0,
                default: 120.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            _ => None,
        }
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            0 => Some(self.rate_hz),
            1 => Some(self.depth),
            2 => Some(self.delay_ms),
            3 => Some(self.feedback),
            4 => Some(self.stereo_phase),
            5 => Some(match self.lfo_shape {
                LfoShape::Sine => 0.0,
                LfoShape::Triangle => 1.0,
                LfoShape::Saw => 2.0,
                LfoShape::Square => 3.0,
                LfoShape::SampleAndHold => 4.0,
            }),
            6 => Some(self.dry_wet),
            7 => Some(if self.bypass { 1.0 } else { 0.0 }),
            8 => Some(match self.sync_mode {
                SyncMode::Free => 0.0,
                SyncMode::Sync => 1.0,
            }),
            9 => Some(self.sync_note as f64),
            10 => Some(self.bpm),
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
                self.delay_ms = value.clamp(0.1, 10.0);
                self.delay_smoother.set_target(self.delay_ms);
            }
            3 => {
                self.feedback = value.clamp(-0.95, 0.95);
                self.feedback_smoother.set_target(self.feedback);
            }
            4 => {
                self.stereo_phase = value.clamp(0.0, 180.0);
                self.lfo_r.set_phase(self.stereo_phase / 360.0);
            }
            5 => {
                let shape = LfoShape::from_index(value.round() as u32);
                self.lfo_shape = shape;
                self.lfo_l.set_shape(shape);
                self.lfo_r.set_shape(shape);
            }
            6 => {
                self.dry_wet = value.clamp(0.0, 1.0);
                self.mix_smoother.set_target(self.dry_wet);
            }
            7 => {
                self.bypass = value >= 0.5;
            }
            8 => {
                self.sync_mode = if value >= 0.5 {
                    SyncMode::Sync
                } else {
                    SyncMode::Free
                };
            }
            9 => {
                self.sync_note = (value.round() as u32).min(16);
            }
            10 => {
                self.bpm = value.clamp(20.0, 300.0);
            }
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.2} Hz", value)),
            1 => Some(format!("{:.0}%", value * 100.0)),
            2 => Some(format!("{:.1} ms", value)),
            3 => Some(format!("{:.0}%", value * 100.0)),
            4 => Some(format!("{:.0}\u{00b0}", value)),
            5 => Some(
                match LfoShape::from_index(value.round() as u32) {
                    LfoShape::Sine => "Sine",
                    LfoShape::Triangle => "Tri",
                    LfoShape::Saw => "Saw",
                    LfoShape::Square => "Sq",
                    LfoShape::SampleAndHold => "S&H",
                }
                .into(),
            ),
            6 => Some(format!("{:.0}%", value * 100.0)),
            7 => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
            8 => Some(if value >= 0.5 { "Sync" } else { "Free" }.into()),
            9 => Some(
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
            10 => Some(format!("{:.1} BPM", value)),
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
        let mut flanger = Flanger::new(44100);
        flanger.set_param(7, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        flanger.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
        let mut flanger = Flanger::new(44100);

        // rate_hz
        flanger.set_param(0, 2.5);
        assert_eq!(flanger.get_param(0), Some(2.5));

        // depth
        flanger.set_param(1, 0.8);
        assert_eq!(flanger.get_param(1), Some(0.8));

        // delay_ms
        flanger.set_param(2, 5.0);
        assert_eq!(flanger.get_param(2), Some(5.0));

        // feedback
        flanger.set_param(3, -0.7);
        assert_eq!(flanger.get_param(3), Some(-0.7));

        // stereo_phase
        flanger.set_param(4, 120.0);
        assert_eq!(flanger.get_param(4), Some(120.0));

        // lfo_shape
        flanger.set_param(5, 2.0);
        assert_eq!(flanger.get_param(5), Some(2.0));

        // dry_wet
        flanger.set_param(6, 0.75);
        assert_eq!(flanger.get_param(6), Some(0.75));

        // bypass
        flanger.set_param(7, 1.0);
        assert_eq!(flanger.get_param(7), Some(1.0));

        // sync_mode
        flanger.set_param(8, 1.0);
        assert_eq!(flanger.get_param(8), Some(1.0));

        // sync_note
        flanger.set_param(9, 5.0);
        assert_eq!(flanger.get_param(9), Some(5.0));

        // bpm
        flanger.set_param(10, 140.0);
        assert_eq!(flanger.get_param(10), Some(140.0));

        // Clamping
        flanger.set_param(0, -5.0);
        assert_eq!(flanger.get_param(0), Some(0.05));

        flanger.set_param(0, 100.0);
        assert_eq!(flanger.get_param(0), Some(10.0));

        flanger.set_param(1, -1.0);
        assert_eq!(flanger.get_param(1), Some(0.0));

        flanger.set_param(1, 5.0);
        assert_eq!(flanger.get_param(1), Some(1.0));

        flanger.set_param(2, 0.0);
        assert_eq!(flanger.get_param(2), Some(0.1));

        flanger.set_param(2, 100.0);
        assert_eq!(flanger.get_param(2), Some(10.0));

        flanger.set_param(3, -2.0);
        assert_eq!(flanger.get_param(3), Some(-0.95));

        flanger.set_param(3, 2.0);
        assert_eq!(flanger.get_param(3), Some(0.95));

        // Invalid param
        assert_eq!(flanger.get_param(99), None);
        assert!(flanger.param_info(11).is_none());
    }

    #[test]
    fn test_through_zero_flips_polarity() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        // Generate an impulse followed by silence — the delayed output will
        // show the feedback polarity clearly.
        let mut input = vec![0.0f32; num_samples];
        input[0] = 1.0;

        // --- Positive feedback ---
        let mut flanger_pos = Flanger::new(sr);
        flanger_pos.set_param(0, 0.05); // very slow LFO
        flanger_pos.set_param(1, 0.0); // no depth modulation
        flanger_pos.set_param(2, 2.0); // 2 ms delay
        flanger_pos.set_param(3, 0.5); // positive feedback
        flanger_pos.set_param(6, 1.0); // 100% wet
        // Jump smoothers
        flanger_pos.rate_smoother.reset(0.05);
        flanger_pos.depth_smoother.reset(0.0);
        flanger_pos.delay_smoother.reset(2.0);
        flanger_pos.feedback_smoother.reset(0.5);
        flanger_pos.mix_smoother.reset(1.0);

        let mut out_pos_l = vec![0.0f32; num_samples];
        let mut out_pos_r = vec![0.0f32; num_samples];
        flanger_pos.process_effect(&input, &input, &mut out_pos_l, &mut out_pos_r);

        // --- Negative feedback (through-zero) ---
        let mut flanger_neg = Flanger::new(sr);
        flanger_neg.set_param(0, 0.05);
        flanger_neg.set_param(1, 0.0);
        flanger_neg.set_param(2, 2.0);
        flanger_neg.set_param(3, -0.5); // negative feedback
        flanger_neg.set_param(6, 1.0);
        flanger_neg.rate_smoother.reset(0.05);
        flanger_neg.depth_smoother.reset(0.0);
        flanger_neg.delay_smoother.reset(2.0);
        flanger_neg.feedback_smoother.reset(-0.5);
        flanger_neg.mix_smoother.reset(1.0);

        let mut out_neg_l = vec![0.0f32; num_samples];
        let mut out_neg_r = vec![0.0f32; num_samples];
        flanger_neg.process_effect(&input, &input, &mut out_neg_l, &mut out_neg_r);

        // The first reflection (at ~2ms = ~88 samples) should be the same
        // magnitude but opposite sign for negative vs positive feedback.
        // Find the first significant peak after the initial impulse.
        let delay_samples = (2.0 * sr as f64 / 1000.0).round() as usize;
        let search_start = delay_samples.saturating_sub(5);
        let search_end = (delay_samples + 10).min(num_samples);

        // Find peak in the search window
        let mut max_pos = 0.0f32;
        let mut max_neg = 0.0f32;
        let mut peak_pos_idx = search_start;
        let mut peak_neg_idx = search_start;

        for i in search_start..search_end {
            if out_pos_l[i].abs() > max_pos {
                max_pos = out_pos_l[i].abs();
                peak_pos_idx = i;
            }
            if out_neg_l[i].abs() > max_neg {
                max_neg = out_neg_l[i].abs();
                peak_neg_idx = i;
            }
        }

        // Second reflection should have flipped polarity
        let second_start = delay_samples * 2 - 5;
        let second_end = (delay_samples * 2 + 10).min(num_samples);

        let mut second_peak_pos = 0.0f64;
        let mut second_peak_neg = 0.0f64;
        let mut second_pos_sign = 1.0f64;
        let mut second_neg_sign = 1.0f64;

        for i in second_start..second_end {
            if (out_pos_l[i] as f64).abs() > second_peak_pos {
                second_peak_pos = (out_pos_l[i] as f64).abs();
                second_pos_sign = if out_pos_l[i] >= 0.0 { 1.0 } else { -1.0 };
            }
            if (out_neg_l[i] as f64).abs() > second_peak_neg {
                second_peak_neg = (out_neg_l[i] as f64).abs();
                second_neg_sign = if out_neg_l[i] >= 0.0 { 1.0 } else { -1.0 };
            }
        }

        // The second reflection of negative feedback should have opposite sign
        // compared to positive feedback (through-zero polarity flip propagates)
        assert!(
            second_peak_pos > 1e-4 && second_peak_neg > 1e-4,
            "Second reflection should be detectable: pos={}, neg={}",
            second_peak_pos,
            second_peak_neg,
        );

        // With positive feedback the second echo has the same sign as the first;
        // with negative feedback it should be flipped.
        assert!(
            second_pos_sign != second_neg_sign
                || (out_pos_l[peak_pos_idx].signum() != out_neg_l[peak_neg_idx].signum()),
            "Through-zero feedback should flip polarity somewhere in the feedback chain"
        );
    }

    #[test]
    fn test_saturation_bounds_output() {
        let sr = 44100u32;
        let num_samples = sr as usize * 2; // 2 seconds

        // Generate a loud 100 Hz square wave — lots of energy to stress feedback
        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                let phase = (i as f64 / sr as f64) * 100.0;
                if phase.fract() < 0.5 { 0.9 } else { -0.9 }
            })
            .collect();

        let mut flanger = Flanger::new(sr);
        flanger.set_param(0, 1.0); // rate
        flanger.set_param(1, 0.5); // depth
        flanger.set_param(2, 3.0); // delay
        flanger.set_param(3, 0.94); // near-max feedback
        flanger.set_param(6, 1.0); // 100% wet
        // Jump smoothers
        flanger.rate_smoother.reset(1.0);
        flanger.depth_smoother.reset(0.5);
        flanger.delay_smoother.reset(3.0);
        flanger.feedback_smoother.reset(0.94);
        flanger.mix_smoother.reset(1.0);

        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        flanger.process_effect(&input, &input, &mut out_l, &mut out_r);

        // Output should stay bounded — tanh saturation prevents explosion
        let max_output = out_l
            .iter()
            .chain(out_r.iter())
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);

        assert!(
            max_output < 5.0,
            "Output should be bounded by soft saturation, got peak {}",
            max_output,
        );
    }

    #[test]
    fn test_feedback_zero_no_resonance() {
        let sr = 44100u32;
        let num_samples = sr as usize;

        // Single impulse
        let mut input = vec![0.0f32; num_samples];
        input[0] = 1.0;

        let mut flanger = Flanger::new(sr);
        flanger.set_param(0, 0.05); // slow LFO
        flanger.set_param(1, 0.0); // no modulation
        flanger.set_param(2, 2.0); // 2 ms delay
        flanger.set_param(3, 0.0); // zero feedback
        flanger.set_param(6, 1.0); // 100% wet
        // Jump smoothers
        flanger.rate_smoother.reset(0.05);
        flanger.depth_smoother.reset(0.0);
        flanger.delay_smoother.reset(2.0);
        flanger.feedback_smoother.reset(0.0);
        flanger.mix_smoother.reset(1.0);

        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        flanger.process_effect(&input, &input, &mut out_l, &mut out_r);

        // With feedback=0, there should be exactly one delayed impulse
        // and no subsequent echoes.
        let delay_samples = (2.0 * sr as f64 / 1000.0).round() as usize;

        // Count significant peaks after the main impulse
        let mut peak_count = 0;
        let threshold = 0.01;
        let mut in_peak = false;

        for i in (delay_samples + 10)..num_samples {
            if out_l[i].abs() > threshold {
                if !in_peak {
                    peak_count += 1;
                    in_peak = true;
                }
            } else {
                in_peak = false;
            }
        }

        assert_eq!(
            peak_count, 0,
            "feedback=0 should produce no echoes after the initial delay tap, found {} peaks",
            peak_count,
        );
    }

    #[test]
    fn test_param_info_complete() {
        let flanger = Flanger::new(44100);
        assert_eq!(flanger.param_count(), 11);

        for i in 0..11 {
            let info = flanger.param_info(i);
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
        assert!(flanger.param_info(11).is_none());
    }
}
