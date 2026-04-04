//! 4-voice chorus with sinc-interpolated fractional delay.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. Each voice has its own `FractionalDelayLine` and `Lfo`
//! with evenly distributed phase offsets for rich detuning.
//!
//! ## Algorithm
//!
//! ```text
//! For each voice i (0..voices):
//!     lfo_phase_offset = i / voices
//!     modulated_delay  = delay_ms + lfo.next(rate) * depth * delay_ms * 0.5
//!     voice_output     = delay_line[i].read(modulated_delay * sample_rate / 1000)
//!
//! Sum all voices (no per-voice normalization; dry/wet mix controls level)
//! Apply high_cut lowpass filter to wet signal
//! Stereo spread: voice i pans to position based on spread param
//! Output: dry * input + wet * mixed_voices
//! ```

use std::f64::consts::PI;

use super::delay_line::FractionalDelayLine;
use super::lfo::Lfo;
use crate::common::ParamSmoother;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

/// Maximum delay line length in milliseconds.
///
/// Must exceed delay_ms max (30) plus modulation excursion (~15 ms at full
/// depth) with headroom for the sinc kernel.
const MAX_DELAY_MS: f64 = 60.0;

/// Sinc kernel width (8-point Kaiser-windowed).
const SINC_POINTS: usize = 8;

/// Maximum number of chorus voices.
const MAX_VOICES: usize = 4;

/// Smoothing ramp time in milliseconds for parameter changes.
const SMOOTH_MS: f64 = 5.0;

// ---------------------------------------------------------------------------
// HighCutFilter — lightweight biquad for the wet signal lowpass
// ---------------------------------------------------------------------------

