//! Gain utility with polarity invert and mono sum.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. Uses `ParamSmoother` for zipper-free gain changes.
//!
//! ## Algorithm
//!
//! ```text
//! gain_linear = 10^(gain_db / 20), clamp gain_db at -120 minimum
//! polarity_mult = if polarity { -1.0 } else { 1.0 }
//! output = input * gain_linear * polarity_mult
//! if mono: output_L = output_R = (L + R) / 2
//! ```

use crate::common::ParamSmoother;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Gain
// ---------------------------------------------------------------------------

/// Gain utility with smoothed gain, polarity inversion, and mono summing.
pub struct Gain {
    sample_rate: u32,

    // Parameters
    gain_db: f64,
    polarity: bool,
    mono: bool,
    bypass: bool,

    // Internal state
    gain_smoother: ParamSmoother,
}

impl Gain {
    /// Create a new gain utility with default parameters.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        Self {
            sample_rate,

            gain_db: 0.0,
            polarity: false,
            mono: false,
            bypass: false,

            gain_smoother: ParamSmoother::new(0.0, sr, 5.0),
        }
    }

    /// Convert dB to linear gain, treating <= -120 dB as silence.
    #[inline]
    fn db_to_linear(db: f64) -> f64 {
        if db <= -120.0 {
            0.0
        } else {
            10.0_f64.powf(db / 20.0)
        }
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Gain {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Gain",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.gain_smoother.reset(self.gain_db);
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

        let polarity_mult: f64 = if self.polarity { -1.0 } else { 1.0 };

        for i in 0..len {
            let smoothed_db = self.gain_smoother.next();
            let gain_linear = Self::db_to_linear(smoothed_db) * polarity_mult;

            let l = in_l[i] as f64;
            let r = in_r[i] as f64;

            if self.mono {
                let mid = (l + r) * 0.5;
                let out = (mid * gain_linear) as f32;
                out_l[i] = out;
                out_r[i] = out;
            } else {
                out_l[i] = (l * gain_linear) as f32;
                out_r[i] = (r * gain_linear) as f32;
            }
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Gain utility does not have a separate volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: gain_db   (-120..24)
    // 1: polarity  (0/1, stepped)
    // 2: mono      (0/1, stepped)
    // 3: bypass    (0/1, stepped)

    fn param_count(&self) -> u32 {
        4
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Gain".into(),
                group: "Utility".into(),
                min: -120.0,
                max: 24.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Polarity".into(),
                group: "Utility".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Mono".into(),
                group: "Utility".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
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
            0 => Some(self.gain_db),
            1 => Some(if self.polarity { 1.0 } else { 0.0 }),
            2 => Some(if self.mono { 1.0 } else { 0.0 }),
            3 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.gain_db = value.clamp(-120.0, 24.0);
                self.gain_smoother.set_target(self.gain_db);
            }
            1 => self.polarity = value >= 0.5,
            2 => self.mono = value >= 0.5,
            3 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => {
                if value <= -120.0 {
                    Some("-inf".into())
                } else {
                    Some(format!("{:.1} dB", value))
                }
            }
            1 => Some(if value >= 0.5 { "Invert" } else { "Normal" }.into()),
            2 => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
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
        let mut gain = Gain::new(44100);
        gain.set_param(3, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        gain.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
        let mut gain = Gain::new(44100);

        // gain_db
        gain.set_param(0, -6.0);
        assert_eq!(gain.get_param(0), Some(-6.0));

        // polarity
        gain.set_param(1, 1.0);
        assert_eq!(gain.get_param(1), Some(1.0));

        // mono
        gain.set_param(2, 1.0);
        assert_eq!(gain.get_param(2), Some(1.0));

        // bypass
        gain.set_param(3, 1.0);
        assert_eq!(gain.get_param(3), Some(1.0));

        // Clamping
        gain.set_param(0, -200.0);
        assert_eq!(gain.get_param(0), Some(-120.0));

        gain.set_param(0, 50.0);
        assert_eq!(gain.get_param(0), Some(24.0));

        // Invalid param
        assert_eq!(gain.get_param(99), None);
        assert!(gain.param_info(4).is_none());
    }

    #[test]
    fn test_gain_maps_correctly() {
        let sr = 44100;

        // 0 dB = unity (output == input)
        {
            let mut g = Gain::new(sr);
            g.set_param(0, 0.0);
            g.gain_smoother.reset(0.0);

            let input = vec![0.5f32; 256];
            let mut out_l = vec![0.0f32; 256];
            let mut out_r = vec![0.0f32; 256];

            g.process_effect(&input, &input, &mut out_l, &mut out_r);

            for i in 0..256 {
                let diff = (out_l[i] - input[i]).abs();
                assert!(
                    diff < 1e-6,
                    "0dB: sample {} differs by {} (in={}, out={})",
                    i,
                    diff,
                    input[i],
                    out_l[i]
                );
            }
        }

        // +6 dB ~ 2x amplitude
        {
            let mut g = Gain::new(sr);
            g.set_param(0, 6.0);
            g.gain_smoother.reset(6.0);

            let input = vec![0.25f32; 256];
            let mut out_l = vec![0.0f32; 256];
            let mut out_r = vec![0.0f32; 256];

            g.process_effect(&input, &input, &mut out_l, &mut out_r);

            // 10^(6/20) = 1.99526...
            let expected = 0.25 * 10.0_f64.powf(6.0 / 20.0);
            for i in 0..256 {
                let diff = (out_l[i] as f64 - expected).abs();
                assert!(
                    diff < 1e-5,
                    "+6dB: sample {} differs by {} (expected={}, out={})",
                    i,
                    diff,
                    expected,
                    out_l[i]
                );
            }
        }

        // -6 dB ~ 0.5x amplitude
        {
            let mut g = Gain::new(sr);
            g.set_param(0, -6.0);
            g.gain_smoother.reset(-6.0);

            let input = vec![0.5f32; 256];
            let mut out_l = vec![0.0f32; 256];
            let mut out_r = vec![0.0f32; 256];

            g.process_effect(&input, &input, &mut out_l, &mut out_r);

            // 10^(-6/20) = 0.50119...
            let expected = 0.5 * 10.0_f64.powf(-6.0 / 20.0);
            for i in 0..256 {
                let diff = (out_l[i] as f64 - expected).abs();
                assert!(
                    diff < 1e-5,
                    "-6dB: sample {} differs by {} (expected={}, out={})",
                    i,
                    diff,
                    expected,
                    out_l[i]
                );
            }
        }
    }

    #[test]
    fn test_polarity_inverts() {
        let mut g = Gain::new(44100);
        g.set_param(0, 0.0); // 0 dB
        g.set_param(1, 1.0); // polarity invert
        g.gain_smoother.reset(0.0);

        let input = vec![0.5f32; 256];
        let mut out_l = vec![0.0f32; 256];
        let mut out_r = vec![0.0f32; 256];

        g.process_effect(&input, &input, &mut out_l, &mut out_r);

        for i in 0..256 {
            let diff = (out_l[i] + input[i]).abs(); // should sum to zero
            assert!(
                diff < 1e-6,
                "polarity: sample {} not inverted (in={}, out={})",
                i,
                input[i],
                out_l[i]
            );
        }
    }

    #[test]
    fn test_mono_sums_channels() {
        let mut g = Gain::new(44100);
        g.set_param(0, 0.0); // 0 dB
        g.set_param(2, 1.0); // mono on
        g.gain_smoother.reset(0.0);

        let in_l = vec![1.0f32; 256];
        let in_r = vec![0.0f32; 256];
        let mut out_l = vec![0.0f32; 256];
        let mut out_r = vec![0.0f32; 256];

        g.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        for i in 0..256 {
            let diff_l = (out_l[i] - 0.5).abs();
            let diff_r = (out_r[i] - 0.5).abs();
            assert!(
                diff_l < 1e-6,
                "mono L: sample {} expected 0.5, got {}",
                i,
                out_l[i]
            );
            assert!(
                diff_r < 1e-6,
                "mono R: sample {} expected 0.5, got {}",
                i,
                out_r[i]
            );
            // Both channels should be identical
            assert_eq!(
                out_l[i].to_bits(),
                out_r[i].to_bits(),
                "mono: L and R should be bit-identical at sample {}",
                i
            );
        }
    }

    #[test]
    fn test_param_info_complete() {
        let g = Gain::new(44100);
        assert_eq!(g.param_count(), 4);

        for i in 0..4 {
            let info = g.param_info(i);
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
        assert!(g.param_info(4).is_none());
    }
}
