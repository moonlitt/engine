//! Multiband compressor with Linkwitz-Riley 4th-order crossover.
//!
//! Splits the stereo input into 1–6 frequency bands using LR4 crossovers
//! (two cascaded Butterworth biquads per split), applies independent
//! compression to each band, then sums the bands back together.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core`.
//!
//! ## Parameters (38 total)
//!
//! | ID   | Name              | Range           | Default  |
//! |------|-------------------|-----------------|----------|
//! | 0    | Band Count        | 1..6 (stepped)  | 4        |
//! | 1    | Output Gain (dB)  | -24..24         | 0        |
//! | 2    | Bypass            | 0/1             | 0        |
//! | 3–7  | Crossover 1–5     | 20..20000 Hz    | 100/500/2k/8k/16k |
//! | 8–37 | Per-band params   | (see below)     |          |
//!
//! Per-band (6 bands × 5 params, band N starts at 8 + N*5):
//!   +0 threshold_db, +1 ratio, +2 attack_ms, +3 release_ms, +4 makeup_db

use super::envelope::EnvelopeFollower;
use crate::common::DbLut;
use crate::eq::biquad::{Biquad, BiquadCoeffs, FilterType};
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_BANDS: usize = 6;
const MAX_CROSSOVERS: usize = MAX_BANDS - 1; // 5
const PARAM_COUNT: u32 = 38;

// Butterworth Q for LR4 (two cascaded = Linkwitz-Riley 4th order)
const BUTTERWORTH_Q: f64 = std::f64::consts::FRAC_1_SQRT_2; // 1/sqrt(2) ≈ 0.7071

// Default crossover frequencies
const DEFAULT_CROSSOVER_FREQS: [f64; MAX_CROSSOVERS] = [100.0, 500.0, 2000.0, 8000.0, 16000.0];

// Per-band parameter defaults
const DEFAULT_THRESHOLD_DB: f64 = -20.0;
const DEFAULT_RATIO: f64 = 4.0;
const DEFAULT_ATTACK_MS: f64 = 10.0;
const DEFAULT_RELEASE_MS: f64 = 100.0;
const DEFAULT_MAKEUP_DB: f64 = 0.0;

// ---------------------------------------------------------------------------
// LR4 Crossover — one split point
// ---------------------------------------------------------------------------

/// A single Linkwitz-Riley 4th-order crossover split.
///
/// Produces a lowpass and highpass output. Each is two cascaded Butterworth
/// 2nd-order biquads, per channel (stereo = 8 biquads total).
#[derive(Clone)]
struct Lr4Crossover {
    /// LP stage 1 and 2, per channel [L, R]
    lp: [[Biquad; 2]; 2],
    /// HP stage 1 and 2, per channel [L, R]
    hp: [[Biquad; 2]; 2],
    freq: f64,
}

impl Lr4Crossover {
    fn new(sample_rate: f64, freq: f64) -> Self {
        let mut xover = Self {
            lp: [[Biquad::new(), Biquad::new()], [Biquad::new(), Biquad::new()]],
            hp: [[Biquad::new(), Biquad::new()], [Biquad::new(), Biquad::new()]],
            freq,
        };
        xover.update_coeffs(sample_rate, freq);
        xover
    }

    fn update_coeffs(&mut self, sample_rate: f64, freq: f64) {
        self.freq = freq;
        let lp_coeffs = BiquadCoeffs::design(FilterType::Lowpass, sample_rate, freq, 0.0, BUTTERWORTH_Q);
        let hp_coeffs = BiquadCoeffs::design(FilterType::Highpass, sample_rate, freq, 0.0, BUTTERWORTH_Q);

        for ch in 0..2 {
            self.lp[ch][0].set_coeffs(lp_coeffs);
            self.lp[ch][1].set_coeffs(lp_coeffs);
            self.hp[ch][0].set_coeffs(hp_coeffs);
            self.hp[ch][1].set_coeffs(hp_coeffs);
        }
    }

