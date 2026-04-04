//! Brickwall limiter with lookahead delay and program-dependent auto-release.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as a master bus
//! limiter. Guarantees output never exceeds the ceiling. All internal
//! arithmetic is f64; only the audio I/O boundary touches f32.
//!
//! ## Algorithm
//!
//! 1. Peak-detect the un-delayed input (stereo-linked)
//! 2. Compute gain reduction: `min(0, threshold_db - peak_db)` (infinite ratio)
//! 3. Smooth via dual-envelope auto-release (fast 2ms + slow 100ms) or single
//!    user-set release
//! 4. Apply smoothed gain to the lookahead-delayed signal
//! 5. Hard-clamp at ceiling

use super::envelope::EnvelopeFollower;
use crate::common::param_smoother::ParamSmoother;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Circular delay buffer (stereo)
// ---------------------------------------------------------------------------

/// Stereo circular delay buffer for lookahead.
struct LookaheadBuffer {
    buf_l: Vec<f64>,
    buf_r: Vec<f64>,
    write_pos: usize,
    len: usize,
}

impl LookaheadBuffer {
    fn new(delay_samples: usize) -> Self {
        let len = delay_samples.max(1);
        Self {
            buf_l: vec![0.0; len],
            buf_r: vec![0.0; len],
            write_pos: 0,
            len,
        }
    }

    /// Push a new stereo sample pair and return the delayed pair.
    #[inline]
    fn process(&mut self, l: f64, r: f64) -> (f64, f64) {
        let read_pos = self.write_pos;
        let out_l = self.buf_l[read_pos];
        let out_r = self.buf_r[read_pos];
        self.buf_l[self.write_pos] = l;
        self.buf_r[self.write_pos] = r;
        self.write_pos += 1;
        if self.write_pos >= self.len {
            self.write_pos = 0;
        }
        (out_l, out_r)
    }

    fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.write_pos = 0;
    }

    /// Resize the buffer (new delay length in samples). Resets contents.
    fn resize(&mut self, delay_samples: usize) {
        let len = delay_samples.max(1);
        self.buf_l = vec![0.0; len];
        self.buf_r = vec![0.0; len];
        self.write_pos = 0;
        self.len = len;
    }
}

// ---------------------------------------------------------------------------
// Limiter
// ---------------------------------------------------------------------------

/// Brickwall limiter with lookahead delay and program-dependent auto-release.
///
/// 8 parameters: threshold, ceiling, release, lookahead, attack,
/// oversampling (placeholder), auto_release, bypass.
pub struct Limiter {
    sample_rate: u32,

    // Parameters
    threshold_db: f64,
    ceiling_db: f64,
    release_ms: f64,
    lookahead_ms: f64,
    attack_ms: f64,
    oversampling: u32,
    auto_release: bool,
    bypass: bool,

    // Parameter smoothers
    threshold_smoother: ParamSmoother,
    ceiling_smoother: ParamSmoother,

    // Internal state
    lookahead: LookaheadBuffer,

    // Auto-release: dual envelope followers
    fast_env: EnvelopeFollower,
    slow_env: EnvelopeFollower,

    // Single envelope for manual release mode
    manual_env: EnvelopeFollower,
}

/// Compute lookahead delay in samples from milliseconds and sample rate.
fn lookahead_samples(ms: f64, sample_rate: u32) -> usize {
    (ms * sample_rate as f64 / 1000.0) as usize
}

impl Limiter {
    /// Create a new limiter with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        let delay = lookahead_samples(1.0, sample_rate);

        let mut fast_env = EnvelopeFollower::new(sr);
        fast_env.set_attack_ms(0.1);
        fast_env.set_release_ms(2.0);

        let mut slow_env = EnvelopeFollower::new(sr);
        slow_env.set_attack_ms(0.1);
        slow_env.set_release_ms(100.0);

        let mut manual_env = EnvelopeFollower::new(sr);
        manual_env.set_attack_ms(0.1);
        manual_env.set_release_ms(100.0);

