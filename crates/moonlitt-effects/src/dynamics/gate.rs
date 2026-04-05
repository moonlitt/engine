//! Noise gate / expander with hysteresis, hold timer, and dual sidechain
//! filters (HPF + LPF).
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. All internal arithmetic is f64; only the audio I/O
//! boundary touches f32.
//!
//! ## Algorithm
//!
//! 1. Sidechain: input → HPF → LPF (filters the detector input, not the
//!    audio path)
//! 2. Level detection (Peak or RMS)
//! 3. Gate state machine with hysteresis and hold timer:
//!    - Closed → Open (attack) when level > threshold
//!    - Open → Holding when level < threshold - hysteresis
//!    - Holding → Open if level rises again; → Closing when hold expires
//!    - Closing → Closed when release envelope settles
//! 4. Gain envelope: open = 0 dB (unity), closed = range_db
//! 5. Apply gain to the original (unfiltered) input

use super::envelope::EnvelopeFollower;
use crate::eq::biquad::{Biquad, BiquadCoeffs, FilterType};
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
// Gate state machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateState {
    Closed,
    Opening,
    Open,
    Holding,
    Closing,
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
    fn new(window_size: usize) -> Self {
        Self {
            buffer: vec![0.0; window_size.max(1)],
            sum: 0.0,
            pos: 0,
        }
    }

    #[inline]
    fn process(&mut self, sample: f64) -> f64 {
        let sq = sample * sample;
        self.sum -= self.buffer[self.pos];
        self.sum += sq;
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
// Gate
// ---------------------------------------------------------------------------

/// Noise gate with hysteresis, hold timer, and dual sidechain filters.
pub struct Gate {
    sample_rate: u32,

    // Parameters
    threshold_db: f64,
    range_db: f64,
    attack_ms: f64,
    hold_ms: f64,
    release_ms: f64,
    hysteresis_db: f64,
    sidechain_hpf_freq: f64,
    sidechain_lpf_freq: f64,
    detection_mode: DetectionMode,
    bypass: bool,

    // Internal state
    state: GateState,
    gain_db: f64,
    hold_counter: u32,
    hold_samples: u32,

    // Envelope follower for attack/release smoothing
    envelope: EnvelopeFollower,

    // Sidechain filters (stereo: [left, right])
    sc_hpf: [Biquad; 2],
    sc_lpf: [Biquad; 2],

    // RMS detector
    rms_window: [RmsWindow; 2],

    // External sidechain
    sidechain_ext_l: Vec<f32>,
    sidechain_ext_r: Vec<f32>,
    use_external_sidechain: bool,
}

impl Gate {
    /// Create a new noise gate with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        let rms_window_size = (sr * 0.01) as usize; // 10ms

        let mut gate = Self {
            sample_rate,

            threshold_db: -40.0,
            range_db: -80.0,
            attack_ms: 0.5,
            hold_ms: 50.0,
            release_ms: 200.0,
            hysteresis_db: 3.0,
            sidechain_hpf_freq: 20.0,
            sidechain_lpf_freq: 20000.0,
            detection_mode: DetectionMode::Peak,
            bypass: false,

            state: GateState::Closed,
            gain_db: -80.0, // start closed
            hold_counter: 0,
            hold_samples: 0,

            envelope: EnvelopeFollower::new(sr),

            sc_hpf: [Biquad::new(), Biquad::new()],
            sc_lpf: [Biquad::new(), Biquad::new()],

            rms_window: [RmsWindow::new(rms_window_size), RmsWindow::new(rms_window_size)],

            sidechain_ext_l: Vec::new(),
            sidechain_ext_r: Vec::new(),
            use_external_sidechain: false,
        };

        gate.update_envelope_coeffs();
        gate.update_hold_samples();
        gate.update_hpf();
        gate.update_lpf();
        gate
    }

    fn update_envelope_coeffs(&mut self) {
        self.envelope.set_attack_ms(self.attack_ms);
        self.envelope.set_release_ms(self.release_ms);
    }

    fn update_hold_samples(&mut self) {
        self.hold_samples = (self.hold_ms * 0.001 * self.sample_rate as f64) as u32;
    }

    fn update_hpf(&mut self) {
        let sr = self.sample_rate as f64;
        let coeffs = BiquadCoeffs::design(
            FilterType::Highpass,
            sr,
            self.sidechain_hpf_freq,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2, // Butterworth Q
        );
        self.sc_hpf[0].set_coeffs(coeffs);
        self.sc_hpf[1].set_coeffs(coeffs);
    }

    fn update_lpf(&mut self) {
        let sr = self.sample_rate as f64;
        let coeffs = BiquadCoeffs::design(
            FilterType::Lowpass,
            sr,
            self.sidechain_lpf_freq,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2, // Butterworth Q
        );
        self.sc_lpf[0].set_coeffs(coeffs);
        self.sc_lpf[1].set_coeffs(coeffs);
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Gate {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Noise Gate",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.envelope.reset();
        self.sc_hpf[0].reset();
        self.sc_hpf[1].reset();
        self.sc_lpf[0].reset();
        self.sc_lpf[1].reset();
        self.rms_window[0].reset();
        self.rms_window[1].reset();
        self.state = GateState::Closed;
        self.gain_db = self.range_db;
        self.hold_counter = 0;
    }

    // -- MIDI: no-op for a gate effect --
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

            // Sidechain: HPF → LPF (does NOT modify audio path)
            let sc_l = self.sc_lpf[0].process(self.sc_hpf[0].process(det_l));
            let sc_r = self.sc_lpf[1].process(self.sc_hpf[1].process(det_r));

            // Detect level (stereo-linked: max of both channels)
            let detected = match self.detection_mode {
                DetectionMode::Peak => sc_l.abs().max(sc_r.abs()),
                DetectionMode::Rms => {
                    let rms_l = self.rms_window[0].process(sc_l);
                    let rms_r = self.rms_window[1].process(sc_r);
                    rms_l.max(rms_r)
                }
            };

            // Convert to dB (floor at -120 dB to avoid -inf)
            let level_db = if detected > 1e-6 {
                20.0 * detected.log10()
            } else {
                -120.0
            };

            // State machine transitions
            match self.state {
                GateState::Closed => {
                    if level_db > self.threshold_db {
                        self.state = GateState::Opening;
                    }
                }
                GateState::Opening => {
                    // Attack envelope is handled by EnvelopeFollower below.
                    // Once gain reaches near unity, transition to Open.
                    if self.gain_db > -0.1 {
                        self.state = GateState::Open;
                    }
                    // If level drops during attack, go back to closing
                    if level_db < self.threshold_db - self.hysteresis_db {
                        self.state = GateState::Closing;
                    }
                }
                GateState::Open => {
                    if level_db < self.threshold_db - self.hysteresis_db {
                        self.state = GateState::Holding;
                        self.hold_counter = self.hold_samples;
                    }
                }
                GateState::Holding => {
                    if level_db > self.threshold_db {
                        // Level came back up — return to open
                        self.state = GateState::Open;
                    } else if self.hold_counter == 0 {
                        self.state = GateState::Closing;
                    } else {
                        self.hold_counter -= 1;
                    }
                }
                GateState::Closing => {
                    if level_db > self.threshold_db {
                        self.state = GateState::Opening;
                    } else if self.gain_db <= self.range_db + 0.1 {
                        self.state = GateState::Closed;
                    }
                }
            }

            // Determine target gain based on state
            let target_gain_db = match self.state {
                GateState::Closed | GateState::Closing => self.range_db,
                GateState::Open | GateState::Opening | GateState::Holding => 0.0,
            };

            // Use envelope follower to smooth the gain transition.
            // We feed the linear target to the envelope and get a smoothed value.
            let target_linear = db_to_linear(target_gain_db);
            let smoothed_linear = self.envelope.process(target_linear);

            // Convert back to dB for state tracking
            self.gain_db = linear_to_db(smoothed_linear);

            // Apply gain to the original (unfiltered) input
            out_l[i] = (l * smoothed_linear) as f32;
            out_r[i] = (r * smoothed_linear) as f32;
        }

        // Reset external sidechain flag each cycle
        self.use_external_sidechain = false;
    }

    fn set_volume(&mut self, _volume: f32) {
        // Gate does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: threshold_db    (-80..0, default -40)
    // 1: range_db        (-80..0, default -80)
    // 2: attack_ms       (0.01..100, default 0.5)
    // 3: hold_ms         (0..500, default 50)
    // 4: release_ms      (5..2000, default 200)
    // 5: hysteresis_db   (0..20, default 3)
    // 6: sidechain_hpf   (20..2000, default 20)
    // 7: sidechain_lpf   (200..20000, default 20000)
    // 8: detection_mode  (0=Peak, 1=RMS)
    // 9: bypass          (0=Off, 1=On)

    fn param_count(&self) -> u32 {
        10
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Threshold".into(),
                group: "Gate".into(),
                min: -80.0,
                max: 0.0,
                default: -40.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Range".into(),
                group: "Gate".into(),
                min: -80.0,
                max: 0.0,
                default: -80.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Attack".into(),
                group: "Envelope".into(),
                min: 0.01,
                max: 100.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Hold".into(),
                group: "Envelope".into(),
                min: 0.0,
                max: 500.0,
                default: 50.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Release".into(),
                group: "Envelope".into(),
                min: 5.0,
                max: 2000.0,
                default: 200.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Hysteresis".into(),
                group: "Gate".into(),
                min: 0.0,
                max: 20.0,
                default: 3.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Sidechain HPF".into(),
                group: "Sidechain".into(),
                min: 20.0,
                max: 2000.0,
                default: 20.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            7 => Some(ParamInfo {
                id: 7,
                name: "Sidechain LPF".into(),
                group: "Sidechain".into(),
                min: 200.0,
                max: 20000.0,
                default: 20000.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            8 => Some(ParamInfo {
                id: 8,
                name: "Detection Mode".into(),
                group: "Sidechain".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            9 => Some(ParamInfo {
                id: 9,
                name: "Bypass".into(),
                group: "Gate".into(),
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
            1 => Some(self.range_db),
            2 => Some(self.attack_ms),
            3 => Some(self.hold_ms),
            4 => Some(self.release_ms),
            5 => Some(self.hysteresis_db),
            6 => Some(self.sidechain_hpf_freq),
            7 => Some(self.sidechain_lpf_freq),
            8 => Some(match self.detection_mode {
                DetectionMode::Peak => 0.0,
                DetectionMode::Rms => 1.0,
            }),
            9 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => self.threshold_db = value.clamp(-80.0, 0.0),
            1 => self.range_db = value.clamp(-80.0, 0.0),
            2 => {
                self.attack_ms = value.clamp(0.01, 100.0);
                self.update_envelope_coeffs();
            }
            3 => {
                self.hold_ms = value.clamp(0.0, 500.0);
                self.update_hold_samples();
            }
            4 => {
                self.release_ms = value.clamp(5.0, 2000.0);
                self.update_envelope_coeffs();
            }
            5 => self.hysteresis_db = value.clamp(0.0, 20.0),
            6 => {
                self.sidechain_hpf_freq = value.clamp(20.0, 2000.0);
                self.update_hpf();
            }
            7 => {
                self.sidechain_lpf_freq = value.clamp(200.0, 20000.0);
                self.update_lpf();
            }
            8 => {
                self.detection_mode = if value >= 0.5 {
                    DetectionMode::Rms
                } else {
                    DetectionMode::Peak
                };
            }
            9 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:.1} dB", value)),
            1 => Some(format!("{:.1} dB", value)),
            2 => Some(format!("{:.2} ms", value)),
            3 => Some(format!("{:.0} ms", value)),
            4 => Some(format!("{:.0} ms", value)),
            5 => Some(format!("{:.1} dB", value)),
            6 => Some(format!("{:.0} Hz", value)),
            7 => Some(format!("{:.0} Hz", value)),
            8 => Some(if value >= 0.5 { "RMS".into() } else { "Peak".into() }),
            9 => Some(if value >= 0.5 { "On".into() } else { "Off".into() }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn db_to_linear(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

#[inline]
fn linear_to_db(linear: f64) -> f64 {
    if linear > 1e-6 {
        20.0 * linear.log10()
    } else {
        -120.0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use moonlitt_core::AudioBackend;
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

    #[test]
    fn test_bypass_is_bitexact() {
        let mut gate = Gate::new(44100);
        gate.set_param(9, 1.0); // bypass
        let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];
        gate.process_effect(&input, &input, &mut out_l, &mut out_r);
        assert_eq!(input, out_l);
    }

    #[test]
    fn test_param_round_trip() {
        let mut gate = Gate::new(44100);
        for id in 0..gate.param_count() {
            let info = gate.param_info(id).unwrap();
            gate.set_param(id, info.default);
            let val = gate.get_param(id).unwrap();
            assert!(
                (val - info.default).abs() < 0.01,
                "Param {id} round-trip failed"
            );
        }
    }

    #[test]
    fn test_below_threshold_attenuates() {
        let mut gate = Gate::new(44100);
        gate.set_param(0, -20.0); // threshold = -20dB
        gate.set_param(1, -80.0); // range = -80dB
        gate.set_param(3, 0.0); // hold = 0ms
        gate.set_param(4, 5.0); // release = 5ms (fast)

        // Feed quiet signal (-40dB = amplitude ~0.01) as a sine wave
        let amp = 0.01_f64;
        let input = sine_wave(1000.0, amp, 44100, 4410); // 100ms of 1kHz
        let mut out_l = vec![0.0f32; 4410];
        let mut out_r = vec![0.0f32; 4410];

        // Process several blocks to let gate close
        for _ in 0..5 {
            gate.process_effect(&input, &input, &mut out_l, &mut out_r);
        }
        gate.process_effect(&input, &input, &mut out_l, &mut out_r);
        // Last samples should be heavily attenuated
        let last = out_l[4409].abs();
        let input_last = input[4409].abs();
        assert!(
            last < input_last * 0.1,
            "Gate should attenuate below threshold: output={last}, input={input_last}"
        );
    }

    #[test]
    fn test_above_threshold_passes() {
        let mut gate = Gate::new(44100);
        gate.set_param(0, -40.0); // threshold = -40dB

        // Feed loud signal (-10dB = amplitude ~0.316) as a sine wave
        let amp = 0.316_f64;
        let input = sine_wave(1000.0, amp, 44100, 4410);
        let mut out_l = vec![0.0f32; 4410];
        let mut out_r = vec![0.0f32; 4410];

        for _ in 0..3 {
            gate.process_effect(&input, &input, &mut out_l, &mut out_r);
        }
        // Compare RMS of output to RMS of input — should be close
        let in_rms: f64 = input.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
            / input.len() as f64;
        let out_rms: f64 = out_l.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
            / out_l.len() as f64;
        let ratio = out_rms.sqrt() / in_rms.sqrt();
        assert!(
            (ratio - 1.0).abs() < 0.15,
            "Gate should pass signal above threshold: RMS ratio={ratio:.4}"
        );
    }

    #[test]
    fn test_param_info_complete() {
        let gate = Gate::new(44100);
        assert_eq!(gate.param_count(), 10);
        for i in 0..10 {
            assert!(
                gate.param_info(i).is_some(),
                "Missing param_info for id {i}"
            );
        }
    }

    #[test]
    fn test_info() {
        let gate = Gate::new(44100);
        assert_eq!(
            gate.info().backend_type,
            moonlitt_core::BackendType::PluginHost
        );
    }

    // -----------------------------------------------------------------------
    // Sidechain tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_supports_sidechain() {
        let gate = Gate::new(44100);
        assert!(gate.supports_sidechain());
    }

    #[test]
    fn test_external_sidechain_detection() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        // Quiet input signal (-60 dB) — below threshold
        let quiet_amp = 10.0_f64.powf(-60.0 / 20.0);
        let quiet = sine_wave(1000.0, quiet_amp, sr, num_samples);

        // Loud sidechain signal (-10 dB) — above threshold
        let loud_amp = 10.0_f64.powf(-10.0 / 20.0);
        let loud = sine_wave(440.0, loud_amp, sr, num_samples);

        // Gate A: no external sidechain — quiet signal, gate closes
        let mut gate_internal = Gate::new(sr);
        gate_internal.set_param(0, -30.0); // threshold = -30 dB
        gate_internal.set_param(1, -80.0); // range = -80 dB
        gate_internal.set_param(2, 0.5);   // attack = 0.5ms
        gate_internal.set_param(3, 0.0);   // hold = 0ms
        gate_internal.set_param(4, 5.0);   // release = 5ms

        // Process several blocks to let gate close
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        for _ in 0..3 {
            gate_internal.process_effect(&quiet, &quiet, &mut out_l, &mut out_r);
        }
        gate_internal.process_effect(&quiet, &quiet, &mut out_l, &mut out_r);
        let check_start = sr as usize / 2;
        let rms_internal = out_l[check_start..].iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
            / (num_samples - check_start) as f64;
        let rms_internal = rms_internal.sqrt();

        // Gate B: external sidechain with loud signal — gate should open
        let mut gate_external = Gate::new(sr);
        gate_external.set_param(0, -30.0);
        gate_external.set_param(1, -80.0);
        gate_external.set_param(2, 0.5);
        gate_external.set_param(3, 0.0);
        gate_external.set_param(4, 5.0);

        let mut out_ext_l = vec![0.0f32; num_samples];
        let mut out_ext_r = vec![0.0f32; num_samples];
        for _ in 0..3 {
            gate_external.set_sidechain(&loud, &loud);
            gate_external.process_effect(&quiet, &quiet, &mut out_ext_l, &mut out_ext_r);
        }
        gate_external.set_sidechain(&loud, &loud);
        gate_external.process_effect(&quiet, &quiet, &mut out_ext_l, &mut out_ext_r);
        let rms_external = out_ext_l[check_start..].iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
            / (num_samples - check_start) as f64;
        let rms_external = rms_external.sqrt();

        // With external sidechain (loud), the gate should be open,
        // so the output should be louder than when the gate is closed.
        assert!(
            rms_external > rms_internal * 5.0,
            "External sidechain should open gate: internal RMS={:.6}, external RMS={:.6}",
            rms_internal, rms_external
        );
    }
}
