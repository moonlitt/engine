//! Split-band sibilance reduction (de-esser) with listen mode.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. All internal arithmetic is f64; only the audio I/O
//! boundary touches f32.
//!
//! ## Algorithm
//!
//! A bandpass filter isolates the sibilance band for level detection.
//! When the detected level exceeds the threshold, gain reduction is
//! applied using a compressor-style hard-knee formula.
//!
//! Two modes are supported:
//! - **Wideband** (mode=0): gain reduction is applied to the full input.
//! - **Split-band** (mode=1): only the bandpass-isolated component is
//!   attenuated, leaving the rest of the signal untouched.
//!
//! **Listen mode** routes only the bandpass-filtered signal to the output,
//! letting the user hear exactly what the detector is seeing for tuning
//! frequency and Q.

use super::envelope::EnvelopeFollower;
use crate::eq::biquad::{Biquad, BiquadCoeffs};
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Bandpass coefficient design (Audio EQ Cookbook)
// ---------------------------------------------------------------------------

/// Design a second-order bandpass filter (constant 0-dB peak gain).
///
/// Uses the BPF formula from the Robert Bristow-Johnson Audio EQ Cookbook:
/// ```text
/// b0 =  sin(w0) / 2  =  Q * alpha
/// b1 =  0
/// b2 = -sin(w0) / 2  = -Q * alpha
/// a0 =  1 + alpha
/// a1 = -2 * cos(w0)
/// a2 =  1 - alpha
/// ```
/// where `alpha = sin(w0) / (2*Q)`.
fn design_bandpass(sample_rate: f64, freq: f64, q: f64) -> BiquadCoeffs {
    let w0 = 2.0 * PI * freq / sample_rate;
    let sin_w0 = w0.sin();
    let cos_w0 = w0.cos();
    let alpha = sin_w0 / (2.0 * q);

    let b0 = alpha;
    let b1 = 0.0;
    let b2 = -alpha;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;

    let inv_a0 = 1.0 / a0;
    BiquadCoeffs {
        b0: b0 * inv_a0,
        b1: b1 * inv_a0,
        b2: b2 * inv_a0,
        a1: a1 * inv_a0,
        a2: a2 * inv_a0,
    }
}

// ---------------------------------------------------------------------------
// De-esser
// ---------------------------------------------------------------------------

/// Split-band sibilance reduction effect with listen mode for tuning.
pub struct DeEsser {
    sample_rate: u32,

    // Parameters
    threshold_db: f64,
    frequency: f64,
    bandwidth_q: f64,
    ratio: f64,
    mode: u32,        // 0 = wideband, 1 = split-band
    listen_mode: bool, // output bandpass signal only
    bypass: bool,

    // Internal state — stereo (L/R)
    detector_bpf: [Biquad; 2],
    envelope: [EnvelopeFollower; 2],
}

impl DeEsser {
    /// Create a new de-esser with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        let mut ds = Self {
            sample_rate,

            threshold_db: -20.0,
            frequency: 6000.0,
            bandwidth_q: 2.0,
            ratio: 4.0,
            mode: 1,
            listen_mode: false,
            bypass: false,

