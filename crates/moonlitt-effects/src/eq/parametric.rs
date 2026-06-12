//! 8-band parametric EQ with `AudioBackend` implementation.
//!
//! Each band is an independent biquad filter. Enabled bands are cascaded
//! (series connection) per channel. Coefficient recalculation happens at
//! parameter-change time, never in the audio loop.

use super::biquad::{Biquad, BiquadCoeffs, FilterType};
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Band descriptor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Band {
    pub filter_type: FilterType,
    pub frequency: f64,
    pub gain_db: f64,
    pub q: f64,
    pub enabled: bool,
}

impl Default for Band {
    fn default() -> Self {
        Self {
            filter_type: FilterType::Peak,
            frequency: 1000.0,
            gain_db: 0.0,
            q: 1.0,
            enabled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Default frequency layout (classic console EQ)
// ---------------------------------------------------------------------------

const DEFAULT_FREQS: [f64; 8] = [60.0, 170.0, 400.0, 1000.0, 2500.0, 6000.0, 12000.0, 16000.0];

// ---------------------------------------------------------------------------
// ParametricEq
// ---------------------------------------------------------------------------

pub struct ParametricEq {
    sample_rate: u32,
    bands: [Band; 8],
    filters_left: [Biquad; 8],
    filters_right: [Biquad; 8],
    bypass: bool,
}

impl ParametricEq {
    pub fn new(sample_rate: u32) -> Self {
        let bands: [Band; 8] = std::array::from_fn(|i| Band {
            frequency: DEFAULT_FREQS[i],
            ..Band::default()
        });

        Self {
            sample_rate,
            bands,
            filters_left: std::array::from_fn(|_| Biquad::new()),
            filters_right: std::array::from_fn(|_| Biquad::new()),
            bypass: false,
        }
    }

    /// Set a complete band configuration and recompute coefficients.
    pub fn set_band(&mut self, index: usize, band: Band) {
        assert!(index < 8);
        self.bands[index] = band;
        self.recompute_band(index);
    }

    /// Recompute biquad coefficients for a single band.
    fn recompute_band(&mut self, index: usize) {
        let b = &self.bands[index];
        let coeffs = if b.enabled {
            BiquadCoeffs::design(
                b.filter_type,
                self.sample_rate as f64,
                b.frequency,
                b.gain_db,
                b.q,
            )
        } else {
            BiquadCoeffs::passthrough()
        };
        self.filters_left[index].set_coeffs(coeffs);
        self.filters_right[index].set_coeffs(coeffs);
    }

    /// Check if any band is enabled.
    fn any_band_enabled(&self) -> bool {
        self.bands.iter().any(|b| b.enabled)
    }
}

// ---------------------------------------------------------------------------
// AudioBackend
// ---------------------------------------------------------------------------

impl AudioBackend for ParametricEq {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Parametric EQ",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Built-in effect, nothing to load.
        Ok(())
    }

    fn unload(&mut self) {
        // Reset all filter state.
        for i in 0..8 {
            self.filters_left[i].reset();
            self.filters_right[i].reset();
        }
    }

    // -- MIDI: no-op for an EQ effect --
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
        if self.bypass || !self.any_band_enabled() {
            out_l[..len].copy_from_slice(&in_l[..len]);
            out_r[..len].copy_from_slice(&in_r[..len]);
            return;
        }

        for i in 0..len {
            let mut l = in_l[i] as f64;
            let mut r = in_r[i] as f64;

            for band_idx in 0..8 {
                if self.bands[band_idx].enabled {
                    l = self.filters_left[band_idx].process(l);
                    r = self.filters_right[band_idx].process(r);
                }
            }

            out_l[i] = l as f32;
            out_r[i] = r as f32;
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // EQ does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0 // IIR — zero latency
    }

    // -- Parameters --
    // Layout: band*4+0 = freq, band*4+1 = gain, band*4+2 = Q, band*4+3 = type
    //         32 = bypass

    fn param_count(&self) -> u32 {
        33
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        if index >= 33 {
            return None;
        }

        if index == 32 {
            return Some(ParamInfo {
                id: 32,
                name: "Bypass".into(),
                group: "Global".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            });
        }

        let band = index / 4;
        let sub = index % 4;
        let band_name = format!("Band {}", band + 1);

        match sub {
            0 => Some(ParamInfo {
                id: index,
                name: format!("{} Frequency", band_name),
                group: band_name,
                min: 20.0,
                max: 20000.0,
                default: DEFAULT_FREQS[band as usize],
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            1 => Some(ParamInfo {
                id: index,
                name: format!("{} Gain", band_name),
                group: band_name,
                min: -24.0,
                max: 24.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: index,
                name: format!("{} Q", band_name),
                group: band_name,
                min: 0.1,
                max: 18.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            3 => Some(ParamInfo {
                id: index,
                name: format!("{} Type", band_name),
                group: band_name,
                min: 0.0,
                max: 5.0,
                default: 0.0,
                step_count: 5,
                flags: ParamFlags::STEPPED,
            }),
            _ => unreachable!(),
        }
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        if id == 32 {
            return Some(if self.bypass { 1.0 } else { 0.0 });
        }
        if id >= 32 {
            return None;
        }

        let band = id as usize / 4;
        let sub = id % 4;
        let b = &self.bands[band];

        match sub {
            0 => Some(b.frequency),
            1 => Some(b.gain_db),
            2 => Some(b.q),
            3 => Some(b.filter_type.to_index() as f64),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if id == 32 {
            self.bypass = value >= 0.5;
            return;
        }
        if id >= 32 {
            return;
        }

        let band = id as usize / 4;
        let sub = id % 4;

        match sub {
            0 => self.bands[band].frequency = value.clamp(20.0, 20000.0),
            1 => self.bands[band].gain_db = value.clamp(-24.0, 24.0),
            2 => self.bands[band].q = value.clamp(0.1, 18.0),
            3 => self.bands[band].filter_type = FilterType::from_index(value as u32),
            _ => return,
        }

        // Recalculate coefficients for the changed band.
        self.recompute_band(band);
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        if id == 32 {
            return Some(if value >= 0.5 {
                "On".into()
            } else {
                "Off".into()
            });
        }
        if id >= 32 {
            return None;
        }

        let sub = id % 4;
        match sub {
            0 => Some(format!("{:.0} Hz", value)),
            1 => Some(format!("{:+.1} dB", value)),
            2 => Some(format!("{:.2}", value)),
            3 => {
                let ft = FilterType::from_index(value as u32);
                Some(format!("{:?}", ft))
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

    /// Generate a mono sine wave at the given frequency.
    fn sine_wave(freq: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * PI * freq * t).sin() as f32
            })
            .collect()
    }

    /// Measure RMS amplitude of a buffer.
    fn rms_amplitude(buf: &[f32]) -> f64 {
        let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum_sq / buf.len() as f64).sqrt()
    }

    /// Enable a single band via set_param calls, then enable it.
    fn configure_band(
        eq: &mut ParametricEq,
        band: u32,
        filter_type: FilterType,
        freq: f64,
        gain_db: f64,
        q: f64,
    ) {
        let base = band * 4;
        eq.set_param(base + 0, freq);
        eq.set_param(base + 1, gain_db);
        eq.set_param(base + 2, q);
        eq.set_param(base + 3, filter_type.to_index() as f64);
        // Enable the band
        eq.bands[band as usize].enabled = true;
        eq.recompute_band(band as usize);
    }

    // -----------------------------------------------------------------------
    // test_bypass_is_bitexact
    // -----------------------------------------------------------------------

    #[test]
    fn test_bypass_is_bitexact() {
        let mut eq = ParametricEq::new(44100);
        eq.set_param(32, 1.0); // bypass on

        let input: Vec<f32> = (0..256).map(|i| (i as f32) * 0.001 - 0.128).collect();
        let silent = vec![0.0f32; 256];
        let mut out_l = vec![0.0f32; 256];
        let mut out_r = vec![0.0f32; 256];

        eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

        for i in 0..256 {
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
    // test_all_bands_disabled_is_passthrough
    // -----------------------------------------------------------------------

    #[test]
    fn test_all_bands_disabled_is_passthrough() {
        let mut eq = ParametricEq::new(44100);
        // All bands disabled by default, bypass off
        assert!(!eq.bypass);

        let input: Vec<f32> = (0..256).map(|i| (i as f32) * 0.002 - 0.256).collect();
        let mut out_l = vec![0.0f32; 256];
        let mut out_r = vec![0.0f32; 256];

        eq.process_effect(&input, &input, &mut out_l, &mut out_r);

        for i in 0..256 {
            assert_eq!(
                out_l[i].to_bits(),
                input[i].to_bits(),
                "all-disabled left sample {} not bit-exact",
                i
            );
            assert_eq!(
                out_r[i].to_bits(),
                input[i].to_bits(),
                "all-disabled right sample {} not bit-exact",
                i
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_peak_eq_boost
    // -----------------------------------------------------------------------

    #[test]
    fn test_peak_eq_boost() {
        let sr = 48000;
        let mut eq = ParametricEq::new(sr);
        configure_band(&mut eq, 0, FilterType::Peak, 1000.0, 6.0, 1.0);

        // Generate enough samples for the filter to reach steady state.
        let num_samples = sr as usize * 2; // 2 seconds
        let input = sine_wave(1000.0, sr, num_samples);
        let silent = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

        // Measure gain from the last second (steady state).
        let tail_start = sr as usize;
        let input_rms = rms_amplitude(&input[tail_start..]);
        let output_rms = rms_amplitude(&out_l[tail_start..]);
        let gain_db = 20.0 * (output_rms / input_rms).log10();

        let error = (gain_db - 6.0).abs();
        assert!(
            error < 1e-5,
            "expected +6.0 dB, got {:.8} dB (error {:.2e})",
            gain_db,
            error
        );
    }

    // -----------------------------------------------------------------------
    // test_peak_eq_cut
    // -----------------------------------------------------------------------

    #[test]
    fn test_peak_eq_cut() {
        let sr = 48000;
        let mut eq = ParametricEq::new(sr);
        configure_band(&mut eq, 0, FilterType::Peak, 1000.0, -6.0, 1.0);

        let num_samples = sr as usize * 2;
        let input = sine_wave(1000.0, sr, num_samples);
        let silent = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

        let tail_start = sr as usize;
        let input_rms = rms_amplitude(&input[tail_start..]);
        let output_rms = rms_amplitude(&out_l[tail_start..]);
        let gain_db = 20.0 * (output_rms / input_rms).log10();

        let error = (gain_db - (-6.0)).abs();
        assert!(
            error < 1e-5,
            "expected -6.0 dB, got {:.8} dB (error {:.2e})",
            gain_db,
            error
        );
    }

    // -----------------------------------------------------------------------
    // test_highpass_filter
    // -----------------------------------------------------------------------

    #[test]
    fn test_highpass_filter() {
        let sr = 48000;
        let mut eq = ParametricEq::new(sr);
        configure_band(&mut eq, 0, FilterType::Highpass, 1000.0, 0.0, 0.707);

        let num_samples = sr as usize * 2;

        // Test 1: 100 Hz should be attenuated > 40 dB
        {
            let input = sine_wave(100.0, sr, num_samples);
            let silent = vec![0.0f32; num_samples];
            let mut out_l = vec![0.0f32; num_samples];
            let mut out_r = vec![0.0f32; num_samples];

            eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

            let tail_start = sr as usize;
            let input_rms = rms_amplitude(&input[tail_start..]);
            let output_rms = rms_amplitude(&out_l[tail_start..]);
            let attenuation_db = 20.0 * (input_rms / output_rms).log10();

            assert!(
                attenuation_db > 40.0,
                "100 Hz through 1kHz HP should be attenuated >40 dB, got {:.1} dB",
                attenuation_db
            );
        }

        // Reset filter state for next test
        eq.filters_left[0].reset();
        eq.filters_right[0].reset();

        // Test 2: 10000 Hz should pass through (~0 dB)
        {
            let input = sine_wave(10000.0, sr, num_samples);
            let silent = vec![0.0f32; num_samples];
            let mut out_l = vec![0.0f32; num_samples];
            let mut out_r = vec![0.0f32; num_samples];

            eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

            let tail_start = sr as usize;
            let input_rms = rms_amplitude(&input[tail_start..]);
            let output_rms = rms_amplitude(&out_l[tail_start..]);
            let gain_db = 20.0 * (output_rms / input_rms).log10();

            assert!(
                gain_db.abs() < 0.5,
                "10 kHz through 1 kHz HP should be ~0 dB, got {:.2} dB",
                gain_db
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_lowpass_filter
    // -----------------------------------------------------------------------

    #[test]
    fn test_lowpass_filter() {
        let sr = 48000;
        let mut eq = ParametricEq::new(sr);
        configure_band(&mut eq, 0, FilterType::Lowpass, 1000.0, 0.0, 0.707);

        let num_samples = sr as usize * 2;

        // Test 1: 10000 Hz should be attenuated > 40 dB
        {
            let input = sine_wave(10000.0, sr, num_samples);
            let silent = vec![0.0f32; num_samples];
            let mut out_l = vec![0.0f32; num_samples];
            let mut out_r = vec![0.0f32; num_samples];

            eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

            let tail_start = sr as usize;
            let input_rms = rms_amplitude(&input[tail_start..]);
            let output_rms = rms_amplitude(&out_l[tail_start..]);
            let attenuation_db = 20.0 * (input_rms / output_rms).log10();

            assert!(
                attenuation_db > 40.0,
                "10 kHz through 1 kHz LP should be attenuated >40 dB, got {:.1} dB",
                attenuation_db
            );
        }

        eq.filters_left[0].reset();
        eq.filters_right[0].reset();

        // Test 2: 100 Hz should pass through (~0 dB)
        {
            let input = sine_wave(100.0, sr, num_samples);
            let silent = vec![0.0f32; num_samples];
            let mut out_l = vec![0.0f32; num_samples];
            let mut out_r = vec![0.0f32; num_samples];

            eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

            let tail_start = sr as usize;
            let input_rms = rms_amplitude(&input[tail_start..]);
            let output_rms = rms_amplitude(&out_l[tail_start..]);
            let gain_db = 20.0 * (output_rms / input_rms).log10();

            assert!(
                gain_db.abs() < 0.5,
                "100 Hz through 1 kHz LP should be ~0 dB, got {:.2} dB",
                gain_db
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_notch_filter
    // -----------------------------------------------------------------------

    #[test]
    fn test_notch_filter() {
        let sr = 48000;
        let mut eq = ParametricEq::new(sr);
        configure_band(&mut eq, 0, FilterType::Notch, 1000.0, 0.0, 10.0);

        let num_samples = sr as usize * 2;

        // Test 1: 1000 Hz should be deeply attenuated (>40 dB)
        {
            let input = sine_wave(1000.0, sr, num_samples);
            let silent = vec![0.0f32; num_samples];
            let mut out_l = vec![0.0f32; num_samples];
            let mut out_r = vec![0.0f32; num_samples];

            eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

            let tail_start = sr as usize;
            let input_rms = rms_amplitude(&input[tail_start..]);
            let output_rms = rms_amplitude(&out_l[tail_start..]);
            let attenuation_db = 20.0 * (input_rms / output_rms).log10();

            assert!(
                attenuation_db > 40.0,
                "1 kHz through notch at 1 kHz should be attenuated >40 dB, got {:.1} dB",
                attenuation_db
            );
        }

        eq.filters_left[0].reset();
        eq.filters_right[0].reset();

        // Test 2: 500 Hz should pass through
        {
            let input = sine_wave(500.0, sr, num_samples);
            let silent = vec![0.0f32; num_samples];
            let mut out_l = vec![0.0f32; num_samples];
            let mut out_r = vec![0.0f32; num_samples];

            eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

            let tail_start = sr as usize;
            let input_rms = rms_amplitude(&input[tail_start..]);
            let output_rms = rms_amplitude(&out_l[tail_start..]);
            let gain_db = 20.0 * (output_rms / input_rms).log10();

            assert!(
                gain_db.abs() < 0.5,
                "500 Hz through notch at 1 kHz should be ~0 dB, got {:.2} dB",
                gain_db
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_cascaded_bands
    // -----------------------------------------------------------------------

    #[test]
    fn test_cascaded_bands() {
        let sr = 48000;
        let mut eq = ParametricEq::new(sr);

        // Band 0: Peak +3 dB at 1000 Hz
        configure_band(&mut eq, 0, FilterType::Peak, 1000.0, 3.0, 1.0);
        // Band 1: Peak +3 dB at 1000 Hz
        configure_band(&mut eq, 1, FilterType::Peak, 1000.0, 3.0, 1.0);

        let num_samples = sr as usize * 2;
        let input = sine_wave(1000.0, sr, num_samples);
        let silent = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

        let tail_start = sr as usize;
        let input_rms = rms_amplitude(&input[tail_start..]);
        let output_rms = rms_amplitude(&out_l[tail_start..]);
        let gain_db = 20.0 * (output_rms / input_rms).log10();

        let error = (gain_db - 6.0).abs();
        assert!(
            error < 1e-5,
            "two cascaded +3 dB bands should give +6 dB, got {:.8} dB (error {:.2e})",
            gain_db,
            error
        );
    }

    // -----------------------------------------------------------------------
    // Param round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_round_trip() {
        let mut eq = ParametricEq::new(44100);

        // Set band 2 frequency
        eq.set_param(8, 2500.0); // band 2, param 0 = freq
        assert_eq!(eq.get_param(8), Some(2500.0));

        // Set bypass
        eq.set_param(32, 1.0);
        assert_eq!(eq.get_param(32), Some(1.0));
        eq.set_param(32, 0.0);
        assert_eq!(eq.get_param(32), Some(0.0));

        // Param count
        assert_eq!(eq.param_count(), 33);

        // Invalid param
        assert_eq!(eq.get_param(99), None);
        assert!(eq.param_info(33).is_none());
    }

    // -----------------------------------------------------------------------
    // Info
    // -----------------------------------------------------------------------

    #[test]
    fn test_info() {
        let eq = ParametricEq::new(44100);
        let info = eq.info();
        assert_eq!(info.name, "Parametric EQ");
        assert_eq!(info.backend_type, BackendType::PluginHost);
        assert!(info.extensions.is_empty());
    }

    #[test]
    fn test_latency_is_zero() {
        let eq = ParametricEq::new(44100);
        assert_eq!(eq.latency(), 0);
    }
}