/// A minimal 2nd-order IIR filter (Direct Form II Transposed) used
/// exclusively for the high-cut lowpass on the wet signal.
///
/// Inlined here so the chorus module doesn't depend on the `parametric-eq`
/// feature gate.
#[derive(Debug, Clone)]
struct HighCutFilter {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl HighCutFilter {
    /// Design a 2nd-order Butterworth lowpass filter.
    fn design_lowpass(sample_rate: f64, freq: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let q = std::f64::consts::FRAC_1_SQRT_2; // Butterworth Q
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

    /// Process a single sample (Direct Form II Transposed).
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Chorus
// ---------------------------------------------------------------------------

/// 4-voice chorus with sinc-interpolated fractional delay lines.
///
/// Each voice modulates its delay time with an LFO at an evenly distributed
/// phase offset, producing the characteristic thickening / detuning effect.
/// A high-cut lowpass filter tames the wet signal brightness.
pub struct Chorus {
    sample_rate: u32,

    // Parameters
    rate_hz: f64,
    depth: f64,
    delay_ms: f64,
    voices: u32,
    stereo_spread: f64,
    high_cut: f64,
    dry_wet: f64,
    bypass: bool,

    // Per-voice state (always MAX_VOICES allocated; only `voices` are active)
    delay_lines_l: Vec<FractionalDelayLine>,
    delay_lines_r: Vec<FractionalDelayLine>,
    lfos: Vec<Lfo>,

    // High-cut filters (stereo)
    hc_l: HighCutFilter,
    hc_r: HighCutFilter,

    // Smoothers
    rate_smoother: ParamSmoother,
    depth_smoother: ParamSmoother,
    delay_smoother: ParamSmoother,
    spread_smoother: ParamSmoother,
    mix_smoother: ParamSmoother,
}

impl Chorus {
    /// Create a new chorus with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        let mut delay_lines_l = Vec::with_capacity(MAX_VOICES);
        let mut delay_lines_r = Vec::with_capacity(MAX_VOICES);
        let mut lfos = Vec::with_capacity(MAX_VOICES);

        let default_voices = 4u32;

        for i in 0..MAX_VOICES {
            delay_lines_l.push(FractionalDelayLine::new(MAX_DELAY_MS, sample_rate, SINC_POINTS));
            delay_lines_r.push(FractionalDelayLine::new(MAX_DELAY_MS, sample_rate, SINC_POINTS));

            let mut lfo = Lfo::new(sample_rate);
            // Distribute phases evenly across active voices
            lfo.set_phase(i as f64 / default_voices as f64);
            lfos.push(lfo);
        }

        let default_high_cut = 12000.0;

        Self {
            sample_rate,

            rate_hz: 0.8,
            depth: 0.5,
            delay_ms: 12.0,
            voices: default_voices,
            stereo_spread: 0.7,
            high_cut: default_high_cut,
            dry_wet: 0.5,
            bypass: false,

            delay_lines_l,
            delay_lines_r,
            lfos,

            hc_l: HighCutFilter::design_lowpass(sr, default_high_cut),
            hc_r: HighCutFilter::design_lowpass(sr, default_high_cut),

            rate_smoother: ParamSmoother::new(0.8, sr, SMOOTH_MS),
            depth_smoother: ParamSmoother::new(0.5, sr, SMOOTH_MS),
            delay_smoother: ParamSmoother::new(12.0, sr, SMOOTH_MS),
            spread_smoother: ParamSmoother::new(0.7, sr, SMOOTH_MS),
            mix_smoother: ParamSmoother::new(0.5, sr, SMOOTH_MS),
        }
    }

    /// Redistribute LFO phases evenly across the current voice count.
    fn redistribute_phases(&mut self) {
        let v = self.voices as f64;
        for (i, lfo) in self.lfos.iter_mut().enumerate() {
            lfo.set_phase(i as f64 / v);
        }
    }

    /// Compute the stereo pan gains for voice `i` out of `total` voices.
    ///
    /// Returns (left_gain, right_gain) using constant-power panning.
    /// With spread=0 all voices are centred; spread=1 distributes them
    /// across the full stereo field.
    #[inline]
    fn voice_pan(i: usize, total: usize, spread: f64) -> (f64, f64) {
        if total <= 1 {
            return (1.0, 1.0);
        }
        // Position in [-1, 1]: evenly spaced, centred around 0
        let pos = (2.0 * i as f64 / (total - 1) as f64 - 1.0) * spread;
        // Constant-power pan: angle in [0, pi/2]
        let angle = (pos * 0.5 + 0.5) * PI * 0.5;
        (angle.cos(), angle.sin())
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Chorus {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Chorus",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        for dl in &mut self.delay_lines_l {
            dl.clear();
        }
        for dl in &mut self.delay_lines_r {
            dl.clear();
        }
        for lfo in &mut self.lfos {
            lfo.reset_phase();
        }
        self.redistribute_phases();
        self.hc_l.reset();
        self.hc_r.reset();
        self.rate_smoother.reset(self.rate_hz);
        self.depth_smoother.reset(self.depth);
        self.delay_smoother.reset(self.delay_ms);
        self.spread_smoother.reset(self.stereo_spread);
        self.mix_smoother.reset(self.dry_wet);
    }

    // -- MIDI: no-op for a chorus effect --
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
        let num_voices = self.voices as usize;

        for n in 0..len {
            let rate = self.rate_smoother.next();
            let depth = self.depth_smoother.next();
            let base_delay = self.delay_smoother.next();
            let spread = self.spread_smoother.next();
            let mix = self.mix_smoother.next();

            let dry = 1.0 - mix;
            let wet = mix;

            // Write input into all delay lines
            for i in 0..num_voices {
                self.delay_lines_l[i].write(in_l[n]);
                self.delay_lines_r[i].write(in_r[n]);
            }

            // Sum voices with stereo panning
            let mut sum_l = 0.0f64;
            let mut sum_r = 0.0f64;

            for i in 0..num_voices {
                // LFO modulation: [-1, 1] scaled by depth * base_delay * 0.5
                let lfo_val = self.lfos[i].next(rate);
                let modulation = lfo_val * depth * base_delay * 0.5;
                let delay_samples = (base_delay + modulation) * sr / 1000.0;
                // Clamp to valid range
                let delay_samples = delay_samples.max(1.0);

                let voice_l = self.delay_lines_l[i].read(delay_samples) as f64;
                let voice_r = self.delay_lines_r[i].read(delay_samples) as f64;

                let (pan_l, pan_r) = Self::voice_pan(i, num_voices, spread);

                sum_l += voice_l * pan_l;
                sum_r += voice_r * pan_r;
            }

            // High-cut filter on wet signal
            let wet_l = self.hc_l.process(sum_l);
            let wet_r = self.hc_r.process(sum_r);

            // Mix dry + wet
            out_l[n] = (dry * in_l[n] as f64 + wet * wet_l) as f32;
            out_r[n] = (dry * in_r[n] as f64 + wet * wet_r) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Chorus does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: rate_hz       (0.05..5.0)
    // 1: depth         (0..1)
    // 2: delay_ms      (5..30)
    // 3: voices        (1..4, stepped, step_count=3)
    // 4: stereo_spread (0..1)
    // 5: high_cut      (200..20000)
    // 6: dry_wet       (0..1)
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
                min: 0.05,
                max: 5.0,
                default: 0.8,
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
                name: "Delay".into(),
                group: "Modulation".into(),
                min: 5.0,
                max: 30.0,
                default: 12.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Voices".into(),
                group: "Modulation".into(),
                min: 1.0,
                max: 4.0,
                default: 4.0,
                step_count: 3,
                flags: ParamFlags::STEPPED,
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Stereo Spread".into(),
                group: "Stereo".into(),
                min: 0.0,
                max: 1.0,
                default: 0.7,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "High Cut".into(),
                group: "Filter".into(),
                min: 200.0,
                max: 20000.0,
                default: 12000.0,
                step_count: 0,
                flags: ParamFlags::empty(),
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
            _ => None,
        }
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            0 => Some(self.rate_hz),
            1 => Some(self.depth),
            2 => Some(self.delay_ms),
            3 => Some(self.voices as f64),
            4 => Some(self.stereo_spread),
            5 => Some(self.high_cut),
            6 => Some(self.dry_wet),
            7 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.rate_hz = value.clamp(0.05, 5.0);
                self.rate_smoother.set_target(self.rate_hz);
            }
            1 => {
                self.depth = value.clamp(0.0, 1.0);
                self.depth_smoother.set_target(self.depth);
            }
            2 => {
                self.delay_ms = value.clamp(5.0, 30.0);
                self.delay_smoother.set_target(self.delay_ms);
            }
            3 => {
                let v = (value.round() as u32).clamp(1, MAX_VOICES as u32);
                self.voices = v;
                self.redistribute_phases();
            }
            4 => {
                self.stereo_spread = value.clamp(0.0, 1.0);
                self.spread_smoother.set_target(self.stereo_spread);
            }
            5 => {
                self.high_cut = value.clamp(200.0, 20000.0);
                let sr = self.sample_rate as f64;
                self.hc_l = HighCutFilter::design_lowpass(sr, self.high_cut);
                self.hc_r = HighCutFilter::design_lowpass(sr, self.high_cut);
            }
            6 => {
                self.dry_wet = value.clamp(0.0, 1.0);
                self.mix_smoother.set_target(self.dry_wet);
            }
            7 => {
                self.bypass = value >= 0.5;
            }
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.2} Hz", value)),
            1 => Some(format!("{:.0}%", value * 100.0)),
            2 => Some(format!("{:.1} ms", value)),
            3 => Some(format!("{}", value.round() as u32)),
            4 => Some(format!("{:.0}%", value * 100.0)),
            5 => Some(format!("{:.0} Hz", value)),
            6 => Some(format!("{:.0}%", value * 100.0)),
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
        let mut chorus = Chorus::new(44100);
        chorus.set_param(7, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        chorus.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
        let mut chorus = Chorus::new(44100);

        // rate_hz
        chorus.set_param(0, 2.5);
        assert_eq!(chorus.get_param(0), Some(2.5));

        // depth
        chorus.set_param(1, 0.8);
        assert_eq!(chorus.get_param(1), Some(0.8));

        // delay_ms
        chorus.set_param(2, 20.0);
        assert_eq!(chorus.get_param(2), Some(20.0));

        // voices
        chorus.set_param(3, 2.0);
        assert_eq!(chorus.get_param(3), Some(2.0));

        // stereo_spread
        chorus.set_param(4, 0.9);
        assert_eq!(chorus.get_param(4), Some(0.9));

        // high_cut
        chorus.set_param(5, 8000.0);
        assert_eq!(chorus.get_param(5), Some(8000.0));

        // dry_wet
        chorus.set_param(6, 0.75);
        assert_eq!(chorus.get_param(6), Some(0.75));

        // bypass
        chorus.set_param(7, 1.0);
        assert_eq!(chorus.get_param(7), Some(1.0));

        // Clamping
        chorus.set_param(0, -5.0);
        assert_eq!(chorus.get_param(0), Some(0.05));

        chorus.set_param(0, 100.0);
        assert_eq!(chorus.get_param(0), Some(5.0));

        chorus.set_param(1, -1.0);
        assert_eq!(chorus.get_param(1), Some(0.0));

        chorus.set_param(1, 5.0);
        assert_eq!(chorus.get_param(1), Some(1.0));

        chorus.set_param(2, 1.0);
        assert_eq!(chorus.get_param(2), Some(5.0));

        chorus.set_param(2, 100.0);
        assert_eq!(chorus.get_param(2), Some(30.0));

        // Invalid param
        assert_eq!(chorus.get_param(99), None);
        assert!(chorus.param_info(8).is_none());
    }