            detector_bpf: [Biquad::new(), Biquad::new()],
            envelope: [EnvelopeFollower::new(sr), EnvelopeFollower::new(sr)],
        };

        ds.update_bpf();
        ds.update_envelope_coeffs();
        ds
    }

    /// Recompute bandpass filter coefficients from current parameters.
    fn update_bpf(&mut self) {
        let sr = self.sample_rate as f64;
        let coeffs = design_bandpass(sr, self.frequency, self.bandwidth_q);
        self.detector_bpf[0].set_coeffs(coeffs);
        self.detector_bpf[1].set_coeffs(coeffs);
    }

    /// Configure envelope follower for fast sibilance tracking.
    fn update_envelope_coeffs(&mut self) {
        // De-essing needs very fast attack (~0.5ms) and moderate release (~10ms).
        for env in &mut self.envelope {
            env.set_attack_ms(0.5);
            env.set_release_ms(10.0);
        }
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for DeEsser {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "De-esser",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.detector_bpf[0].reset();
        self.detector_bpf[1].reset();
        self.envelope[0].reset();
        self.envelope[1].reset();
    }

    // -- MIDI: no-op for a de-esser effect --
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

            // Bandpass filter for detection
            let band_l = self.detector_bpf[0].process(l);
            let band_r = self.detector_bpf[1].process(r);

            // Listen mode: output only the bandpass signal
            if self.listen_mode {
                out_l[i] = band_l as f32;
                out_r[i] = band_r as f32;
                continue;
            }

            // Detect level via envelope follower
            let level_l = self.envelope[0].process(band_l.abs());
            let level_r = self.envelope[1].process(band_r.abs());

            // Compute gain reduction for each channel
            let gr_l = compute_gain_reduction(level_l, self.threshold_db, self.ratio);
            let gr_r = compute_gain_reduction(level_r, self.threshold_db, self.ratio);

            // Apply gain reduction based on mode
            if self.mode == 0 {
                // Wideband: attenuate the entire signal
                out_l[i] = (l * gr_l) as f32;
                out_r[i] = (r * gr_r) as f32;
            } else {
                // Split-band: attenuate only the sibilance band
                let rest_l = l - band_l;
                let rest_r = r - band_r;
                out_l[i] = (rest_l + band_l * gr_l) as f32;
                out_r[i] = (rest_r + band_r * gr_r) as f32;
            }
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // De-esser does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: threshold_db  (-40..0, default -20)
    // 1: frequency     (2000..12000, default 6000)
    // 2: bandwidth_q   (0.5..8, default 2.0)
    // 3: ratio         (1..20, default 4.0)
    // 4: mode          (0=Wideband, 1=Split-band)
    // 5: listen_mode   (0=Off, 1=On)
    // 6: bypass        (0=Off, 1=On)

    fn param_count(&self) -> u32 {
        7
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Threshold".into(),
                group: "De-esser".into(),
                min: -40.0,
                max: 0.0,
                default: -20.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Frequency".into(),
                group: "Detection".into(),
                min: 2000.0,
                max: 12000.0,
                default: 6000.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Bandwidth Q".into(),
                group: "Detection".into(),
                min: 0.5,
                max: 8.0,
                default: 2.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Ratio".into(),
                group: "De-esser".into(),
                min: 1.0,
                max: 20.0,
                default: 4.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Mode".into(),
                group: "De-esser".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Listen Mode".into(),
                group: "De-esser".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Bypass".into(),
                group: "De-esser".into(),
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
            1 => Some(self.frequency),
            2 => Some(self.bandwidth_q),
            3 => Some(self.ratio),
            4 => Some(self.mode as f64),
            5 => Some(if self.listen_mode { 1.0 } else { 0.0 }),
            6 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => self.threshold_db = value.clamp(-40.0, 0.0),
            1 => {
                self.frequency = value.clamp(2000.0, 12000.0);
                self.update_bpf();
            }
            2 => {
                self.bandwidth_q = value.clamp(0.5, 8.0);
                self.update_bpf();
            }
            3 => self.ratio = value.clamp(1.0, 20.0),
            4 => self.mode = if value >= 0.5 { 1 } else { 0 },
            5 => self.listen_mode = value >= 0.5,
            6 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.1} dB", value)),
            1 => Some(format!("{:.0} Hz", value)),
            2 => Some(format!("{:.1}", value)),
            3 => Some(format!("{:.1}:1", value)),
            4 => Some(
                if value >= 0.5 {
                    "Split-band"
                } else {
                    "Wideband"
                }
                .into(),
            ),
            5 => Some(if value >= 0.5 { "On".into() } else { "Off".into() }),
            6 => Some(if value >= 0.5 { "On".into() } else { "Off".into() }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Gain reduction — free function
// ---------------------------------------------------------------------------

/// Compute linear gain reduction from an envelope level.
///
/// Uses a compressor-style hard-knee formula:
/// ```text
/// if level_db > threshold_db:
///     gain_reduction_db = (threshold_db - level_db) * (1 - 1/ratio)
/// else:
///     gain_reduction_db = 0
/// ```
/// Returns the linear gain multiplier (always <= 1.0).
#[inline]
fn compute_gain_reduction(level: f64, threshold_db: f64, ratio: f64) -> f64 {
    // Convert envelope level to dB (floor at -120 dB to avoid -inf)
    let level_db = if level > 1e-6 {
        20.0 * level.log10()
    } else {
        -120.0
    };

    if level_db > threshold_db {
        let gr_db = (threshold_db - level_db) * (1.0 - 1.0 / ratio);
        10.0_f64.powf(gr_db / 20.0)
    } else {
        1.0
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
        let mut ds = DeEsser::new(44100);
        ds.set_param(6, 1.0); // bypass
        let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.1).sin()).collect();
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];
        ds.process_effect(&input, &input, &mut out_l, &mut out_r);
        assert_eq!(input, out_l);
    }

    #[test]
    fn test_param_round_trip() {
        let mut ds = DeEsser::new(44100);
        for id in 0..ds.param_count() {
            let info = ds.param_info(id).unwrap();
            ds.set_param(id, info.default);
            let val = ds.get_param(id).unwrap();
            assert!(
                (val - info.default).abs() < 0.01,
                "Param {id} round-trip failed"
            );
        }
    }

    #[test]
    fn test_sibilance_attenuated() {
        let mut ds = DeEsser::new(44100);
        ds.set_param(0, -20.0); // threshold
        ds.set_param(1, 6000.0); // frequency
        ds.set_param(3, 10.0); // ratio

        // Feed loud 6kHz sine (in the sibilance band)
        let len = 4410;
        let input: Vec<f32> = (0..len)
            .map(|i| {
                0.5 * (2.0 * std::f32::consts::PI * 6000.0 * i as f32 / 44100.0).sin()
            })
            .collect();
        let mut out_l = vec![0.0f32; len];
        let mut out_r = vec![0.0f32; len];

        // Let it settle
        for _ in 0..5 {
            ds.process_effect(&input, &input, &mut out_l, &mut out_r);
        }
        ds.process_effect(&input, &input, &mut out_l, &mut out_r);

        let input_rms: f32 = (input.iter().map(|s| s * s).sum::<f32>() / len as f32).sqrt();
        let out_rms: f32 = (out_l.iter().map(|s| s * s).sum::<f32>() / len as f32).sqrt();
        assert!(
            out_rms < input_rms * 0.7,
            "6kHz should be attenuated: in_rms={input_rms}, out_rms={out_rms}"
        );
    }

    #[test]
    fn test_non_sibilant_passes() {
        let mut ds = DeEsser::new(44100);
        ds.set_param(0, -20.0);
        ds.set_param(1, 6000.0);
        ds.set_param(4, 1.0); // split-band mode

        // Feed 200Hz sine (far below sibilance band)
        let len = 4410;
        let input: Vec<f32> = (0..len)
            .map(|i| {
                0.3 * (2.0 * std::f32::consts::PI * 200.0 * i as f32 / 44100.0).sin()
            })
            .collect();
        let mut out_l = vec![0.0f32; len];
        let mut out_r = vec![0.0f32; len];

        for _ in 0..5 {
            ds.process_effect(&input, &input, &mut out_l, &mut out_r);
        }
        ds.process_effect(&input, &input, &mut out_l, &mut out_r);

        let input_rms: f32 = (input.iter().map(|s| s * s).sum::<f32>() / len as f32).sqrt();
        let out_rms: f32 = (out_l.iter().map(|s| s * s).sum::<f32>() / len as f32).sqrt();
        assert!(
            (out_rms - input_rms).abs() < input_rms * 0.1,
            "200Hz should pass in split-band: in={input_rms}, out={out_rms}"
        );
    }

    #[test]
    fn test_listen_mode() {
        let mut ds = DeEsser::new(44100);
        ds.set_param(5, 1.0); // listen mode on
        ds.set_param(1, 6000.0);

        // Feed white-ish noise (mix of frequencies)
        let len = 4410;
        let input: Vec<f32> = (0..len)
            .map(|i| {
                0.3 * (2.0 * std::f32::consts::PI * 200.0 * i as f32 / 44100.0).sin()
                    + 0.3 * (2.0 * std::f32::consts::PI * 6000.0 * i as f32 / 44100.0).sin()
            })
            .collect();
        let mut out_l = vec![0.0f32; len];
        let mut out_r = vec![0.0f32; len];

        for _ in 0..3 {
            ds.process_effect(&input, &input, &mut out_l, &mut out_r);
        }
        // Listen mode should output only the bandpass signal, not the full mix
        // The output should have less energy than input (only the 6kHz component)
        let input_rms: f32 = (input.iter().map(|s| s * s).sum::<f32>() / len as f32).sqrt();
        let out_rms: f32 = (out_l.iter().map(|s| s * s).sum::<f32>() / len as f32).sqrt();
        assert!(
            out_rms < input_rms * 0.9,
            "Listen mode should output only band, not full signal"
        );
    }

    #[test]
    fn test_param_info_complete() {
        let ds = DeEsser::new(44100);
        assert_eq!(ds.param_count(), 7);
        for i in 0..7 {
            assert!(ds.param_info(i).is_some());
        }
    }
}
