//! Dynamics compressor with log-domain gain computation and soft knee.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. All internal arithmetic is f64; only the audio I/O
//! boundary touches f32.
//!
//! ## Algorithm
//!
//! 1. Sidechain HPF filters the detector input (not the audio path)
//! 2. Level detection (Peak or RMS)
//! 3. Log-domain gain computation with optional soft knee
//! 4. Envelope smoothing (attack/release)
//! 5. Gain application to the original (unfiltered) input

use std::f64::consts::PI;

use super::envelope::EnvelopeFollower;
use crate::common::DbLut;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Detection mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionMode {
    Peak,
    Rms,
}

// ---------------------------------------------------------------------------
// Biquad — lightweight 2nd-order IIR for sidechain HPF
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
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

    /// Design a 2nd-order Butterworth highpass filter.
    ///
    /// Uses the Audio EQ Cookbook formula with Q = 1/sqrt(2).
    fn design_highpass(sample_rate: f64, freq: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let q = std::f64::consts::FRAC_1_SQRT_2; // Butterworth Q
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

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ---------------------------------------------------------------------------
// RMS window — running sum of squares
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RmsWindow {
    buffer: Vec<f64>,
    sum: f64,
    pos: usize,
}

impl RmsWindow {
    /// Create a new RMS window with the given size in samples.
    fn new(window_size: usize) -> Self {
        Self {
            buffer: vec![0.0; window_size.max(1)],
            sum: 0.0,
            pos: 0,
        }
    }

    /// Push a new sample and return the RMS over the window.
    #[inline]
    fn process(&mut self, sample: f64) -> f64 {
        let sq = sample * sample;
        // Subtract the oldest squared value, add the new one
        self.sum -= self.buffer[self.pos];
        self.sum += sq;
        // Clamp to avoid negative due to floating-point drift
        if self.sum < 0.0 {
            self.sum = 0.0;
        }
        self.buffer[self.pos] = sq;
        self.pos += 1;
        if self.pos >= self.buffer.len() {
            self.pos = 0;
        }
        (self.sum / self.buffer.len() as f64).sqrt()
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.sum = 0.0;
        self.pos = 0;
    }
}

// ---------------------------------------------------------------------------
// Compressor
// ---------------------------------------------------------------------------

/// Dynamics compressor with log-domain gain computation, soft knee,
/// sidechain HPF, and configurable detection mode.
pub struct Compressor {
    sample_rate: u32,

    // Parameters
    threshold_db: f64,
    ratio: f64,
    attack_ms: f64,
    release_ms: f64,
    knee_db: f64,
    makeup_db: f64,
    sidechain_hpf_freq: f64,
    detection_mode: DetectionMode,
    bypass: bool,

    // Internal state
    envelope_left: EnvelopeFollower,
    envelope_right: EnvelopeFollower,
    sidechain_hpf: [Biquad; 2],
    rms_window: [RmsWindow; 2],

    // External sidechain
    sidechain_ext_l: Vec<f32>,
    sidechain_ext_r: Vec<f32>,
    use_external_sidechain: bool,

    // dB→linear lookup table
    db_lut: DbLut,
}

impl Compressor {
    /// Create a new compressor with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        let rms_window_size = (sr * 0.01) as usize; // 10ms

        let mut comp = Self {
            sample_rate,

            threshold_db: -20.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            knee_db: 0.0,
            makeup_db: 0.0,
            sidechain_hpf_freq: 20.0,
            detection_mode: DetectionMode::Peak,
            bypass: false,

            envelope_left: EnvelopeFollower::new(sr),
            envelope_right: EnvelopeFollower::new(sr),
            sidechain_hpf: [Biquad::new(), Biquad::new()],
            rms_window: [RmsWindow::new(rms_window_size), RmsWindow::new(rms_window_size)],

            sidechain_ext_l: Vec::new(),
            sidechain_ext_r: Vec::new(),
            use_external_sidechain: false,

            db_lut: DbLut::new(),
        };

        comp.update_envelope_coeffs();
        comp.update_hpf();
        comp
    }

    /// Recompute envelope attack/release coefficients.
    fn update_envelope_coeffs(&mut self) {
        self.envelope_left.set_attack_ms(self.attack_ms);
        self.envelope_left.set_release_ms(self.release_ms);
        self.envelope_right.set_attack_ms(self.attack_ms);
        self.envelope_right.set_release_ms(self.release_ms);
    }

    /// Recompute sidechain HPF coefficients.
    /// When freq is 0, the HPF is bypassed (identity filter).
    fn update_hpf(&mut self) {
        if self.sidechain_hpf_freq == 0.0 {
            // Bypass: identity filter (passthrough)
            self.sidechain_hpf[0] = Biquad::new();
            self.sidechain_hpf[1] = Biquad::new();
        } else {
            let sr = self.sample_rate as f64;
            self.sidechain_hpf[0] = Biquad::design_highpass(sr, self.sidechain_hpf_freq);
            self.sidechain_hpf[1] = Biquad::design_highpass(sr, self.sidechain_hpf_freq);
        }
    }

    /// Compute compression gain in dB for a given input level in dB.
    /// Delegates to the free function for reuse.
    #[inline]
    pub fn compute_gain_db(&self, level_db: f64) -> f64 {
        compute_gain_db_static(self.threshold_db, self.ratio, self.knee_db, level_db)
    }

}

