//! Bitcrusher — sample rate reduction + bit depth reduction with dither.
//!
//! Implements lo-fi effects through:
//! - Sample & hold at reduced sample rate (counter-based, supports fractional)
//! - Bit depth quantization with optional TPDF dither
//! - Clock jitter via xorshift64 PRNG
//!
//! Implements the `AudioBackend` trait from `moonlitt-core`.

use crate::common::param_smoother::ParamSmoother;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// xorshift64 PRNG — deterministic, fast, no allocation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x1234_5678_9ABC_DEF0
            } else {
                seed
            },
        }
    }

    /// Generate next pseudo-random u64.
    #[inline]
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Generate a uniform f64 in [0, 1).
    #[inline]
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Generate TPDF noise in [-1, 1]: sum of two uniform [-0.5, 0.5].
    #[inline]
    fn next_tpdf(&mut self) -> f64 {
        let a = self.next_f64() - 0.5;
        let b = self.next_f64() - 0.5;
        a + b
    }
}

// ---------------------------------------------------------------------------
// Bitcrusher
// ---------------------------------------------------------------------------

const SMOOTHING_MS: f64 = 5.0;

/// Bitcrusher effect: sample rate + bit depth reduction with dither and jitter.
pub struct Bitcrusher {
    sample_rate: u32,

    // Parameters
    bit_depth: u32,      // 1..24
    rate_reduction: f64, // 1..100
    dither: f64,         // 0..1
    dry_wet: f64,        // 0..1
    jitter: f64,         // 0..1
    bypass: bool,

    // Parameter smoothers
    smooth_dry_wet: ParamSmoother,

    // Sample & hold state (per channel)
    sh_counter_l: f64,
    sh_counter_r: f64,
    sh_held_l: f64,
    sh_held_r: f64,

    // PRNG for dither and jitter
    rng: Xorshift64,
}

impl Bitcrusher {
    /// Create a new Bitcrusher with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        Self {
            sample_rate,

            bit_depth: 8,
            rate_reduction: 1.0,
            dither: 0.5,
            dry_wet: 1.0,
            jitter: 0.0,
            bypass: false,

            smooth_dry_wet: ParamSmoother::new(1.0, sr, SMOOTHING_MS),

            sh_counter_l: 0.0,
            sh_counter_r: 0.0,
            sh_held_l: 0.0,
            sh_held_r: 0.0,

