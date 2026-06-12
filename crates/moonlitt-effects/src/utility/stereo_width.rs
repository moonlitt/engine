//! Stereo width processor using Mid/Side encoding.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. Uses `ParamSmoother` for zipper-free parameter changes.
//!
//! ## Algorithm
//!
//! ```text
//! mid  = (L + R) / 2
//! side = (L - R) / 2
//! mid  *= mid_gain_linear
//! side *= side_gain_linear * width
//! L_out = mid + side
//! R_out = mid - side
//! ```

use crate::common::ParamSmoother;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// StereoWidth
// ---------------------------------------------------------------------------

/// Stereo width processor with M/S encoding, independent mid/side gains.
pub struct StereoWidth {
    sample_rate: u32,

    // Parameters
    width: f64,
    mid_gain_db: f64,
    side_gain_db: f64,
    bypass: bool,

    // Internal state
    width_smoother: ParamSmoother,
    mid_gain_smoother: ParamSmoother,
    side_gain_smoother: ParamSmoother,
}

impl StereoWidth {
    /// Create a new stereo width processor with default parameters.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        Self {
            sample_rate,

            width: 1.0,
            mid_gain_db: 0.0,
            side_gain_db: 0.0,
            bypass: false,

            width_smoother: ParamSmoother::new(1.0, sr, 5.0),
            mid_gain_smoother: ParamSmoother::new(0.0, sr, 5.0),
            side_gain_smoother: ParamSmoother::new(0.0, sr, 5.0),
        }
    }

    /// Convert dB to linear gain.
    #[inline]
    fn db_to_linear(db: f64) -> f64 {
        10.0_f64.powf(db / 20.0)
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for StereoWidth {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Stereo Width",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.width_smoother.reset(self.width);
        self.mid_gain_smoother.reset(self.mid_gain_db);
        self.side_gain_smoother.reset(self.side_gain_db);
    }

    // -- MIDI: no-op for a utility effect --
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
            let w = self.width_smoother.next();
            let mid_gain = Self::db_to_linear(self.mid_gain_smoother.next());
            let side_gain = Self::db_to_linear(self.side_gain_smoother.next());

            let l = in_l[i] as f64;
            let r = in_r[i] as f64;

            // M/S encode
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5;

            // Apply gains and width
            let mid_out = mid * mid_gain;
            let side_out = side * side_gain * w;

            // M/S decode
            out_l[i] = (mid_out + side_out) as f32;
            out_r[i] = (mid_out - side_out) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Stereo width does not have a separate volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: width        (0..2)
    // 1: mid_gain_db  (-24..24)
    // 2: side_gain_db (-24..24)
    // 3: bypass       (0/1, stepped)

    fn param_count(&self) -> u32 {
        4
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Width".into(),
                group: "Stereo".into(),
                min: 0.0,
                max: 2.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Mid Gain".into(),
                group: "Stereo".into(),
                min: -24.0,
                max: 24.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Side Gain".into(),
                group: "Stereo".into(),
                min: -24.0,
                max: 24.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
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
            0 => Some(self.width),
            1 => Some(self.mid_gain_db),
            2 => Some(self.side_gain_db),
            3 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.width = value.clamp(0.0, 2.0);
                self.width_smoother.set_target(self.width);
            }
            1 => {
                self.mid_gain_db = value.clamp(-24.0, 24.0);
                self.mid_gain_smoother.set_target(self.mid_gain_db);
            }
            2 => {
                self.side_gain_db = value.clamp(-24.0, 24.0);
                self.side_gain_smoother.set_target(self.side_gain_db);
            }
            3 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.0}%", value * 100.0)),
            1 => Some(format!("{:.1} dB", value)),
            2 => Some(format!("{:.1} dB", value)),
            3 => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
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
        let mut sw = StereoWidth::new(44100);
        sw.set_param(3, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        sw.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
        let mut sw = StereoWidth::new(44100);

        // width
        sw.set_param(0, 0.5);
        assert_eq!(sw.get_param(0), Some(0.5));

        // mid_gain_db
        sw.set_param(1, 6.0);
        assert_eq!(sw.get_param(1), Some(6.0));

        // side_gain_db
        sw.set_param(2, -12.0);
        assert_eq!(sw.get_param(2), Some(-12.0));

        // bypass
        sw.set_param(3, 1.0);
        assert_eq!(sw.get_param(3), Some(1.0));

        // Clamping
        sw.set_param(0, -1.0);
        assert_eq!(sw.get_param(0), Some(0.0));

        sw.set_param(0, 5.0);
        assert_eq!(sw.get_param(0), Some(2.0));

        sw.set_param(1, -50.0);
        assert_eq!(sw.get_param(1), Some(-24.0));

        sw.set_param(1, 50.0);
        assert_eq!(sw.get_param(1), Some(24.0));

        sw.set_param(2, -50.0);
        assert_eq!(sw.get_param(2), Some(-24.0));

        sw.set_param(2, 50.0);
        assert_eq!(sw.get_param(2), Some(24.0));

        // Invalid param
        assert_eq!(sw.get_param(99), None);
        assert!(sw.param_info(4).is_none());
    }

    #[test]
    fn test_width_zero_mono() {
        let mut sw = StereoWidth::new(44100);
        sw.set_param(0, 0.0); // width = 0 (mono)
        sw.width_smoother.reset(0.0);
        sw.mid_gain_smoother.reset(0.0);
        sw.side_gain_smoother.reset(0.0);

        // Stereo input: L and R are different
        let in_l: Vec<f32> = (0..256).map(|i| (i as f32) * 0.004).collect();
        let in_r: Vec<f32> = (0..256).map(|i| 1.0 - (i as f32) * 0.004).collect();
        let mut out_l = vec![0.0f32; 256];
        let mut out_r = vec![0.0f32; 256];

        sw.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        // With width=0, side is zeroed, so L_out == R_out == mid
        for i in 0..256 {
            let diff = (out_l[i] - out_r[i]).abs();
            assert!(
                diff < 1e-6,
                "width=0: L and R should be equal at sample {} (L={}, R={})",
                i,
                out_l[i],
                out_r[i]
            );
        }
    }

    #[test]
    fn test_width_one_passthrough() {
        let mut sw = StereoWidth::new(44100);
        sw.set_param(0, 1.0); // width = 1
        sw.set_param(1, 0.0); // mid_gain = 0 dB
        sw.set_param(2, 0.0); // side_gain = 0 dB
        sw.width_smoother.reset(1.0);
        sw.mid_gain_smoother.reset(0.0);
        sw.side_gain_smoother.reset(0.0);

        let in_l: Vec<f32> = (0..256)
            .map(|i| ((i as f64 / 44100.0) * 440.0 * std::f64::consts::TAU).sin() as f32)
            .collect();
        let in_r: Vec<f32> = (0..256)
            .map(|i| ((i as f64 / 44100.0) * 880.0 * std::f64::consts::TAU).sin() as f32)
            .collect();
        let mut out_l = vec![0.0f32; 256];
        let mut out_r = vec![0.0f32; 256];

        sw.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        for i in 0..256 {
            let diff_l = (out_l[i] - in_l[i]).abs();
            let diff_r = (out_r[i] - in_r[i]).abs();
            assert!(
                diff_l < 1e-6,
                "width=1: L sample {} differs by {} (in={}, out={})",
                i,
                diff_l,
                in_l[i],
                out_l[i]
            );
            assert!(
                diff_r < 1e-6,
                "width=1: R sample {} differs by {} (in={}, out={})",
                i,
                diff_r,
                in_r[i],
                out_r[i]
            );
        }
    }

    #[test]
    fn test_mid_side_independent() {
        let sr = 44100;

        // Mono signal: L == R, so side content = 0
        let mono_signal = vec![0.5f32; 256];

        // Test 1: Boost mid by +6 dB with a mono signal -> level should increase
        {
            let mut sw = StereoWidth::new(sr);
            sw.set_param(0, 1.0); // width = 1
            sw.set_param(1, 6.0); // mid_gain = +6 dB
            sw.set_param(2, 0.0); // side_gain = 0 dB
            sw.width_smoother.reset(1.0);
            sw.mid_gain_smoother.reset(6.0);
            sw.side_gain_smoother.reset(0.0);

            let mut out_l = vec![0.0f32; 256];
            let mut out_r = vec![0.0f32; 256];

            sw.process_effect(&mono_signal, &mono_signal, &mut out_l, &mut out_r);

            // Mid of a mono signal at 0.5 = (0.5 + 0.5) / 2 = 0.5
            // After +6 dB mid gain: 0.5 * ~1.995 = ~0.998
            let expected = 0.5 * 10.0_f64.powf(6.0 / 20.0);
            for i in 0..256 {
                let diff = (out_l[i] as f64 - expected).abs();
                assert!(
                    diff < 1e-5,
                    "mid boost: sample {} expected ~{:.4}, got {}",
                    i,
                    expected,
                    out_l[i]
                );
            }
        }

        // Test 2: Boost side by +6 dB with a mono signal -> no change
        // (mono signal has zero side content)
        {
            let mut sw = StereoWidth::new(sr);
            sw.set_param(0, 1.0); // width = 1
            sw.set_param(1, 0.0); // mid_gain = 0 dB
            sw.set_param(2, 6.0); // side_gain = +6 dB
            sw.width_smoother.reset(1.0);
            sw.mid_gain_smoother.reset(0.0);
            sw.side_gain_smoother.reset(6.0);

            let mut out_l = vec![0.0f32; 256];
            let mut out_r = vec![0.0f32; 256];

            sw.process_effect(&mono_signal, &mono_signal, &mut out_l, &mut out_r);

            // Side of a mono signal = (0.5 - 0.5) / 2 = 0
            // Boosting zero side content does nothing. Output should equal input.
            for i in 0..256 {
                let diff = (out_l[i] - mono_signal[i]).abs();
                assert!(
                    diff < 1e-6,
                    "side boost on mono: sample {} should be unchanged (in={}, out={})",
                    i,
                    mono_signal[i],
                    out_l[i]
                );
            }
        }
    }

    #[test]
    fn test_param_info_complete() {
        let sw = StereoWidth::new(44100);
        assert_eq!(sw.param_count(), 4);

        for i in 0..4 {
            let info = sw.param_info(i);
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

        // No param beyond 3
        assert!(sw.param_info(4).is_none());
    }
}