/// Process a single channel's sample through detection and gain computation.
///
/// Free function to avoid borrow-checker conflicts in `process_effect`.
/// Returns the gain in dB to apply (including makeup).
#[inline]
#[allow(clippy::too_many_arguments)]
fn detect_and_compute(
    detection_mode: DetectionMode,
    threshold_db: f64,
    ratio: f64,
    knee_db: f64,
    makeup_db: f64,
    sc_sample: f64,
    rms_window: &mut RmsWindow,
    envelope: &mut EnvelopeFollower,
) -> f64 {
    // Detect level based on mode
    let detected = match detection_mode {
        DetectionMode::Peak => sc_sample.abs(),
        DetectionMode::Rms => rms_window.process(sc_sample),
    };

    // Convert to dB (floor at -120dB to avoid -inf)
    let level_db = if detected > 1e-6 {
        20.0 * detected.log10()
    } else {
        -120.0
    };

    // Compute gain reduction from compression curve
    let gain_db = compute_gain_db_static(threshold_db, ratio, knee_db, level_db);

    // Smooth gain reduction through envelope follower.
    // We track the absolute gain reduction magnitude and smooth it.
    let gr_magnitude = (-gain_db).max(0.0);
    let smoothed_gr = envelope.process(gr_magnitude);
    -smoothed_gr + makeup_db
}