        Self {
            sample_rate,

            threshold_db: -1.0,
            ceiling_db: -0.3,
            release_ms: 100.0,
            lookahead_ms: 1.0,
            attack_ms: 0.1,
            oversampling: 1,
            auto_release: true,
            bypass: false,

            threshold_smoother: ParamSmoother::new(-1.0, sr, 5.0),
            ceiling_smoother: ParamSmoother::new(-0.3, sr, 5.0),

            lookahead: LookaheadBuffer::new(delay),

            fast_env,
            slow_env,
            manual_env,
        }
    }

    /// Update the attack time on all envelope followers.
    fn update_attack(&mut self) {
        self.fast_env.set_attack_ms(self.attack_ms);
        self.slow_env.set_attack_ms(self.attack_ms);
        self.manual_env.set_attack_ms(self.attack_ms);
    }

    /// Update the manual release envelope coefficient.
    fn update_manual_release(&mut self) {
        self.manual_env.set_release_ms(self.release_ms);
    }

    /// Resize the lookahead buffer when lookahead_ms changes.
    fn update_lookahead(&mut self) {
        let delay = lookahead_samples(self.lookahead_ms, self.sample_rate);
        self.lookahead.resize(delay);
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Limiter {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Brickwall Limiter",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.lookahead.reset();
        self.fast_env.reset();
        self.slow_env.reset();
        self.manual_env.reset();
        self.threshold_smoother.reset(self.threshold_db);
        self.ceiling_smoother.reset(self.ceiling_db);
    }

    // -- MIDI: no-op for a limiter effect --
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

        for i in 0..len {
            let l = in_l[i] as f64;
            let r = in_r[i] as f64;

            // Smoothed parameters
            let threshold = self.threshold_smoother.next();
            let ceiling = self.ceiling_smoother.next();
            let ceiling_linear = 10.0_f64.powf(ceiling / 20.0);

            // 1. Peak detection on un-delayed input (stereo-linked)
            let peak = l.abs().max(r.abs());
            let peak_db = if peak > 1e-6 {
                20.0 * peak.log10()
            } else {
                -120.0
            };

            // 2. Gain computation (infinite ratio = brickwall)
            let gain_reduction_db = (threshold - peak_db).min(0.0);

            // 3. Envelope smoothing
            let gr_magnitude = -gain_reduction_db; // positive value

            let smoothed_gr = if self.auto_release {
                // Dual-envelope auto-release
                let fast_level = self.fast_env.process(gr_magnitude);
                let slow_level = self.slow_env.process(gr_magnitude);

                // Blend based on relative envelope levels
                let blend = fast_level / (fast_level + slow_level + 1e-10);
                fast_level * blend + slow_level * (1.0 - blend)
            } else {
                // Single envelope with user-set release
                self.manual_env.process(gr_magnitude)
            };

            let final_gain_db = -smoothed_gr;
            let gain_linear = 10.0_f64.powf(final_gain_db / 20.0);

            // 4. Apply gain to lookahead-delayed signal
            let (delayed_l, delayed_r) = self.lookahead.process(l, r);
            let limited_l = delayed_l * gain_linear;
            let limited_r = delayed_r * gain_linear;

            // 5. Hard-clamp at ceiling
            out_l[i] = (limited_l.clamp(-ceiling_linear, ceiling_linear)) as f32;
            out_r[i] = (limited_r.clamp(-ceiling_linear, ceiling_linear)) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Limiter does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        lookahead_samples(self.lookahead_ms, self.sample_rate) as u32
    }

    // -- Parameters --
    // 0: threshold_db   (-30..0,   default -1.0)
    // 1: ceiling_db     (-30..0,   default -0.3)
    // 2: release_ms     (10..1000, default 100)
    // 3: lookahead_ms   (0.5..5.0, default 1.0)
    // 4: attack_ms      (0.01..5.0, default 0.1)
    // 5: oversampling   (1, 2, 4,  default 1) — placeholder
    // 6: auto_release   (0/1,      default 1)
    // 7: bypass         (0/1,      default 0)

    fn param_count(&self) -> u32 {
        8
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Threshold".into(),
                group: "Dynamics".into(),
                min: -30.0,
                max: 0.0,
                default: -1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Ceiling".into(),
                group: "Dynamics".into(),
                min: -30.0,
                max: 0.0,
                default: -0.3,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Release".into(),
                group: "Dynamics".into(),
                min: 10.0,
                max: 1000.0,
                default: 100.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Lookahead".into(),
                group: "Dynamics".into(),
                min: 0.5,
                max: 5.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Attack".into(),
                group: "Dynamics".into(),
                min: 0.01,
                max: 5.0,
                default: 0.1,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Oversampling".into(),
                group: "Dynamics".into(),
                min: 1.0,
                max: 4.0,
                default: 1.0,
                step_count: 2,
                flags: ParamFlags::STEPPED,
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Auto Release".into(),
                group: "Dynamics".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
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
            0 => Some(self.threshold_db),
            1 => Some(self.ceiling_db),
            2 => Some(self.release_ms),
            3 => Some(self.lookahead_ms),
            4 => Some(self.attack_ms),
            5 => Some(self.oversampling as f64),
            6 => Some(if self.auto_release { 1.0 } else { 0.0 }),
            7 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.threshold_db = value.clamp(-30.0, 0.0);
                self.threshold_smoother.set_target(self.threshold_db);
            }
            1 => {
                self.ceiling_db = value.clamp(-30.0, 0.0);
                self.ceiling_smoother.set_target(self.ceiling_db);
            }
            2 => {
                self.release_ms = value.clamp(10.0, 1000.0);
                self.update_manual_release();
            }
            3 => {
                self.lookahead_ms = value.clamp(0.5, 5.0);
                self.update_lookahead();
            }
            4 => {
                self.attack_ms = value.clamp(0.01, 5.0);
                self.update_attack();
            }
            5 => {
                // Stepped: 1, 2, or 4
                let v = value.round() as u32;
                self.oversampling = if v <= 1 {
                    1
                } else if v <= 2 {
                    2
                } else {
                    4
                };
            }
            6 => self.auto_release = value >= 0.5,
            7 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.1} dB", value)),
            1 => Some(format!("{:.1} dB", value)),
            2 => Some(format!("{:.0} ms", value)),
            3 => Some(format!("{:.1} ms", value)),
            4 => Some(format!("{:.2} ms", value)),
            5 => Some(format!("{}x", value as u32)),
            6 => Some(if value >= 0.5 { "On".into() } else { "Off".into() }),
            7 => Some(if value >= 0.5 { "On".into() } else { "Off".into() }),
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
    use moonlitt_core::AudioBackend;

    #[test]
    fn test_bypass_is_bitexact() {
        let mut lim = Limiter::new(44100);
        lim.set_param(7, 1.0); // bypass on
        let input_l: Vec<f32> = (0..512).map(|i| (i as f32 * 0.01).sin()).collect();
        let input_r = input_l.clone();
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];
        lim.process_effect(&input_l, &input_r, &mut out_l, &mut out_r);
        assert_eq!(input_l, out_l);
        assert_eq!(input_r, out_r);
    }

    #[test]
    fn test_param_round_trip() {
        let mut lim = Limiter::new(44100);
        for id in 0..lim.param_count() {
            let info = lim.param_info(id).unwrap();
            lim.set_param(id, info.default);
            let val = lim.get_param(id).unwrap();
            assert!(
                (val - info.default).abs() < 0.01,
                "Param {id} ({}) round-trip failed: {val} != {}",
                info.name,
                info.default
            );
        }
    }

    #[test]
    fn test_peak_never_exceeds_ceiling() {
        let mut lim = Limiter::new(44100);
        lim.set_param(0, -6.0); // threshold = -6dB
        lim.set_param(1, -1.0); // ceiling = -1dB
        let ceiling_linear = 10.0_f32.powf(-1.0 / 20.0);

        // Feed loud signal (amplitude 2.0 = +6dB)
        let len = 4410; // 100ms
        let input: Vec<f32> = (0..len)
            .map(|i| 2.0 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44100.0).sin())
            .collect();
        let mut out_l = vec![0.0f32; len];
        let mut out_r = vec![0.0f32; len];

        // Process multiple blocks to let limiter settle
        for _ in 0..10 {
            lim.process_effect(&input, &input, &mut out_l, &mut out_r);
        }
        // After settling, output should not exceed ceiling
        lim.process_effect(&input, &input, &mut out_l, &mut out_r);
        let max_peak = out_l.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(
            max_peak <= ceiling_linear + 0.01,
            "Peak {max_peak} exceeds ceiling {ceiling_linear}"
        );
    }

    #[test]
    fn test_latency_correct() {
        let lim = Limiter::new(44100);
        // Default lookahead = 1.0ms at 44100Hz = 44 samples
        let expected = (1.0 * 44100.0 / 1000.0) as u32;
        assert_eq!(lim.latency(), expected);
    }

    #[test]
    fn test_param_info_complete() {
        let lim = Limiter::new(44100);
        assert_eq!(lim.param_count(), 8);
        for i in 0..8 {
            assert!(lim.param_info(i).is_some(), "Missing param_info for id {i}");
        }
        assert!(lim.param_info(8).is_none());
    }

    #[test]
    fn test_info() {
        let lim = Limiter::new(44100);
        let info = lim.info();
        assert_eq!(info.backend_type, moonlitt_core::BackendType::PluginHost);
    }
}
