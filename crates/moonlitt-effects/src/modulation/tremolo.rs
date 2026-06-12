//! Tremolo with tempo sync and stereo auto-pan.
//!
//! Pure amplitude modulation via LFO. Implements the `AudioBackend` trait
//! from `moonlitt-core` as an audio effect processor.
//!
//! ## Algorithm
//!
//! ```text
//! gain = 1.0 - depth + depth * (lfo_value * 0.5 + 0.5)
//! output_L = input_L * gain_L
//! output_R = input_R * gain_R
//! ```
//!
//! - LFO output is \[-1, 1\], mapped to \[0, 1\] for gain modulation.
//! - depth=0 -> gain=1 (no effect); depth=1 -> gain swings 0..1.
//! - Mono mode: same LFO for both channels.
//! - Stereo (auto-pan) mode: R channel LFO is 180deg out of phase with L.
//! - Tempo sync: derives LFO frequency from BPM and note value.

use super::lfo::{Lfo, LfoShape, NoteValue};
use crate::common::ParamSmoother;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Stereo mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StereoMode {
    Mono,
    Stereo,
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
// Tremolo
// ---------------------------------------------------------------------------

/// Tremolo effect with 5 LFO waveforms, tempo sync, and stereo auto-pan.
pub struct Tremolo {
    sample_rate: u32,

    // Parameters
    rate_hz: f64,
    depth: f64,
    lfo_shape: LfoShape,
    stereo_mode: StereoMode,
    sync_mode: SyncMode,
    sync_note: u32,
    bpm: f64,
    bypass: bool,

    // Internal state
    lfo_l: Lfo,
    lfo_r: Lfo,
    depth_smoother: ParamSmoother,
    rate_smoother: ParamSmoother,
}

impl Tremolo {
    /// Create a new tremolo with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        let mut lfo_r = Lfo::new(sample_rate);
        lfo_r.set_phase(0.5); // 180 degrees offset for stereo mode

        Self {
            sample_rate,

            rate_hz: 4.0,
            depth: 0.5,
            lfo_shape: LfoShape::Sine,
            stereo_mode: StereoMode::Mono,
            sync_mode: SyncMode::Free,
            sync_note: 8, // Quarter
            bpm: 120.0,
            bypass: false,

            lfo_l: Lfo::new(sample_rate),
            lfo_r,
            depth_smoother: ParamSmoother::new(0.5, sr, 5.0),
            rate_smoother: ParamSmoother::new(4.0, sr, 5.0),
        }
    }

    /// Advance the left LFO one sample and return its output.
    #[inline]
    fn advance_lfo_l(&mut self) -> f64 {
        match self.sync_mode {
            SyncMode::Free => {
                let rate = self.rate_smoother.next();
                self.lfo_l.next(rate)
            }
            SyncMode::Sync => {
                let note = NoteValue::from_index(self.sync_note);
                self.lfo_l.next_synced(self.bpm, note)
            }
        }
    }

    /// Advance the right LFO one sample and return its output.
    #[inline]
    fn advance_lfo_r(&mut self) -> f64 {
        match self.sync_mode {
            SyncMode::Free => {
                let rate = self.rate_smoother.next_value(); // already advanced by L
                self.lfo_r.next(rate)
            }
            SyncMode::Sync => {
                let note = NoteValue::from_index(self.sync_note);
                self.lfo_r.next_synced(self.bpm, note)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Tremolo {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Tremolo",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.lfo_l.reset_phase();
        self.lfo_r.reset_phase();
        self.lfo_r.set_phase(0.5);
        self.depth_smoother.reset(self.depth);
        self.rate_smoother.reset(self.rate_hz);
    }

    // -- MIDI: no-op for a tremolo effect --
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

        for i in 0..len {
            let depth = self.depth_smoother.next();

            // Advance L LFO
            let lfo_val_l = self.advance_lfo_l();

            // Map LFO (-1..1) to gain (0..1), then apply depth
            let gain_l = 1.0 - depth + depth * (lfo_val_l * 0.5 + 0.5);

            let gain_r = match self.stereo_mode {
                StereoMode::Mono => gain_l,
                StereoMode::Stereo => {
                    let lfo_val_r = self.advance_lfo_r();
                    1.0 - depth + depth * (lfo_val_r * 0.5 + 0.5)
                }
            };

            out_l[i] = (in_l[i] as f64 * gain_l) as f32;
            out_r[i] = (in_r[i] as f64 * gain_r) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Tremolo does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: rate_hz       (0.1..20)
    // 1: depth         (0..1)
    // 2: lfo_shape     (0..4, stepped)
    // 3: stereo_mode   (0/1, stepped)
    // 4: sync_mode     (0/1, stepped)
    // 5: sync_note     (0..16, stepped)
    // 6: bpm           (20..300)
    // 7: bypass        (0/1, stepped)

    fn param_count(&self) -> u32 {
        8
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Rate".into(),
                group: "Modulation".into(),
                min: 0.1,
                max: 20.0,
                default: 4.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Depth".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "LFO Shape".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 4.0,
                default: 0.0,
                step_count: 4,
                flags: ParamFlags::STEPPED,
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Stereo Mode".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Sync Mode".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Sync Note".into(),
                group: "Sync".into(),
                min: 0.0,
                max: 16.0,
                default: 8.0,
                step_count: 16,
                flags: ParamFlags::STEPPED,
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "BPM".into(),
                group: "Sync".into(),
                min: 20.0,
                max: 300.0,
                default: 120.0,
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
            _ => None,
        }
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            0 => Some(self.rate_hz),
            1 => Some(self.depth),
            2 => Some(match self.lfo_shape {
                LfoShape::Sine => 0.0,
                LfoShape::Triangle => 1.0,
                LfoShape::Saw => 2.0,
                LfoShape::Square => 3.0,
                LfoShape::SampleAndHold => 4.0,
            }),
            3 => Some(match self.stereo_mode {
                StereoMode::Mono => 0.0,
                StereoMode::Stereo => 1.0,
            }),
            4 => Some(match self.sync_mode {
                SyncMode::Free => 0.0,
                SyncMode::Sync => 1.0,
            }),
            5 => Some(self.sync_note as f64),
            6 => Some(self.bpm),
            7 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.rate_hz = value.clamp(0.1, 20.0);
                self.rate_smoother.set_target(self.rate_hz);
            }
            1 => {
                self.depth = value.clamp(0.0, 1.0);
                self.depth_smoother.set_target(self.depth);
            }
            2 => {
                let shape = LfoShape::from_index(value.round() as u32);
                self.lfo_shape = shape;
                self.lfo_l.set_shape(shape);
                self.lfo_r.set_shape(shape);
            }
            3 => {
                self.stereo_mode = if value >= 0.5 {
                    StereoMode::Stereo
                } else {
                    StereoMode::Mono
                };
            }
            4 => {
                self.sync_mode = if value >= 0.5 {
                    SyncMode::Sync
                } else {
                    SyncMode::Free
                };
            }
            5 => {
                self.sync_note = (value.round() as u32).min(16);
            }
            6 => {
                self.bpm = value.clamp(20.0, 300.0);
            }
            7 => {
                self.bypass = value >= 0.5;
            }
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.1} Hz", value)),
            1 => Some(format!("{:.0}%", value * 100.0)),
            2 => Some(
                match LfoShape::from_index(value.round() as u32) {
                    LfoShape::Sine => "Sine",
                    LfoShape::Triangle => "Triangle",
                    LfoShape::Saw => "Saw",
                    LfoShape::Square => "Square",
                    LfoShape::SampleAndHold => "S&H",
                }
                .into(),
            ),
            3 => Some(if value >= 0.5 { "Stereo" } else { "Mono" }.into()),
            4 => Some(if value >= 0.5 { "Sync" } else { "Free" }.into()),
            5 => Some(
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
            6 => Some(format!("{:.1} BPM", value)),
            7 => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
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
        let mut trem = Tremolo::new(44100);
        trem.set_param(7, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        trem.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
        let mut trem = Tremolo::new(44100);

        // rate_hz
        trem.set_param(0, 7.5);
        assert_eq!(trem.get_param(0), Some(7.5));

        // depth
        trem.set_param(1, 0.8);
        assert_eq!(trem.get_param(1), Some(0.8));

        // lfo_shape
        trem.set_param(2, 2.0);
        assert_eq!(trem.get_param(2), Some(2.0));

        // stereo_mode
        trem.set_param(3, 1.0);
        assert_eq!(trem.get_param(3), Some(1.0));

        // sync_mode
        trem.set_param(4, 1.0);
        assert_eq!(trem.get_param(4), Some(1.0));

        // sync_note
        trem.set_param(5, 5.0);
        assert_eq!(trem.get_param(5), Some(5.0));

        // bpm
        trem.set_param(6, 140.0);
        assert_eq!(trem.get_param(6), Some(140.0));

        // bypass
        trem.set_param(7, 1.0);
        assert_eq!(trem.get_param(7), Some(1.0));

        // Clamping
        trem.set_param(0, -5.0);
        assert_eq!(trem.get_param(0), Some(0.1));

        trem.set_param(0, 100.0);
        assert_eq!(trem.get_param(0), Some(20.0));

        trem.set_param(1, -1.0);
        assert_eq!(trem.get_param(1), Some(0.0));

        trem.set_param(1, 5.0);
        assert_eq!(trem.get_param(1), Some(1.0));

        // Invalid param
        assert_eq!(trem.get_param(99), None);
        assert!(trem.param_info(8).is_none());
    }

    #[test]
    fn test_depth_zero_passthrough() {
        let mut trem = Tremolo::new(44100);
        trem.set_param(1, 0.0); // depth = 0
                                // Jump smoother to target immediately
        trem.depth_smoother.reset(0.0);

        let input: Vec<f32> = (0..1024)
            .map(|i| ((i as f64 / 44100.0) * 440.0 * std::f64::consts::TAU).sin() as f32)
            .collect();
        let mut out_l = vec![0.0f32; 1024];
        let mut out_r = vec![0.0f32; 1024];

        trem.process_effect(&input, &input, &mut out_l, &mut out_r);

        for i in 0..1024 {
            let diff = (out_l[i] - input[i]).abs();
            assert!(
                diff < 1e-6,
                "depth=0: sample {} differs by {:.9} (in={}, out={})",
                i,
                diff,
                input[i],
                out_l[i]
            );
        }
    }

    #[test]
    fn test_depth_one_reaches_silence() {
        let sr = 44100u32;
        let mut trem = Tremolo::new(sr);
        trem.set_param(0, 1.0); // rate = 1 Hz
        trem.set_param(1, 1.0); // depth = 1.0 (full)
        trem.set_param(2, 0.0); // Sine LFO
                                // Jump smoothers to target immediately
        trem.depth_smoother.reset(1.0);
        trem.rate_smoother.reset(1.0);

        // Generate 2 seconds of constant amplitude signal
        let num_samples = sr as usize * 2;
        let input = vec![0.5f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        trem.process_effect(&input, &input, &mut out_l, &mut out_r);

        // With depth=1 and sine LFO, the gain should reach near-zero
        // when the LFO is at its minimum (-1 -> mapped gain = 0).
        // Find the minimum output sample magnitude.
        let min_output = out_l.iter().map(|s| s.abs()).fold(f32::MAX, f32::min);

        assert!(
            min_output < 0.01,
            "depth=1: minimum output should be near zero, got {}",
            min_output
        );
    }

    #[test]
    fn test_stereo_mode_opposite() {
        let sr = 44100u32;
        let mut trem = Tremolo::new(sr);
        trem.set_param(0, 2.0); // rate = 2 Hz
        trem.set_param(1, 1.0); // depth = 1.0 (full)
        trem.set_param(2, 0.0); // Sine LFO
        trem.set_param(3, 1.0); // Stereo mode
                                // Jump smoothers to target immediately
        trem.depth_smoother.reset(1.0);
        trem.rate_smoother.reset(2.0);

        // Generate 1 second of constant amplitude signal
        let num_samples = sr as usize;
        let input = vec![1.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        trem.process_effect(&input, &input, &mut out_l, &mut out_r);

        // In stereo mode, when L is at max gain, R should be at min gain
        // (180 degrees out of phase). Find the sample where L is maximum.
        let (max_l_idx, _max_l_val) = out_l
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap();

        let l_at_peak = out_l[max_l_idx];
        let r_at_peak = out_r[max_l_idx];

        // When L is near max (1.0), R should be near min (0.0)
        assert!(
            l_at_peak > 0.9,
            "L channel peak should be near 1.0, got {}",
            l_at_peak
        );
        assert!(
            r_at_peak < 0.15,
            "R channel at L's peak should be near 0.0, got {}",
            r_at_peak
        );
    }

    #[test]
    fn test_param_info_complete() {
        let trem = Tremolo::new(44100);
        assert_eq!(trem.param_count(), 8);

        for i in 0..8 {
            let info = trem.param_info(i);
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
        }

        // No param beyond 7
        assert!(trem.param_info(8).is_none());
    }
}