    /// Split a single stereo sample pair into (low_l, low_r, high_l, high_r).
    #[inline]
    fn process(&mut self, l: f64, r: f64) -> (f64, f64, f64, f64) {
        // LP: cascade two biquads (use temporaries to avoid double borrow)
        let tmp_lp_l = self.lp[0][0].process(l);
        let lp_l = self.lp[0][1].process(tmp_lp_l);
        let tmp_lp_r = self.lp[1][0].process(r);
        let lp_r = self.lp[1][1].process(tmp_lp_r);
        // HP: cascade two biquads
        let tmp_hp_l = self.hp[0][0].process(l);
        let hp_l = self.hp[0][1].process(tmp_hp_l);
        let tmp_hp_r = self.hp[1][0].process(r);
        let hp_r = self.hp[1][1].process(tmp_hp_r);
        (lp_l, lp_r, hp_l, hp_r)
    }

    fn reset(&mut self) {
        for ch in 0..2 {
            self.lp[ch][0].reset();
            self.lp[ch][1].reset();
            self.hp[ch][0].reset();
            self.hp[ch][1].reset();
        }
    }
}

// ---------------------------------------------------------------------------
// Per-band compression state
// ---------------------------------------------------------------------------

struct BandCompressor {
    threshold_db: f64,
    ratio: f64,
    attack_ms: f64,
    release_ms: f64,
    makeup_db: f64,
    envelope_l: EnvelopeFollower,
    envelope_r: EnvelopeFollower,
}

impl BandCompressor {
    fn new(sample_rate: f64) -> Self {
        let mut bc = Self {
            threshold_db: DEFAULT_THRESHOLD_DB,
            ratio: DEFAULT_RATIO,
            attack_ms: DEFAULT_ATTACK_MS,
            release_ms: DEFAULT_RELEASE_MS,
            makeup_db: DEFAULT_MAKEUP_DB,
            envelope_l: EnvelopeFollower::new(sample_rate),
            envelope_r: EnvelopeFollower::new(sample_rate),
        };
        bc.update_envelope_coeffs();
        bc
    }

    fn update_envelope_coeffs(&mut self) {
        self.envelope_l.set_attack_ms(self.attack_ms);
        self.envelope_l.set_release_ms(self.release_ms);
        self.envelope_r.set_attack_ms(self.attack_ms);
        self.envelope_r.set_release_ms(self.release_ms);
    }

    /// Process one stereo sample pair and return the compressed output.
    #[inline]
    fn process(&mut self, l: f64, r: f64, db_lut: &DbLut) -> (f64, f64) {
        let out_l = Self::process_channel(
            l,
            self.threshold_db,
            self.ratio,
            self.makeup_db,
            &mut self.envelope_l,
            db_lut,
        );
        let out_r = Self::process_channel(
            r,
            self.threshold_db,
            self.ratio,
            self.makeup_db,
            &mut self.envelope_r,
            db_lut,
        );
        (out_l, out_r)
    }

    /// Process a single channel sample through detection + gain application.
    #[inline]
    fn process_channel(
        sample: f64,
        threshold_db: f64,
        ratio: f64,
        makeup_db: f64,
        envelope: &mut EnvelopeFollower,
        db_lut: &DbLut,
    ) -> f64 {
        let detected = sample.abs();
        let level_db = amp_to_db(detected);
        let gr = compute_gain_db(threshold_db, ratio, level_db);
        let gr_mag = (-gr).max(0.0);
        let smoothed_gr = envelope.process(gr_mag);
        let total_db = -smoothed_gr + makeup_db;
        let gain = db_lut.db_to_linear(total_db);
        sample * gain
    }