    #[test]
    fn test_multi_voice_thicker() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        // Generate a 440 Hz sine input
        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                ((i as f64 / sr as f64) * 440.0 * std::f64::consts::TAU).sin() as f32
            })
            .collect();

        // --- 1 voice ---
        let mut chorus_1v = Chorus::new(sr);
        chorus_1v.set_param(3, 1.0); // 1 voice
        chorus_1v.set_param(1, 0.5); // depth
        chorus_1v.set_param(6, 1.0); // 100% wet
        // Jump smoothers
        chorus_1v.depth_smoother.reset(0.5);
        chorus_1v.mix_smoother.reset(1.0);

        let mut out_1v_l = vec![0.0f32; num_samples];
        let mut out_1v_r = vec![0.0f32; num_samples];
        chorus_1v.process_effect(&input, &input, &mut out_1v_l, &mut out_1v_r);

        // --- 4 voices ---
        let mut chorus_4v = Chorus::new(sr);
        chorus_4v.set_param(3, 4.0); // 4 voices
        chorus_4v.set_param(1, 0.5); // depth
        chorus_4v.set_param(6, 1.0); // 100% wet
        // Jump smoothers
        chorus_4v.depth_smoother.reset(0.5);
        chorus_4v.mix_smoother.reset(1.0);

        let mut out_4v_l = vec![0.0f32; num_samples];
        let mut out_4v_r = vec![0.0f32; num_samples];
        chorus_4v.process_effect(&input, &input, &mut out_4v_l, &mut out_4v_r);

        // Compute RMS of the wet signals (skip first 2048 samples for settling)
        let skip = 2048;
        let rms_1v: f64 = out_1v_l[skip..]
            .iter()
            .chain(out_1v_r[skip..].iter())
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / ((num_samples - skip) * 2) as f64;

        let rms_4v: f64 = out_4v_l[skip..]
            .iter()
            .chain(out_4v_r[skip..].iter())
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / ((num_samples - skip) * 2) as f64;

        assert!(
            rms_4v > rms_1v,
            "4 voices should produce more RMS energy than 1 voice: 4v={rms_4v:.6}, 1v={rms_1v:.6}"
        );
    }

    #[test]
    fn test_stereo_spread_distributes() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                ((i as f64 / sr as f64) * 440.0 * std::f64::consts::TAU).sin() as f32
            })
            .collect();

        let mut chorus = Chorus::new(sr);
        chorus.set_param(3, 4.0); // 4 voices
        chorus.set_param(4, 1.0); // spread = 1.0 (full)
        chorus.set_param(6, 1.0); // 100% wet
        chorus.set_param(1, 0.5); // depth
        // Jump smoothers
        chorus.depth_smoother.reset(0.5);
        chorus.mix_smoother.reset(1.0);
        chorus.spread_smoother.reset(1.0);

        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        chorus.process_effect(&input, &input, &mut out_l, &mut out_r);

        // With spread=1, L and R should differ because voices are panned
        // differently. Compute difference energy.
        let skip = 2048;
        let diff_energy: f64 = out_l[skip..]
            .iter()
            .zip(out_r[skip..].iter())
            .map(|(&l, &r)| ((l - r) as f64).powi(2))
            .sum::<f64>()
            / (num_samples - skip) as f64;

        assert!(
            diff_energy > 1e-6,
            "spread=1 should produce L/R differences, got diff_energy={diff_energy:.10}"
        );
    }

    #[test]
    fn test_depth_zero_constant_delay() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                ((i as f64 / sr as f64) * 440.0 * std::f64::consts::TAU).sin() as f32
            })
            .collect();

        let mut chorus = Chorus::new(sr);
        chorus.set_param(1, 0.0); // depth = 0
        chorus.set_param(6, 1.0); // 100% wet
        // Jump smoothers
        chorus.depth_smoother.reset(0.0);
        chorus.mix_smoother.reset(1.0);

        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        chorus.process_effect(&input, &input, &mut out_l, &mut out_r);

        // With depth=0, the delay is constant (no modulation).
        // The output should be a delayed copy of the input with stable energy.
        // Measure RMS in two consecutive windows — they should be similar.
        let skip = 4096; // let the delay lines fill
        let window = 8820; // ~200ms window

        let rms_a: f64 = out_l[skip..skip + window]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / window as f64;

        let rms_b: f64 = out_l[skip + window..skip + 2 * window]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / window as f64;

        let ratio = if rms_a > rms_b {
            rms_a / rms_b
        } else {
            rms_b / rms_a
        };

        assert!(
            ratio < 1.05,
            "depth=0: energy should be stable across windows, ratio={ratio:.4}"
        );
    }

    #[test]
    fn test_param_info_complete() {
        let chorus = Chorus::new(44100);
        assert_eq!(chorus.param_count(), 8);

        for i in 0..8 {
            let info = chorus.param_info(i);
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

        // No param beyond 7
        assert!(chorus.param_info(8).is_none());
    }
}