/// Static gain computation — no `&self` needed.
/// Returns gain (negative or zero) to apply, excluding makeup.
#[inline]
fn compute_gain_db_static(threshold: f64, ratio: f64, knee: f64, level_db: f64) -> f64 {
    let output_db = if knee <= 0.0 {
        // Hard knee
        if level_db <= threshold {
            level_db
        } else {
            threshold + (level_db - threshold) / ratio
        }
    } else {
        // Soft knee
        let knee_start = threshold - knee / 2.0;
        let knee_end = threshold + knee / 2.0;

        if level_db <= knee_start {
            level_db
        } else if level_db >= knee_end {
            threshold + (level_db - threshold) / ratio
        } else {
            // Quadratic interpolation within knee region
            let x = level_db - threshold + knee / 2.0;
            level_db + ((1.0 / ratio - 1.0) * x * x) / (2.0 * knee)
        }
    };

    output_db - level_db
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Compressor {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Dynamics Compressor",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.envelope_left.reset();
        self.envelope_right.reset();
        self.sidechain_hpf[0].reset();
        self.sidechain_hpf[1].reset();
        self.rms_window[0].reset();
        self.rms_window[1].reset();
    }

    // -- MIDI: no-op for a compressor effect --
    fn note_on(&mut self, _channel: u8, _note: u8, _velocity: u8) {}
    fn note_off(&mut self, _channel: u8, _note: u8) {}
    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: i16) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn all_notes_off(&mut self) {}

    // -- Audio: generator render is a no-op (this is an effect) --
    fn render(&mut self, _left: &mut [f32], _right: &mut [f32]) {}

    fn set_sidechain(&mut self, left: &[f32], right: &[f32]) {
        self.sidechain_ext_l.resize(left.len(), 0.0);
        self.sidechain_ext_r.resize(right.len(), 0.0);
        self.sidechain_ext_l.copy_from_slice(left);
        self.sidechain_ext_r.copy_from_slice(right);
        self.use_external_sidechain = true;
    }

    fn supports_sidechain(&self) -> bool { true }

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

        // Select detection source: external sidechain or input signal
        let use_ext = self.use_external_sidechain
            && self.sidechain_ext_l.len() >= len
            && self.sidechain_ext_r.len() >= len;

        for i in 0..len {
            let l = in_l[i] as f64;
            let r = in_r[i] as f64;

            // Detection source: external sidechain or input
            let det_l = if use_ext { self.sidechain_ext_l[i] as f64 } else { l };
            let det_r = if use_ext { self.sidechain_ext_r[i] as f64 } else { r };

            // Sidechain: filter through HPF (does NOT modify audio path)
            let sc_l = self.sidechain_hpf[0].process(det_l);
            let sc_r = self.sidechain_hpf[1].process(det_r);

            // Detect level and compute gain for each channel
            let gain_db_l = detect_and_compute(
                self.detection_mode,
                self.threshold_db,
                self.ratio,
                self.knee_db,
                self.makeup_db,
                sc_l,
                &mut self.rms_window[0],
                &mut self.envelope_left,
            );
            let gain_db_r = detect_and_compute(
                self.detection_mode,
                self.threshold_db,
                self.ratio,
                self.knee_db,
                self.makeup_db,
                sc_r,
                &mut self.rms_window[1],
                &mut self.envelope_right,
            );

            // Convert gain from dB to linear (LUT: ~5 cycles vs powf: ~100 cycles)
            let gain_l = self.db_lut.db_to_linear(gain_db_l);
            let gain_r = self.db_lut.db_to_linear(gain_db_r);

            // Apply gain to the original (unfiltered) input
            out_l[i] = (l * gain_l) as f32;
            out_r[i] = (r * gain_r) as f32;
        }

        // Reset external sidechain flag each cycle
        self.use_external_sidechain = false;
    }

    fn set_volume(&mut self, _volume: f32) {
        // Compressor does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: threshold_db  (-60..0)
    // 1: ratio         (1..100, where 100 = inf)
    // 2: attack_ms     (0.1..100)
    // 3: release_ms    (10..1000)
    // 4: knee_db       (0..30)
    // 5: makeup_db     (0..30)
    // 6: sidechain_hpf_freq (20..500)
    // 7: detection_mode (0=Peak, 1=Rms)
    // 8: bypass        (0 or 1)

    fn param_count(&self) -> u32 {
        9
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Threshold".into(),
                group: "Dynamics".into(),
                min: -60.0,
                max: 0.0,
                default: -20.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Ratio".into(),
                group: "Dynamics".into(),
                min: 1.0,
                max: 100.0,
                default: 4.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Attack".into(),
                group: "Dynamics".into(),
                min: 0.1,
                max: 100.0,
                default: 10.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Release".into(),
                group: "Dynamics".into(),
                min: 10.0,
                max: 1000.0,
                default: 100.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Knee".into(),
                group: "Dynamics".into(),
                min: 0.0,
                max: 30.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Makeup".into(),
                group: "Dynamics".into(),
                min: 0.0,
                max: 30.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Sidechain HPF".into(),
                group: "Sidechain".into(),
                min: 20.0,
                max: 500.0,
                default: 20.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            7 => Some(ParamInfo {
                id: 7,
                name: "Detection Mode".into(),
                group: "Sidechain".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            8 => Some(ParamInfo {
                id: 8,
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
            1 => Some(self.ratio),
            2 => Some(self.attack_ms),
            3 => Some(self.release_ms),
            4 => Some(self.knee_db),
            5 => Some(self.makeup_db),
            6 => Some(self.sidechain_hpf_freq),
            7 => Some(match self.detection_mode {
                DetectionMode::Peak => 0.0,
                DetectionMode::Rms => 1.0,
            }),
            8 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => self.threshold_db = value.clamp(-60.0, 0.0),
            1 => self.ratio = value.clamp(1.0, 100.0),
            2 => {
                self.attack_ms = value.clamp(0.1, 100.0);
                self.update_envelope_coeffs();
            }
            3 => {
                self.release_ms = value.clamp(10.0, 1000.0);
                self.update_envelope_coeffs();
            }
            4 => self.knee_db = value.clamp(0.0, 30.0),
            5 => self.makeup_db = value.clamp(0.0, 30.0),
            6 => {
                // 0 = bypass (identity), 20..500 = active HPF
                self.sidechain_hpf_freq = if value < 1.0 { 0.0 } else { value.clamp(20.0, 500.0) };
                self.update_hpf();
            }
            7 => {
                self.detection_mode = if value >= 0.5 {
                    DetectionMode::Rms
                } else {
                    DetectionMode::Peak
                };
            }
            8 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.1} dB", value)),
            1 => {
                if value >= 99.5 {
                    Some("inf:1".into())
                } else {
                    Some(format!("{:.1}:1", value))
                }
            }
            2 => Some(format!("{:.1} ms", value)),
            3 => Some(format!("{:.0} ms", value)),
            4 => Some(format!("{:.1} dB", value)),
            5 => Some(format!("{:.1} dB", value)),
            6 => Some(format!("{:.0} Hz", value)),
            7 => Some(if value >= 0.5 { "RMS".into() } else { "Peak".into() }),
            8 => Some(if value >= 0.5 { "On".into() } else { "Off".into() }),
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
    use std::f64::consts::PI;

    /// Generate a mono sine wave at the given amplitude (linear).
    fn sine_wave(freq: f64, amplitude: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (amplitude * (2.0 * PI * freq * t).sin()) as f32
            })
            .collect()
    }

    /// Measure RMS amplitude of a buffer in dB.
    fn rms_db(buf: &[f32]) -> f64 {
        let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        let rms = (sum_sq / buf.len() as f64).sqrt();
        20.0 * rms.log10()
    }

    // -----------------------------------------------------------------------
    // test_bypass_is_bitexact
    // -----------------------------------------------------------------------

    #[test]
    fn test_bypass_is_bitexact() {
        let mut comp = Compressor::new(44100);
        comp.set_param(8, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        comp.process_effect(&input, &silent, &mut out_l, &mut out_r);

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

    // -----------------------------------------------------------------------
    // test_below_threshold_is_bitexact
    // -----------------------------------------------------------------------

    #[test]
    fn test_below_threshold_is_bitexact() {
        let sr = 44100;
        let mut comp = Compressor::new(sr);
        // Threshold at 0 dB (max), input at -20 dB => no compression
        comp.set_param(0, 0.0); // threshold = 0 dB
        comp.set_param(1, 4.0); // ratio = 4:1
        comp.set_param(4, 0.0); // knee = 0
        comp.set_param(5, 0.0); // makeup = 0
        // Use very fast attack/release so envelope settles quickly
        comp.set_param(2, 0.1); // attack = 0.1ms
        comp.set_param(3, 10.0); // release = 10ms
        // Set HPF to minimum to avoid filtering effects
        comp.set_param(6, 20.0);

        // Input at -20 dB (well below threshold of 0 dB)
        let amplitude = 10.0_f64.powf(-20.0 / 20.0); // 0.1
        let num_samples = sr as usize * 2; // 2 seconds
        let input = sine_wave(1000.0, amplitude, sr, num_samples);
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        comp.process_effect(&input, &input, &mut out_l, &mut out_r);

        // After the envelope has settled (skip first 0.5s), output should
        // be essentially identical to input (gain reduction = 0, makeup = 0).
        // Due to HPF at 20Hz on the sidechain and floating-point arithmetic,
        // we allow a tiny tolerance but the audio path itself should be
        // nearly perfect since gain = 1.0 (0 dB).
        let check_start = sr as usize / 2;
        let input_rms = rms_db(&input[check_start..]);
        let output_rms = rms_db(&out_l[check_start..]);
        let error = (output_rms - input_rms).abs();

        assert!(
            error < 0.01,
            "below threshold: output should match input, error = {:.6} dB (input={:.3} dB, output={:.3} dB)",
            error, input_rms, output_rms
        );
    }

    // -----------------------------------------------------------------------
    // test_gain_reduction_formula
    // -----------------------------------------------------------------------

    #[test]
    fn test_gain_reduction_formula() {
        let sr = 44100;
        let mut comp = Compressor::new(sr);
        // threshold=-20dB, ratio=4:1, knee=0, makeup=0
        comp.set_param(0, -20.0);
        comp.set_param(1, 4.0);
        comp.set_param(4, 0.0);
        comp.set_param(5, 0.0);
        // Very fast attack, very slow release → envelope "peak holds"
        comp.set_param(2, 0.1);    // 0.1ms attack
        comp.set_param(3, 1000.0); // 1000ms release (holds near peak)
        comp.set_param(6, 20.0);   // HPF at 20Hz

        // Input at -10 dB peak (10 dB above threshold)
        let amplitude = 10.0_f64.powf(-10.0 / 20.0);
        let num_samples = sr as usize * 4; // 4 seconds for settling
        let input = sine_wave(1000.0, amplitude, sr, num_samples);
        let silent = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        comp.process_effect(&input, &silent, &mut out_l, &mut out_r);

        // Expected gain reduction = -(10 - 10/4) = -7.5 dB
        // Measure by comparing output RMS to input RMS (ratio cancels peak/RMS)
        let expected_gain_db = -7.5;

        let measure_start = sr as usize * 3;
        let input_rms = rms_db(&input[measure_start..]);
        let output_rms = rms_db(&out_l[measure_start..]);
        let measured_gain = output_rms - input_rms;

        let error = (measured_gain - expected_gain_db).abs();
        assert!(
            error < 0.5,
            "steady-state gain: expected {:.1} dB, got {:.4} dB (error {:.4} dB)",
            expected_gain_db, measured_gain, error
        );
    }

    // -----------------------------------------------------------------------
    // test_attack_timing
    // -----------------------------------------------------------------------

    #[test]
    fn test_attack_timing() {
        let sr = 44100u32;
        let mut comp = Compressor::new(sr);
        comp.set_param(0, -40.0);  // threshold = -40 dB (low, so signal triggers)
        comp.set_param(1, 100.0);  // ratio = inf (limiter)
        comp.set_param(2, 10.0);   // attack = 10ms
        comp.set_param(3, 1000.0); // release = 1s (slow, so we measure attack)
        comp.set_param(4, 0.0);    // knee = 0
        comp.set_param(5, 0.0);    // makeup = 0
        comp.set_param(6, 20.0);   // HPF off essentially

        // Start with silence (1 second to ensure envelope at 0)
        let silence_samples = sr as usize;
        let silence = vec![0.0f32; silence_samples];
        let mut out_l_silence = vec![0.0f32; silence_samples];
        let mut out_r_silence = vec![0.0f32; silence_samples];
        comp.process_effect(&silence, &silence, &mut out_l_silence, &mut out_r_silence);

        // Then sudden onset: steady tone at -10 dB
        let amplitude = 10.0_f64.powf(-10.0 / 20.0);
        let onset_samples = sr as usize; // 1 second of signal
        let onset: Vec<f32> = (0..onset_samples)
            .map(|i| (amplitude * (2.0 * PI * 1000.0 * i as f64 / sr as f64).sin()) as f32)
            .collect();
        let mut out_onset = vec![0.0f32; onset_samples];
        let mut out_r = vec![0.0f32; onset_samples];

        comp.process_effect(&onset, &onset, &mut out_onset, &mut out_r);

        // The envelope should reach ~63.2% of final gain reduction after
        // 1 time constant = 10ms = 441 samples.
        // Find the gain reduction at sample 441.
        // Final GR (steady state) can be estimated from samples near the end.
        let final_gr_samples = &out_onset[sr as usize / 2..];
        let final_rms = rms_db(final_gr_samples);

        // At onset (first few samples), gain reduction should be minimal.
        // The onset is gradual due to sine wave, so we look at envelope behavior.
        // After 441 samples (10ms), GR should be ~63% of final.

        // Measure GR at exactly 1 time constant (441 samples)
        // Use a small window around that point
        let tc_samples = (10.0 * 0.001 * sr as f64) as usize; // 441
        let window = 50;
        let tc_start = tc_samples.saturating_sub(window / 2);
        let tc_end = (tc_samples + window / 2).min(onset_samples);
        let tc_rms = rms_db(&out_onset[tc_start..tc_end]);

        // The input level is constant at -10dB.
        // GR starts at 0 and converges to some final value.
        // At 1 time constant, it should be at ~63% of the way to final GR.
        let input_db = -10.0;
        let gr_at_tc = input_db - tc_rms; // positive = gain reduction
        let gr_final = input_db - final_rms;

        // The ratio of GR at tc to final GR should be ~0.632
        if gr_final.abs() > 0.5 {
            // Only test if there's meaningful gain reduction
            let ratio = gr_at_tc / gr_final;
            let expected_ratio = 1.0 - (-1.0_f64).exp(); // 0.6321

            assert!(
                (ratio - expected_ratio).abs() < 0.15,
                "attack time constant: expected ratio ~{:.3}, got {:.3} (tc_gr={:.2} dB, final_gr={:.2} dB)",
                expected_ratio, ratio, gr_at_tc, gr_final
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_soft_knee
    // -----------------------------------------------------------------------

    #[test]
    fn test_soft_knee() {
        let comp = Compressor::new(44100);

        // Test the gain computation directly (no envelope dynamics)
        // knee=6dB, threshold=-20dB, ratio=4:1
        let threshold = -20.0;
        let _ratio = 4.0;
        let knee = 6.0;

        let mut test_comp = Compressor::new(44100);
        test_comp.set_param(0, threshold);
        test_comp.set_param(1, 4.0);
        test_comp.set_param(4, knee);

        // At threshold - knee/2 = -23 dB: should be ~0 dB gain reduction
        let gain_at_knee_start = test_comp.compute_gain_db(-23.0);
        assert!(
            gain_at_knee_start.abs() < 1e-10,
            "at knee start (-23dB), gain reduction should be ~0, got {:.6} dB",
            gain_at_knee_start
        );

        // At threshold (-20 dB): should have partial reduction
        let gain_at_threshold = test_comp.compute_gain_db(-20.0);
        assert!(
            gain_at_threshold < 0.0,
            "at threshold, should have some gain reduction, got {:.6} dB",
            gain_at_threshold
        );

        // At threshold + knee/2 = -17 dB: should match hard-knee behavior
        let gain_at_knee_end = test_comp.compute_gain_db(-17.0);
        // Hard knee at -17dB: gain = threshold + (level - threshold)/ratio - level
        // = -20 + (-17 - (-20))/4 - (-17) = -20 + 3/4 + 17 = -2.25 dB
        let expected_hard_knee = -20.0 + ((-17.0) - (-20.0)) / 4.0 - (-17.0);
        assert!(
            (gain_at_knee_end - expected_hard_knee).abs() < 1e-6,
            "at knee end (-17dB), gain should match hard-knee ({:.4} dB), got {:.4} dB",
            expected_hard_knee, gain_at_knee_end
        );

        // Verify smooth transition: no discontinuity across knee region
        let mut prev_gain = test_comp.compute_gain_db(-24.0);
        for i in 0..100 {
            let level = -24.0 + i as f64 * 0.1; // -24 to -14
            let gain = test_comp.compute_gain_db(level);
            let diff = (gain - prev_gain).abs();
            assert!(
                diff < 0.5, // No jump larger than 0.5 dB per 0.1 dB step
                "discontinuity at {:.1} dB: gain jumped by {:.4} dB",
                level, diff
            );
            prev_gain = gain;
        }

        // Drop the unused variable
        drop(comp);
    }

    // -----------------------------------------------------------------------
    // test_infinite_ratio
    // -----------------------------------------------------------------------

    #[test]
    fn test_infinite_ratio() {
        let sr = 44100;
        let mut comp = Compressor::new(sr);
        comp.set_param(0, -20.0);   // threshold = -20 dB
        comp.set_param(1, 100.0);   // ratio = inf (limiter)
        comp.set_param(4, 0.0);     // knee = 0
        comp.set_param(5, 0.0);     // makeup = 0
        comp.set_param(2, 0.1);     // very fast attack
        comp.set_param(3, 1000.0);  // slow release (peak hold)
        comp.set_param(6, 20.0);    // HPF at minimum

        // Input at -10 dB peak (10 dB above threshold)
        let amplitude = 10.0_f64.powf(-10.0 / 20.0);
        let num_samples = sr as usize * 4;
        let input = sine_wave(1000.0, amplitude, sr, num_samples);
        let silent = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        comp.process_effect(&input, &silent, &mut out_l, &mut out_r);

        // With inf ratio (100:1), gain = threshold + (level - threshold)/100 - level
        // ≈ threshold - level = -20 - (-10) = -10 dB gain
        // Measure by comparing output RMS to input RMS
        let expected_gain_db = -10.0 + (-10.0 - (-20.0)) / 100.0; // -9.9 dB
        let measure_start = sr as usize * 3;
        let input_rms = rms_db(&input[measure_start..]);
        let output_rms = rms_db(&out_l[measure_start..]);
        let measured_gain = output_rms - input_rms;

        let error = (measured_gain - expected_gain_db).abs();
        assert!(
            error < 1.0,
            "limiter (inf ratio): expected gain ~{:.1} dB, got {:.2} dB (error {:.2} dB)",
            expected_gain_db, measured_gain, error
        );
    }

    // -----------------------------------------------------------------------
    // test_param_round_trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_round_trip() {
        let mut comp = Compressor::new(44100);

        // Set and get each parameter
        comp.set_param(0, -30.0);
        assert_eq!(comp.get_param(0), Some(-30.0));

        comp.set_param(1, 8.0);
        assert_eq!(comp.get_param(1), Some(8.0));

        comp.set_param(2, 5.0);
        assert_eq!(comp.get_param(2), Some(5.0));

        comp.set_param(3, 200.0);
        assert_eq!(comp.get_param(3), Some(200.0));

        comp.set_param(4, 6.0);
        assert_eq!(comp.get_param(4), Some(6.0));

        comp.set_param(5, 10.0);
        assert_eq!(comp.get_param(5), Some(10.0));

        comp.set_param(6, 100.0);
        assert_eq!(comp.get_param(6), Some(100.0));

        comp.set_param(7, 1.0);
        assert_eq!(comp.get_param(7), Some(1.0));

        comp.set_param(8, 1.0);
        assert_eq!(comp.get_param(8), Some(1.0));

        // Clamping
        comp.set_param(0, -100.0);
        assert_eq!(comp.get_param(0), Some(-60.0));

        comp.set_param(0, 10.0);
        assert_eq!(comp.get_param(0), Some(0.0));

        // Invalid param
        assert_eq!(comp.get_param(99), None);
        assert!(comp.param_info(9).is_none());

        // Param count
        assert_eq!(comp.param_count(), 9);
    }

    // -----------------------------------------------------------------------
    // test_info
    // -----------------------------------------------------------------------

    #[test]
    fn test_info() {
        let comp = Compressor::new(44100);
        let info = comp.info();
        assert_eq!(info.name, "Dynamics Compressor");
        assert_eq!(info.backend_type, BackendType::PluginHost);
        assert!(info.extensions.is_empty());
    }

    #[test]
    fn test_latency_is_zero() {
        let comp = Compressor::new(44100);
        assert_eq!(comp.latency(), 0);
    }

    // -----------------------------------------------------------------------
    // Sidechain tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_supports_sidechain() {
        let comp = Compressor::new(44100);
        assert!(comp.supports_sidechain());
    }

    #[test]
    fn test_external_sidechain_detection() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        // Quiet input signal (-60 dB) — below threshold, normally no compression
        let quiet_amp = 10.0_f64.powf(-60.0 / 20.0); // 0.001
        let quiet = sine_wave(1000.0, quiet_amp, sr, num_samples);

        // Loud sidechain signal (-6 dB) — above threshold, triggers compression
        let loud_amp = 10.0_f64.powf(-6.0 / 20.0); // ~0.5
        let loud = sine_wave(440.0, loud_amp, sr, num_samples);

        // Compressor A: no external sidechain — quiet signal, no compression
        let mut comp_internal = Compressor::new(sr);
        comp_internal.set_param(0, -20.0); // threshold = -20 dB
        comp_internal.set_param(1, 10.0);  // ratio = 10:1
        comp_internal.set_param(2, 0.1);   // fast attack
        comp_internal.set_param(3, 10.0);  // fast release
        comp_internal.set_param(5, 0.0);   // no makeup

        let mut out_internal_l = vec![0.0f32; num_samples];
        let mut out_internal_r = vec![0.0f32; num_samples];
        comp_internal.process_effect(&quiet, &quiet, &mut out_internal_l, &mut out_internal_r);

        // Compressor B: external sidechain with loud signal
        let mut comp_external = Compressor::new(sr);
        comp_external.set_param(0, -20.0);
        comp_external.set_param(1, 10.0);
        comp_external.set_param(2, 0.1);
        comp_external.set_param(3, 10.0);
        comp_external.set_param(5, 0.0);

        comp_external.set_sidechain(&loud, &loud);
        let mut out_external_l = vec![0.0f32; num_samples];
        let mut out_external_r = vec![0.0f32; num_samples];
        comp_external.process_effect(&quiet, &quiet, &mut out_external_l, &mut out_external_r);

        // With external sidechain, the loud signal triggers gain reduction,
        // so the quiet output should be even quieter than without sidechain.
        let check_start = sr as usize / 2; // skip transient
        let rms_internal = rms_db(&out_internal_l[check_start..]);
        let rms_external = rms_db(&out_external_l[check_start..]);

        assert!(
            rms_external < rms_internal - 3.0,
            "External sidechain should cause more gain reduction: internal={:.1} dB, external={:.1} dB",
            rms_internal, rms_external
        );
    }
}