    fn reset(&mut self) {
        self.envelope_l.reset();
        self.envelope_r.reset();
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Convert amplitude to dB, flooring at -120 dB.
#[inline]
fn amp_to_db(amp: f64) -> f64 {
    if amp > 1e-6 {
        20.0 * amp.log10()
    } else {
        -120.0
    }
}

/// Convert dB to linear amplitude.
#[inline]
#[allow(dead_code)]
fn db_to_amp(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// Compute gain reduction in dB (hard knee, no soft knee for multiband).
#[inline]
fn compute_gain_db(threshold: f64, ratio: f64, level_db: f64) -> f64 {
    if level_db <= threshold {
        0.0
    } else {
        (threshold - level_db) * (1.0 - 1.0 / ratio)
    }
}

// ---------------------------------------------------------------------------
// MultibandCompressor
// ---------------------------------------------------------------------------

/// Multiband compressor with 1–6 bands and LR4 crossover network.
pub struct MultibandCompressor {
    sample_rate: u32,

    // Global parameters
    band_count: usize,
    output_db: f64,
    bypass: bool,

    // Crossover frequencies (ascending order enforced)
    crossover_freqs: [f64; MAX_CROSSOVERS],

    // Crossover filters
    crossovers: Vec<Lr4Crossover>,

    // Per-band compressors
    bands: Vec<BandCompressor>,

    // dB→linear lookup table
    db_lut: DbLut,
}

impl MultibandCompressor {
    /// Create a new multiband compressor with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;

        let crossovers: Vec<Lr4Crossover> = DEFAULT_CROSSOVER_FREQS
            .iter()
            .map(|&freq| Lr4Crossover::new(sr, freq))
            .collect();

        let bands: Vec<BandCompressor> = (0..MAX_BANDS)
            .map(|_| BandCompressor::new(sr))
            .collect();

        Self {
            sample_rate,
            band_count: 4,
            output_db: 0.0,
            bypass: false,
            crossover_freqs: DEFAULT_CROSSOVER_FREQS,
            crossovers,
            bands,
            db_lut: DbLut::new(),
        }
    }

    /// Update crossover filter coefficients for a specific crossover index.
    fn update_crossover(&mut self, index: usize) {
        let sr = self.sample_rate as f64;
        let freq = self.crossover_freqs[index];
        self.crossovers[index].update_coeffs(sr, freq);
    }

    /// Enforce ascending order on crossover frequencies.
    /// After setting crossover[index], clamp neighbors to maintain order.
    fn enforce_ascending_crossovers(&mut self, index: usize) {
        // Clamp upward: each subsequent crossover must be >= previous
        for i in (index + 1)..MAX_CROSSOVERS {
            if self.crossover_freqs[i] < self.crossover_freqs[i - 1] {
                self.crossover_freqs[i] = self.crossover_freqs[i - 1];
                self.update_crossover(i);
            }
        }
        // Clamp downward: each previous crossover must be <= next
        if index > 0 {
            for i in (0..index).rev() {
                if self.crossover_freqs[i] > self.crossover_freqs[i + 1] {
                    self.crossover_freqs[i] = self.crossover_freqs[i + 1];
                    self.update_crossover(i);
                }
            }
        }
    }

    /// Split input into bands, compress each, and sum.
    #[allow(clippy::needless_range_loop)]
    fn process_sample(&mut self, l: f64, r: f64) -> (f64, f64) {
        let n = self.band_count;
        let db_lut = &self.db_lut;

        if n == 1 {
            // Single band: no crossover, just compress
            return self.bands[0].process(l, r, db_lut);
        }

        // Split into bands using cascaded crossovers.
        // band_signals[i] = (l, r) for band i
        let mut band_signals = [(0.0f64, 0.0f64); MAX_BANDS];

        // The splitting chain:
        // input → crossover[0] → band[0] = LP, rest = HP
        // rest → crossover[1] → band[1] = LP, rest = HP
        // ...
        // final rest = band[n-1]
        let mut rest_l = l;
        let mut rest_r = r;

        for i in 0..(n - 1) {
            let (lp_l, lp_r, hp_l, hp_r) = self.crossovers[i].process(rest_l, rest_r);
            band_signals[i] = (lp_l, lp_r);
            rest_l = hp_l;
            rest_r = hp_r;
        }
        band_signals[n - 1] = (rest_l, rest_r);

        // Compress each band and sum
        let mut sum_l = 0.0;
        let mut sum_r = 0.0;
        for i in 0..n {
            let (cl, cr) = self.bands[i].process(band_signals[i].0, band_signals[i].1, db_lut);
            sum_l += cl;
            sum_r += cr;
        }

        (sum_l, sum_r)
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for MultibandCompressor {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Multiband Compressor",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        for xover in &mut self.crossovers {
            xover.reset();
        }
        for band in &mut self.bands {
            band.reset();
        }
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

        let output_gain = self.db_lut.db_to_linear(self.output_db);

        for i in 0..len {
            let l = in_l[i] as f64;
            let r = in_r[i] as f64;

            let (ol, or) = self.process_sample(l, r);

            out_l[i] = (ol * output_gain) as f32;
            out_r[i] = (or * output_gain) as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {}

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    fn param_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            // Global params
            0 => Some(ParamInfo {
                id: 0,
                name: "Band Count".into(),
                group: "Global".into(),
                min: 1.0,
                max: 6.0,
                default: 4.0,
                step_count: 5,
                flags: ParamFlags::STEPPED,
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Output Gain".into(),
                group: "Global".into(),
                min: -24.0,
                max: 24.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Bypass".into(),
                group: "Global".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),

            // Crossover frequencies (IDs 3–7)
            3..=7 => {
                let xover_idx = (index - 3) as usize;
                Some(ParamInfo {
                    id: index,
                    name: format!("Crossover {}", xover_idx + 1),
                    group: "Crossover".into(),
                    min: 20.0,
                    max: 20000.0,
                    default: DEFAULT_CROSSOVER_FREQS[xover_idx],
                    step_count: 0,
                    flags: ParamFlags::empty(),
                })
            }

            // Per-band params (IDs 8–37)
            8..=37 => {
                let offset = (index - 8) as usize;
                let band_idx = offset / 5;
                let param_in_band = offset % 5;

                let (name, min, max, default, step_count, flags) = match param_in_band {
                    0 => ("Threshold", -60.0, 0.0, DEFAULT_THRESHOLD_DB, 0, ParamFlags::empty()),
                    1 => ("Ratio", 1.0, 100.0, DEFAULT_RATIO, 0, ParamFlags::empty()),
                    2 => ("Attack", 0.1, 100.0, DEFAULT_ATTACK_MS, 0, ParamFlags::empty()),
                    3 => ("Release", 10.0, 1000.0, DEFAULT_RELEASE_MS, 0, ParamFlags::empty()),
                    4 => ("Makeup", -12.0, 24.0, DEFAULT_MAKEUP_DB, 0, ParamFlags::empty()),
                    _ => return None,
                };

                Some(ParamInfo {
                    id: index,
                    name: format!("Band {} {}", band_idx + 1, name),
                    group: format!("Band {}", band_idx + 1),
                    min,
                    max,
                    default,
                    step_count,
                    flags,
                })
            }

            _ => None,
        }
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            0 => Some(self.band_count as f64),
            1 => Some(self.output_db),
            2 => Some(if self.bypass { 1.0 } else { 0.0 }),

            3..=7 => {
                let idx = (id - 3) as usize;
                Some(self.crossover_freqs[idx])
            }

            8..=37 => {
                let offset = (id - 8) as usize;
                let band_idx = offset / 5;
                let param_in_band = offset % 5;
                let band = &self.bands[band_idx];
                match param_in_band {
                    0 => Some(band.threshold_db),
                    1 => Some(band.ratio),
                    2 => Some(band.attack_ms),
                    3 => Some(band.release_ms),
                    4 => Some(band.makeup_db),
                    _ => None,
                }
            }

            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.band_count = (value.round() as usize).clamp(1, MAX_BANDS);
            }
            1 => {
                self.output_db = value.clamp(-24.0, 24.0);
            }
            2 => {
                self.bypass = value >= 0.5;
            }

            3..=7 => {
                let idx = (id - 3) as usize;
                self.crossover_freqs[idx] = value.clamp(20.0, 20000.0);
                self.update_crossover(idx);
                self.enforce_ascending_crossovers(idx);
            }

            8..=37 => {
                let offset = (id - 8) as usize;
                let band_idx = offset / 5;
                let param_in_band = offset % 5;
                let band = &mut self.bands[band_idx];
                match param_in_band {
                    0 => band.threshold_db = value.clamp(-60.0, 0.0),
                    1 => band.ratio = value.clamp(1.0, 100.0),
                    2 => {
                        band.attack_ms = value.clamp(0.1, 100.0);
                        band.update_envelope_coeffs();
                    }
                    3 => {
                        band.release_ms = value.clamp(10.0, 1000.0);
                        band.update_envelope_coeffs();
                    }
                    4 => band.makeup_db = value.clamp(-12.0, 24.0),
                    _ => {}
                }
            }

            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{}", value.round() as u32)),
            1 => Some(format!("{:.1} dB", value)),
            2 => Some(if value >= 0.5 { "On".into() } else { "Off".into() }),

            3..=7 => {
                if value >= 1000.0 {
                    Some(format!("{:.1} kHz", value / 1000.0))
                } else {
                    Some(format!("{:.0} Hz", value))
                }
            }

            8..=37 => {
                let offset = (id - 8) as usize;
                let param_in_band = offset % 5;
                match param_in_band {
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
                    _ => None,
                }
            }

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
        let mut mb = MultibandCompressor::new(44100);
        mb.set_param(2, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        mb.process_effect(&input, &silent, &mut out_l, &mut out_r);

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
        let mut mb = MultibandCompressor::new(44100);

        // Global params
        mb.set_param(0, 3.0);
        assert_eq!(mb.get_param(0), Some(3.0));

        mb.set_param(1, 6.5);
        assert_eq!(mb.get_param(1), Some(6.5));

        mb.set_param(2, 1.0);
        assert_eq!(mb.get_param(2), Some(1.0));

        // Crossover freqs
        mb.set_param(3, 200.0);
        assert_eq!(mb.get_param(3), Some(200.0));

        mb.set_param(4, 1000.0);
        assert_eq!(mb.get_param(4), Some(1000.0));

        mb.set_param(5, 3000.0);
        assert_eq!(mb.get_param(5), Some(3000.0));

        mb.set_param(6, 10000.0);
        assert_eq!(mb.get_param(6), Some(10000.0));

        mb.set_param(7, 18000.0);
        assert_eq!(mb.get_param(7), Some(18000.0));

        // Per-band params (band 0)
        mb.set_param(8, -30.0);  // threshold
        assert_eq!(mb.get_param(8), Some(-30.0));

        mb.set_param(9, 8.0);   // ratio
        assert_eq!(mb.get_param(9), Some(8.0));

        mb.set_param(10, 5.0);  // attack
        assert_eq!(mb.get_param(10), Some(5.0));

        mb.set_param(11, 200.0); // release
        assert_eq!(mb.get_param(11), Some(200.0));

        mb.set_param(12, 3.0);  // makeup
        assert_eq!(mb.get_param(12), Some(3.0));

        // Per-band params (band 5, last)
        mb.set_param(33, -40.0); // threshold
        assert_eq!(mb.get_param(33), Some(-40.0));

        mb.set_param(34, 2.0);  // ratio
        assert_eq!(mb.get_param(34), Some(2.0));

        mb.set_param(35, 20.0); // attack
        assert_eq!(mb.get_param(35), Some(20.0));

        mb.set_param(36, 500.0); // release
        assert_eq!(mb.get_param(36), Some(500.0));

        mb.set_param(37, -6.0); // makeup
        assert_eq!(mb.get_param(37), Some(-6.0));

        // Clamping: band_count
        mb.set_param(0, 0.0);
        assert_eq!(mb.get_param(0), Some(1.0));
        mb.set_param(0, 10.0);
        assert_eq!(mb.get_param(0), Some(6.0));

        // Clamping: output_db
        mb.set_param(1, -50.0);
        assert_eq!(mb.get_param(1), Some(-24.0));
        mb.set_param(1, 50.0);
        assert_eq!(mb.get_param(1), Some(24.0));

        // Invalid param
        assert_eq!(mb.get_param(99), None);
        assert!(mb.param_info(38).is_none());
    }

    // -----------------------------------------------------------------------
    // test_crossover_sum_is_flat
    // -----------------------------------------------------------------------

    #[test]
    fn test_crossover_sum_is_flat() {
        let sr = 44100u32;
        let mut mb = MultibandCompressor::new(sr);

        // Disable compression: threshold=0, ratio=1 (no gain reduction)
        let n_bands = 4;
        mb.set_param(0, n_bands as f64);
        mb.set_param(1, 0.0); // output gain = 0 dB

        for band in 0..MAX_BANDS {
            let base = 8 + band * 5;
            mb.set_param(base as u32, 0.0);      // threshold = 0 dB
            mb.set_param((base + 1) as u32, 1.0); // ratio = 1:1 (no compression)
            mb.set_param((base + 2) as u32, 0.1); // fast attack
            mb.set_param((base + 3) as u32, 10.0); // fast release
            mb.set_param((base + 4) as u32, 0.0); // no makeup
        }

        // Test with sines at various frequencies
        let test_freqs = [50.0, 200.0, 1000.0, 5000.0, 15000.0];
        let num_samples = sr as usize * 2; // 2 seconds for settling
        let amplitude = 0.5;

        for &freq in &test_freqs {
            // Reset state
            mb.unload();

            let input = sine_wave(freq, amplitude, sr, num_samples);
            let mut out_l = vec![0.0f32; num_samples];
            let mut out_r = vec![0.0f32; num_samples];

            mb.process_effect(&input, &input, &mut out_l, &mut out_r);

            // Measure RMS in the last quarter (after filters have settled)
            let measure_start = num_samples * 3 / 4;
            let input_rms = rms_db(&input[measure_start..]);
            let output_rms = rms_db(&out_l[measure_start..]);
            let deviation = (output_rms - input_rms).abs();

            assert!(
                deviation < 0.5,
                "crossover sum not flat at {} Hz: deviation = {:.3} dB (in={:.3}, out={:.3})",
                freq, deviation, input_rms, output_rms
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_single_band_equals_fullband
    // -----------------------------------------------------------------------

    #[test]
    fn test_single_band_equals_fullband() {
        let sr = 44100u32;
        let mut mb = MultibandCompressor::new(sr);

        // 1 band = fullband compressor
        mb.set_param(0, 1.0); // band_count = 1
        mb.set_param(1, 0.0); // output gain = 0

        // Set compression: threshold=-20, ratio=4:1
        mb.set_param(8, -20.0);  // threshold
        mb.set_param(9, 4.0);    // ratio
        mb.set_param(10, 0.1);   // fast attack
        mb.set_param(11, 1000.0); // slow release
        mb.set_param(12, 0.0);   // no makeup

        // Input at -10 dB (10 dB above threshold)
        let amplitude = 10.0_f64.powf(-10.0 / 20.0);
        let num_samples = sr as usize * 4;
        let input = sine_wave(1000.0, amplitude, sr, num_samples);
        let silent = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        mb.process_effect(&input, &silent, &mut out_l, &mut out_r);

        // Expected gain reduction: (threshold - level_db) * (1 - 1/ratio)
        // = (-20 - (-10)) * (1 - 1/4) = -10 * 0.75 = -7.5 dB
        let expected_gain_db = -7.5;

        let measure_start = sr as usize * 3;
        let input_rms = rms_db(&input[measure_start..]);
        let output_rms = rms_db(&out_l[measure_start..]);
        let measured_gain = output_rms - input_rms;

        let error = (measured_gain - expected_gain_db).abs();
        assert!(
            error < 0.5,
            "single-band compression: expected {:.1} dB gain, got {:.4} dB (error {:.4} dB)",
            expected_gain_db, measured_gain, error
        );
    }

    // -----------------------------------------------------------------------
    // test_param_info_complete
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_info_complete() {
        let mb = MultibandCompressor::new(44100);
        assert_eq!(mb.param_count(), 38);

        for i in 0..38 {
            let info = mb.param_info(i);
            assert!(
                info.is_some(),
                "param_info({}) should exist",
                i
            );
            let info = info.unwrap();
            assert_eq!(info.id, i, "param_info({}).id mismatch", i);
            assert!(!info.name.is_empty(), "param_info({}).name is empty", i);
            assert!(info.min <= info.default, "param_info({}).min > default", i);
            assert!(info.default <= info.max, "param_info({}).default > max", i);
        }

        // Beyond 38 should be None
        assert!(mb.param_info(38).is_none());
        assert!(mb.param_info(100).is_none());
    }

    // -----------------------------------------------------------------------
    // test_crossover_ascending_enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn test_crossover_ascending_enforcement() {
        let mut mb = MultibandCompressor::new(44100);

        // Set crossover 3 to a low value; crossovers 1 and 2 should be clamped down
        mb.set_param(5, 50.0); // crossover 3 = 50 Hz

        // Crossover 1 and 2 should be <= 50
        let xover1 = mb.get_param(3).unwrap();
        let xover2 = mb.get_param(4).unwrap();
        let xover3 = mb.get_param(5).unwrap();
        assert!(xover1 <= xover2, "xover1 ({}) > xover2 ({})", xover1, xover2);
        assert!(xover2 <= xover3, "xover2 ({}) > xover3 ({})", xover2, xover3);
    }
}
