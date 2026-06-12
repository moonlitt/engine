//! Auto-filter: envelope follower or LFO modulates a resonant biquad filter's
//! cutoff frequency.
//!
//! ## Algorithm
//!
//! 1. Modulation source: envelope follower (tracks input amplitude) or LFO
//! 2. Exponential frequency mapping: `freq = min_freq × (max_freq/min_freq)^(mod × sensitivity)`
//! 3. Resonant biquad filter (LP/HP/BP) at the modulated frequency
//! 4. Dry/wet mix

use std::f64::consts::PI;

use super::lfo::{Lfo, LfoShape};
use crate::common::ParamSmoother;
use crate::dynamics::envelope::EnvelopeFollower;
use crate::eq::biquad::{Biquad, BiquadCoeffs, FilterType};
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Auto-filter type (LP / HP / BP)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoFilterType {
    Lowpass,
    Highpass,
    Bandpass,
}

impl AutoFilterType {
    fn from_value(v: f64) -> Self {
        let i = v.round() as u32;
        match i {
            0 => Self::Lowpass,
            1 => Self::Highpass,
            _ => Self::Bandpass,
        }
    }

    fn to_value(self) -> f64 {
        match self {
            Self::Lowpass => 0.0,
            Self::Highpass => 1.0,
            Self::Bandpass => 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Modulation source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModSource {
    Envelope,
    Lfo,
}

impl ModSource {
    fn from_value(v: f64) -> Self {
        if v >= 0.5 {
            Self::Lfo
        } else {
            Self::Envelope
        }
    }
}

// ---------------------------------------------------------------------------
// Bandpass coefficient design (cookbook formula, constant 0 dB peak gain)
// ---------------------------------------------------------------------------

/// Design bandpass coefficients (constant 0 dB peak gain, BPF from cookbook).
///
/// `b0 = alpha`, `b1 = 0`, `b2 = -alpha`, `a0 = 1 + alpha`, `a1 = -2cos(w0)`, `a2 = 1 - alpha`
fn design_bandpass(sample_rate: f64, freq: f64, q: f64) -> BiquadCoeffs {
    let w0 = 2.0 * PI * freq / sample_rate;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * q);

    let a0 = 1.0 + alpha;
    let inv_a0 = 1.0 / a0;

    BiquadCoeffs {
        b0: alpha * inv_a0,
        b1: 0.0,
        b2: -alpha * inv_a0,
        a1: -2.0 * cos_w0 * inv_a0,
        a2: (1.0 - alpha) * inv_a0,
    }
}

/// Design filter coefficients for the auto-filter, dispatching by type.
fn design_filter(filter_type: AutoFilterType, sample_rate: f64, freq: f64, q: f64) -> BiquadCoeffs {
    // Clamp frequency to valid range for biquad stability
    let freq = freq.clamp(10.0, sample_rate * 0.499);
    let q = q.max(0.1);

    match filter_type {
        AutoFilterType::Lowpass => {
            BiquadCoeffs::design(FilterType::Lowpass, sample_rate, freq, 0.0, q)
        }
        AutoFilterType::Highpass => {
            BiquadCoeffs::design(FilterType::Highpass, sample_rate, freq, 0.0, q)
        }
        AutoFilterType::Bandpass => design_bandpass(sample_rate, freq, q),
    }
}

// ---------------------------------------------------------------------------
// Coefficient update interval
// ---------------------------------------------------------------------------

/// Update biquad coefficients every N samples for performance.
/// 4 samples at 44.1 kHz ≈ 0.09 ms — inaudible stepping.
const COEFF_UPDATE_INTERVAL: usize = 4;

// ---------------------------------------------------------------------------
// AutoFilter
// ---------------------------------------------------------------------------

/// Auto-filter effect: envelope follower or LFO → resonant biquad filter.
pub struct AutoFilter {
    sample_rate: u32,

    // Parameters
    source: ModSource,
    filter_type: AutoFilterType,
    min_freq: f64,
    max_freq: f64,
    resonance: f64,
    sensitivity: f64,
    attack_ms: f64,
    release_ms: f64,
    lfo_rate: f64,
    lfo_shape: LfoShape,
    dry_wet: f64,
    bypass: bool,

    // Smoothers
    min_freq_smooth: ParamSmoother,
    max_freq_smooth: ParamSmoother,
    resonance_smooth: ParamSmoother,
    dry_wet_smooth: ParamSmoother,

    // Internal DSP state
    envelope_left: EnvelopeFollower,
    envelope_right: EnvelopeFollower,
    lfo: Lfo,
    filter_left: Biquad,
    filter_right: Biquad,
}

impl AutoFilter {
    /// Create a new auto-filter with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        let mut env_l = EnvelopeFollower::new(sr);
        let mut env_r = EnvelopeFollower::new(sr);
        env_l.set_attack_ms(5.0);
        env_l.set_release_ms(50.0);
        env_r.set_attack_ms(5.0);
        env_r.set_release_ms(50.0);

        Self {
            sample_rate,

            source: ModSource::Envelope,
            filter_type: AutoFilterType::Lowpass,
            min_freq: 100.0,
            max_freq: 5000.0,
            resonance: 2.0,
            sensitivity: 0.5,
            attack_ms: 5.0,
            release_ms: 50.0,
            lfo_rate: 1.0,
            lfo_shape: LfoShape::Sine,
            dry_wet: 1.0,
            bypass: false,

            min_freq_smooth: ParamSmoother::new(100.0, sr, 5.0),
            max_freq_smooth: ParamSmoother::new(5000.0, sr, 5.0),
            resonance_smooth: ParamSmoother::new(2.0, sr, 5.0),
            dry_wet_smooth: ParamSmoother::new(1.0, sr, 5.0),

            envelope_left: env_l,
            envelope_right: env_r,
            lfo: Lfo::new(sample_rate),
            filter_left: Biquad::new(),
            filter_right: Biquad::new(),
        }
    }

    /// Update envelope follower attack/release from current params.
    fn update_envelope_coeffs(&mut self) {
        self.envelope_left.set_attack_ms(self.attack_ms);
        self.envelope_left.set_release_ms(self.release_ms);
        self.envelope_right.set_attack_ms(self.attack_ms);
        self.envelope_right.set_release_ms(self.release_ms);
    }

    /// Compute the modulated filter frequency from a modulation value in [0, 1].
    ///
    /// Exponential mapping: `min_freq × (max_freq / min_freq) ^ (mod × sensitivity)`
    #[inline]
    fn compute_freq(min_freq: f64, max_freq: f64, sensitivity: f64, modulation: f64) -> f64 {
        let ratio = max_freq / min_freq.max(1.0);
        min_freq * ratio.powf(modulation * sensitivity)
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for AutoFilter {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Auto-Filter",
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
        self.lfo.reset_phase();
        self.filter_left.reset();
        self.filter_right.reset();
    }

    // -- MIDI: no-op for an effect --
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

        let sr = self.sample_rate as f64;

        for i in 0..len {
            let l = in_l[i] as f64;
            let r = in_r[i] as f64;

            // Advance smoothers
            let min_f = self.min_freq_smooth.next();
            let max_f = self.max_freq_smooth.next();
            let q = self.resonance_smooth.next();
            let wet = self.dry_wet_smooth.next();

            // Compute modulation signal (0..1)
            let modulation = match self.source {
                ModSource::Envelope => {
                    // Use average of L/R envelope
                    let env_l = self.envelope_left.process(l.abs());
                    let env_r = self.envelope_right.process(r.abs());
                    ((env_l + env_r) * 0.5).clamp(0.0, 1.0)
                }
                ModSource::Lfo => {
                    // LFO output is -1..1, map to 0..1
                    let lfo_val = self.lfo.next(self.lfo_rate);
                    (lfo_val * 0.5 + 0.5).clamp(0.0, 1.0)
                }
            };

            // Compute modulated cutoff frequency
            let freq = Self::compute_freq(min_f, max_f, self.sensitivity, modulation);

            // Update biquad coefficients every N samples
            if i % COEFF_UPDATE_INTERVAL == 0 {
                let coeffs = design_filter(self.filter_type, sr, freq, q);
                self.filter_left.set_coeffs(coeffs);
                self.filter_right.set_coeffs(coeffs);
            }

            // Filter
            let filtered_l = self.filter_left.process(l);
            let filtered_r = self.filter_right.process(r);

            // Dry/wet mix
            out_l[i] = (l * (1.0 - wet) + filtered_l * wet) as f32;
            out_r[i] = (r * (1.0 - wet) + filtered_r * wet) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Auto-filter does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    // -- Parameters --
    // 0: source       (0=Envelope, 1=LFO)
    // 1: filter_type  (0=LP, 1=HP, 2=BP)
    // 2: min_freq     (20..5000)
    // 3: max_freq     (200..20000)
    // 4: resonance    (0.5..20)
    // 5: sensitivity  (0..1)
    // 6: attack_ms    (0.1..100)
    // 7: release_ms   (5..1000)
    // 8: lfo_rate     (0.05..20)
    // 9: lfo_shape    (0..4)
    // 10: dry_wet     (0..1)
    // 11: bypass      (0/1)

    fn param_count(&self) -> u32 {
        12
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Source".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Filter Type".into(),
                group: "Filter".into(),
                min: 0.0,
                max: 2.0,
                default: 0.0,
                step_count: 2,
                flags: ParamFlags::STEPPED,
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Min Frequency".into(),
                group: "Filter".into(),
                min: 20.0,
                max: 5000.0,
                default: 100.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Max Frequency".into(),
                group: "Filter".into(),
                min: 200.0,
                max: 20000.0,
                default: 5000.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "Resonance".into(),
                group: "Filter".into(),
                min: 0.5,
                max: 20.0,
                default: 2.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Sensitivity".into(),
                group: "Modulation".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Attack".into(),
                group: "Envelope".into(),
                min: 0.1,
                max: 100.0,
                default: 5.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            7 => Some(ParamInfo {
                id: 7,
                name: "Release".into(),
                group: "Envelope".into(),
                min: 5.0,
                max: 1000.0,
                default: 50.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            8 => Some(ParamInfo {
                id: 8,
                name: "LFO Rate".into(),
                group: "LFO".into(),
                min: 0.05,
                max: 20.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            9 => Some(ParamInfo {
                id: 9,
                name: "LFO Shape".into(),
                group: "LFO".into(),
                min: 0.0,
                max: 4.0,
                default: 0.0,
                step_count: 4,
                flags: ParamFlags::STEPPED,
            }),
            10 => Some(ParamInfo {
                id: 10,
                name: "Dry/Wet".into(),
                group: "Output".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            11 => Some(ParamInfo {
                id: 11,
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
            0 => Some(if self.source == ModSource::Lfo {
                1.0
            } else {
                0.0
            }),
            1 => Some(self.filter_type.to_value()),
            2 => Some(self.min_freq),
            3 => Some(self.max_freq),
            4 => Some(self.resonance),
            5 => Some(self.sensitivity),
            6 => Some(self.attack_ms),
            7 => Some(self.release_ms),
            8 => Some(self.lfo_rate),
            9 => Some(match self.lfo_shape {
                LfoShape::Sine => 0.0,
                LfoShape::Triangle => 1.0,
                LfoShape::Saw => 2.0,
                LfoShape::Square => 3.0,
                LfoShape::SampleAndHold => 4.0,
            }),
            10 => Some(self.dry_wet),
            11 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => self.source = ModSource::from_value(value),
            1 => self.filter_type = AutoFilterType::from_value(value),
            2 => {
                self.min_freq = value.clamp(20.0, 5000.0);
                self.min_freq_smooth.set_target(self.min_freq);
            }
            3 => {
                self.max_freq = value.clamp(200.0, 20000.0);
                self.max_freq_smooth.set_target(self.max_freq);
            }
            4 => {
                self.resonance = value.clamp(0.5, 20.0);
                self.resonance_smooth.set_target(self.resonance);
            }
            5 => self.sensitivity = value.clamp(0.0, 1.0),
            6 => {
                self.attack_ms = value.clamp(0.1, 100.0);
                self.update_envelope_coeffs();
            }
            7 => {
                self.release_ms = value.clamp(5.0, 1000.0);
                self.update_envelope_coeffs();
            }
            8 => self.lfo_rate = value.clamp(0.05, 20.0),
            9 => {
                self.lfo_shape = LfoShape::from_index(value.round() as u32);
                self.lfo.set_shape(self.lfo_shape);
            }
            10 => {
                self.dry_wet = value.clamp(0.0, 1.0);
                self.dry_wet_smooth.set_target(self.dry_wet);
            }
            11 => self.bypass = value >= 0.5,
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(if value >= 0.5 { "LFO" } else { "Envelope" }.into()),
            1 => {
                let i = value.round() as u32;
                Some(
                    match i {
                        0 => "LP",
                        1 => "HP",
                        _ => "BP",
                    }
                    .into(),
                )
            }
            2 => Some(format!("{:.0} Hz", value)),
            3 => Some(format!("{:.0} Hz", value)),
            4 => Some(format!("{:.1}", value)),
            5 => Some(format!("{:.0}%", value * 100.0)),
            6 => Some(format!("{:.1} ms", value)),
            7 => Some(format!("{:.0} ms", value)),
            8 => Some(format!("{:.2} Hz", value)),
            9 => {
                let i = value.round() as u32;
                Some(
                    match i {
                        0 => "Sine",
                        1 => "Tri",
                        2 => "Saw",
                        3 => "Sq",
                        _ => "S&H",
                    }
                    .into(),
                )
            }
            10 => Some(format!("{:.0}%", value * 100.0)),
            11 => Some(if value >= 0.5 {
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
    use std::f64::consts::PI;

    /// Generate a mono sine wave.
    fn sine_wave(freq: f64, amplitude: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (amplitude * (2.0 * PI * freq * t).sin()) as f32
            })
            .collect()
    }

    /// Measure RMS of a buffer.
    fn rms(buf: &[f32]) -> f64 {
        let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum_sq / buf.len() as f64).sqrt()
    }

    // -----------------------------------------------------------------------
    // test_bypass_is_bitexact
    // -----------------------------------------------------------------------

    #[test]
    fn test_bypass_is_bitexact() {
        let mut af = AutoFilter::new(44100);
        af.set_param(11, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        af.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
    // test_param_round_trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_round_trip() {
        let mut af = AutoFilter::new(44100);

        let test_values: [(u32, f64); 12] = [
            (0, 1.0),    // source = LFO
            (1, 2.0),    // filter_type = BP
            (2, 500.0),  // min_freq
            (3, 8000.0), // max_freq
            (4, 5.0),    // resonance
            (5, 0.8),    // sensitivity
            (6, 20.0),   // attack_ms
            (7, 200.0),  // release_ms
            (8, 3.0),    // lfo_rate
            (9, 2.0),    // lfo_shape = Saw
            (10, 0.5),   // dry_wet
            (11, 1.0),   // bypass
        ];

        for (id, value) in &test_values {
            af.set_param(*id, *value);
            let got = af.get_param(*id).unwrap();
            assert!(
                (got - value).abs() < 1e-10,
                "param {} round-trip: set {}, got {}",
                id,
                value,
                got
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_envelope_responds_to_amplitude
    // -----------------------------------------------------------------------

    #[test]
    fn test_envelope_responds_to_amplitude() {
        let sr = 44100u32;
        let mut af = AutoFilter::new(sr);

        // source = Envelope, filter_type = LP, high sensitivity
        af.set_param(0, 0.0); // Envelope
        af.set_param(1, 0.0); // LP
        af.set_param(2, 100.0); // min_freq = 100 Hz
        af.set_param(3, 10000.0); // max_freq = 10000 Hz
        af.set_param(4, 1.0); // resonance = moderate
        af.set_param(5, 1.0); // sensitivity = max
        af.set_param(6, 1.0); // fast attack
        af.set_param(7, 50.0); // moderate release
        af.set_param(10, 1.0); // 100% wet

        let num_samples = sr as usize; // 1 second

        // Process a quiet signal (should keep filter closed = low cutoff = less HF)
        let quiet = sine_wave(5000.0, 0.01, sr, num_samples);
        let mut out_l_quiet = vec![0.0f32; num_samples];
        let mut out_r_quiet = vec![0.0f32; num_samples];
        af.process_effect(&quiet, &quiet, &mut out_l_quiet, &mut out_r_quiet);

        // Reset filter state for fair comparison
        af.unload();

        // Process a loud signal (should open filter = high cutoff = more HF passes)
        let loud = sine_wave(5000.0, 0.8, sr, num_samples);
        let mut out_l_loud = vec![0.0f32; num_samples];
        let mut out_r_loud = vec![0.0f32; num_samples];
        af.process_effect(&loud, &loud, &mut out_l_loud, &mut out_r_loud);

        // Measure output RMS in the second half (after envelope settles)
        let half = num_samples / 2;
        let rms_quiet = rms(&out_l_quiet[half..]);
        let rms_loud = rms(&out_l_loud[half..]);

        // Loud signal should produce more output through LP filter
        // because envelope opens the cutoff, letting 5 kHz pass
        assert!(
            rms_loud > rms_quiet * 2.0,
            "loud signal should produce significantly more output through LP: \
             rms_loud={:.6}, rms_quiet={:.6}",
            rms_loud,
            rms_quiet
        );
    }

    // -----------------------------------------------------------------------
    // test_lfo_sweeps
    // -----------------------------------------------------------------------

    #[test]
    fn test_lfo_sweeps() {
        let sr = 44100u32;
        let mut af = AutoFilter::new(sr);

        // source = LFO, slow rate for clear variation
        af.set_param(0, 1.0); // LFO
        af.set_param(1, 0.0); // LP
        af.set_param(2, 200.0); // min_freq
        af.set_param(3, 8000.0); // max_freq
        af.set_param(4, 2.0); // resonance
        af.set_param(5, 1.0); // sensitivity
        af.set_param(8, 2.0); // lfo_rate = 2 Hz
        af.set_param(9, 0.0); // Sine
        af.set_param(10, 1.0); // 100% wet

        // Feed white-ish noise (deterministic) through the filter
        let num_samples = sr as usize; // 1 second
        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                // Simple deterministic pseudo-noise
                let phase = i as f64 * 0.123456789;
                ((phase * 17.0).sin() * 0.5) as f32
            })
            .collect();

        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        af.process_effect(&input, &input, &mut out_l, &mut out_r);

        // Split output into 4 quarters and compare RMS — they should differ
        // because the LFO sweeps the cutoff frequency
        let quarter = num_samples / 4;
        let rms_values: Vec<f64> = (0..4)
            .map(|q| rms(&out_l[q * quarter..(q + 1) * quarter]))
            .collect();

        // At least some quarters should differ by > 10%
        let mut found_variation = false;
        for i in 0..4 {
            for j in (i + 1)..4 {
                let ratio = if rms_values[i] > rms_values[j] {
                    rms_values[i] / rms_values[j].max(1e-10)
                } else {
                    rms_values[j] / rms_values[i].max(1e-10)
                };
                if ratio > 1.1 {
                    found_variation = true;
                }
            }
        }

        assert!(
            found_variation,
            "LFO should cause variation in output level across quarters: {:?}",
            rms_values
        );
    }

    // -----------------------------------------------------------------------
    // test_param_info_complete
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_info_complete() {
        let af = AutoFilter::new(44100);
        assert_eq!(af.param_count(), 12);

        for i in 0..12 {
            let info = af.param_info(i);
            assert!(info.is_some(), "param_info({}) should return Some", i);
            let info = info.unwrap();
            assert_eq!(info.id, i, "param_info({}).id mismatch", i);
            assert!(!info.name.is_empty(), "param_info({}).name is empty", i);
        }

        // Out of range should return None
        assert!(af.param_info(12).is_none());
        assert!(af.param_info(100).is_none());
    }
}