            rng: Xorshift64::new(0xDEAD_BEEF_CAFE_BABE),
        }
    }

    /// Quantize a sample to the configured bit depth with optional dither.
    #[inline]
    fn quantize(&mut self, sample: f64) -> f64 {
        let levels = (1u64 << (self.bit_depth - 1)) as f64;

        // TPDF dither: scaled to one quantization step * dither amount
        let dither_noise = if self.dither > 0.0 {
            self.rng.next_tpdf() * self.dither / levels
        } else {
            0.0
        };

        let dithered = sample + dither_noise;
        (dithered * levels + 0.5).floor() / levels
    }

    /// Process one sample through sample & hold, returning the held value.
    /// `counter` and `held` are per-channel state.
    #[inline]
    fn sample_and_hold(&mut self, input: f64, counter: &mut f64, held: &mut f64) -> f64 {
        let rate = self.rate_reduction;

        if rate <= 1.0 {
            // No rate reduction — pass through directly
            *held = input;
            return *held;
        }

        // Jitter: random offset to timing
        let jitter_offset = if self.jitter > 0.0 {
            self.rng.next_f64() * self.jitter * 0.5
        } else {
            0.0
        };

        *counter += 1.0 / rate + jitter_offset * (1.0 / rate);

        if *counter >= 1.0 {
            *counter -= 1.0;
            // Prevent counter from drifting too far
            if *counter > 1.0 {
                *counter = 0.0;
            }
            *held = input;
        }

        *held
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Bitcrusher {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Bitcrusher",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.sh_counter_l = 0.0;
        self.sh_counter_r = 0.0;
        self.sh_held_l = 0.0;
        self.sh_held_r = 0.0;
    }

    // MIDI: no-op for an effect
    fn note_on(&mut self, _channel: u8, _note: u8, _velocity: u8) {}
    fn note_off(&mut self, _channel: u8, _note: u8) {}
    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: i16) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn all_notes_off(&mut self) {}

    // Audio: generator render is a no-op (this is an effect)
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
            let dry_wet = self.smooth_dry_wet.next() as f32;

            let l = in_l[i] as f64;
            let r = in_r[i] as f64;

            // Sample & hold at reduced rate
            let mut counter_l = self.sh_counter_l;
            let mut held_l = self.sh_held_l;
            let sh_l = self.sample_and_hold(l, &mut counter_l, &mut held_l);
            self.sh_counter_l = counter_l;
            self.sh_held_l = held_l;

            let mut counter_r = self.sh_counter_r;
            let mut held_r = self.sh_held_r;
            let sh_r = self.sample_and_hold(r, &mut counter_r, &mut held_r);
            self.sh_counter_r = counter_r;
            self.sh_held_r = held_r;

            // Quantize to N bits (with dither)
            let wet_l = self.quantize(sh_l) as f32;
            let wet_r = self.quantize(sh_r) as f32;

            // Dry/wet mix
            out_l[i] = in_l[i] * (1.0 - dry_wet) + wet_l * dry_wet;
            out_r[i] = in_r[i] * (1.0 - dry_wet) + wet_r * dry_wet;
        }
    }

    fn set_volume(&mut self, _volume: f32) {}

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: bit_depth      (1..24, def 8) stepped
    // 1: rate_reduction (1..100, def 1)
    // 2: dither         (0..1, def 0.5)
    // 3: dry_wet        (0..1, def 1)
    // 4: jitter         (0..1, def 0)
    // 5: bypass         (0/1, def 0) stepped

    fn param_count(&self) -> u32 {
        6
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Bit Depth".into(),
                group: "Distortion".into(),
                min: 1.0,
                max: 24.0,
                default: 8.0,
                step_count: 23,
                flags: ParamFlags::STEPPED,
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Rate Reduction".into(),
                group: "Distortion".into(),
                min: 1.0,
                max: 100.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Dither".into(),
                group: "Distortion".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Dry/Wet".into(),
                group: "Distortion".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Jitter".into(),
                group: "Distortion".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
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
            0 => Some(self.bit_depth as f64),
            1 => Some(self.rate_reduction),
            2 => Some(self.dither),
            3 => Some(self.dry_wet),
            4 => Some(self.jitter),
            5 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.bit_depth = (value.round() as u32).clamp(1, 24);
            }
            1 => {
                self.rate_reduction = value.clamp(1.0, 100.0);
            }
            2 => {
                self.dither = value.clamp(0.0, 1.0);
            }
            3 => {
                self.dry_wet = value.clamp(0.0, 1.0);
                self.smooth_dry_wet.set_target(self.dry_wet);
            }
            4 => {
                self.jitter = value.clamp(0.0, 1.0);
            }
            5 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{} bit", value.round() as u32)),
            1 => Some(format!("{:.1}x", value)),
            2 => Some(format!("{:.0}%", value * 100.0)),
            3 => Some(format!("{:.0}%", value * 100.0)),
            4 => Some(format!("{:.0}%", value * 100.0)),
            5 => Some(if value >= 0.5 {
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
    fn bypass_bitexact() {
        let mut bc = Bitcrusher::new(44100);
        bc.set_param(5, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        bc.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
    fn param_round_trip() {
        let mut bc = Bitcrusher::new(44100);

        bc.set_param(0, 16.0);
        assert_eq!(bc.get_param(0), Some(16.0));

        bc.set_param(1, 10.0);
        assert_eq!(bc.get_param(1), Some(10.0));

        bc.set_param(2, 0.75);
        assert_eq!(bc.get_param(2), Some(0.75));

        bc.set_param(3, 0.5);
        assert_eq!(bc.get_param(3), Some(0.5));

        bc.set_param(4, 0.3);
        assert_eq!(bc.get_param(4), Some(0.3));

        bc.set_param(5, 1.0);
        assert_eq!(bc.get_param(5), Some(1.0));

        // Clamping
        bc.set_param(0, 0.0);
        assert_eq!(bc.get_param(0), Some(1.0)); // clamped to min 1

        bc.set_param(0, 50.0);
        assert_eq!(bc.get_param(0), Some(24.0)); // clamped to max 24

        bc.set_param(1, 0.0);
        assert_eq!(bc.get_param(1), Some(1.0)); // clamped to min 1

        // Invalid param
        assert_eq!(bc.get_param(99), None);
        assert!(bc.param_info(6).is_none());
        assert_eq!(bc.param_count(), 6);
    }

    #[test]
    fn depth_24_near_passthrough() {
        // At 24-bit depth with no rate reduction, dither=0, the output
        // should be nearly identical to input (quantization step = 1/2^23).
        let sr = 44100;
        let mut bc = Bitcrusher::new(sr);
        bc.set_param(0, 24.0); // 24 bit
        bc.set_param(1, 1.0); // no rate reduction
        bc.set_param(2, 0.0); // no dither
        bc.set_param(3, 1.0); // 100% wet
        bc.set_param(4, 0.0); // no jitter

        let num_samples = 1024;
        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sr as f64;
                (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32
            })
            .collect();

        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        // Run smoother to settle
        for _ in 0..5 {
            bc.process_effect(&input, &input, &mut out_l, &mut out_r);
        }

        let mut max_err: f32 = 0.0;
        for i in 0..num_samples {
            let err = (out_l[i] - input[i]).abs();
            if err > max_err {
                max_err = err;
            }
        }

        // 24-bit quantization step: 1/2^23 ~ 1.19e-7
        // Allow some margin for f64->f32 conversion
        assert!(
            max_err < 1e-5,
            "24-bit depth should be nearly transparent, max error = {}",
            max_err
        );
    }

    #[test]
    fn rate_1_passthrough() {
        // With rate_reduction=1, no bit crushing (24 bit), dither=0,
        // the output should match input exactly.
        let sr = 44100;
        let mut bc = Bitcrusher::new(sr);
        bc.set_param(0, 24.0); // 24 bit
        bc.set_param(1, 1.0); // no rate reduction
        bc.set_param(2, 0.0); // no dither
        bc.set_param(3, 1.0); // 100% wet
        bc.set_param(4, 0.0); // no jitter

        let num_samples = 1024;
        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sr as f64;
                (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32
            })
            .collect();

        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        // Run multiple times to let dry_wet smoother settle
        for _ in 0..10 {
            bc.process_effect(&input, &input, &mut out_l, &mut out_r);
        }

        let mut max_err: f32 = 0.0;
        for i in 0..num_samples {
            let err = (out_l[i] - input[i]).abs();
            if err > max_err {
                max_err = err;
            }
        }

        assert!(
            max_err < 1e-5,
            "rate=1 should be passthrough, max error = {}",
            max_err
        );
    }

    #[test]
    fn param_info_complete() {
        let bc = Bitcrusher::new(44100);
        for i in 0..bc.param_count() {
            let info = bc.param_info(i);
            assert!(info.is_some(), "param_info({}) should exist", i);
            let info = info.unwrap();
            assert_eq!(info.id, i);
            assert!(!info.name.is_empty());

            let display = bc.param_display(i, info.default);
            assert!(display.is_some(), "param_display({}) should exist", i);
        }
        assert!(bc.param_info(bc.param_count()).is_none());
    }
}
