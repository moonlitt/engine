//! AES17-2020 Signal Quality Compliance Tests
//!
//! References:
//! - AES17-2020: https://www.aes.org/publications/standards/search.cfm?docID=21
//! - AES Standard for Measurement of Digital Audio Equipment
//!
//! Zero tolerance: all assertions use machine epsilon.

use moonlitt_compressor::Compressor;
use moonlitt_core::AudioBackend;
use moonlitt_eq::{Band, BiquadCoeffs, FilterType, ParametricEq};
use moonlitt_reverb::Reverb;
use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

const SAMPLE_RATE: u32 = 48000;
const BLOCK_SIZE: usize = 4096;

// =============================================================================
// Helpers
// =============================================================================

/// Generate a mono sine wave at the given frequency (f64 precision, cast to f32).
fn sine_wave(freq: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / sample_rate as f64;
            (2.0 * PI * freq * t).sin() as f32
        })
        .collect()
}

/// Generate a mono sine wave at a given amplitude in dBFS.
fn sine_wave_dbfs(freq: f64, dbfs: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
    let amplitude = 10.0_f64.powf(dbfs / 20.0);
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / sample_rate as f64;
            (amplitude * (2.0 * PI * freq * t).sin()) as f32
        })
        .collect()
}

/// Measure RMS amplitude of a buffer (f64 precision).
fn rms(buf: &[f32]) -> f64 {
    let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum_sq / buf.len() as f64).sqrt()
}

/// Compute power spectrum using FFT. Returns magnitude^2 per bin (first half only).
fn power_spectrum(signal: &[f32]) -> Vec<f64> {
    let n = signal.len();
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);

    // Apply Hann window
    let mut buffer: Vec<Complex<f64>> = signal
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let w = 0.5 * (1.0 - (2.0 * PI * i as f64 / n as f64).cos());
            Complex::new(s as f64 * w, 0.0)
        })
        .collect();

    fft.process(&mut buffer);

    buffer[..n / 2]
        .iter()
        .map(|c| c.norm_sqr())
        .collect()
}

/// Compute the biquad transfer function magnitude at a given frequency.
///
/// H(e^{jw}) = (b0 + b1*e^{-jw} + b2*e^{-2jw}) / (1 + a1*e^{-jw} + a2*e^{-2jw})
///
/// Returns |H(e^{jw})| (linear magnitude).
fn biquad_magnitude_at(coeffs: &BiquadCoeffs, freq: f64, sample_rate: f64) -> f64 {
    let w = 2.0 * PI * freq / sample_rate;

    let ejw = Complex::new(0.0, -w).exp();
    let e2jw = Complex::new(0.0, -2.0 * w).exp();

    let num = Complex::new(coeffs.b0, 0.0) + Complex::new(coeffs.b1, 0.0) * ejw + Complex::new(coeffs.b2, 0.0) * e2jw;
    let den = Complex::new(1.0, 0.0) + Complex::new(coeffs.a1, 0.0) * ejw + Complex::new(coeffs.a2, 0.0) * e2jw;

    (num / den).norm()
}

/// Create a ParametricEq with bypass=true.
fn eq_bypassed() -> ParametricEq {
    let mut eq = ParametricEq::new(SAMPLE_RATE);
    eq.set_param(32, 1.0); // bypass on
    eq
}

/// Create a ParametricEq with all 8 bands enabled at 0dB gain (flat).
fn eq_flat_active() -> ParametricEq {
    let mut eq = ParametricEq::new(SAMPLE_RATE);
    for i in 0..8 {
        eq.set_band(
            i,
            Band {
                filter_type: FilterType::Peak,
                frequency: [60.0, 170.0, 400.0, 1000.0, 2500.0, 6000.0, 12000.0, 16000.0][i],
                gain_db: 0.0,
                q: 1.0,
                enabled: true,
            },
        );
    }
    eq
}

// =============================================================================
// A1: eq_bypass_thd — bypass must be bit-exact
// =============================================================================

#[test]
fn a1_eq_bypass_thd() {
    let mut eq = eq_bypassed();

    let input = sine_wave(1000.0, SAMPLE_RATE, BLOCK_SIZE);
    let silent = vec![0.0f32; BLOCK_SIZE];
    let mut out_l = vec![0.0f32; BLOCK_SIZE];
    let mut out_r = vec![0.0f32; BLOCK_SIZE];

    eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            input[i].to_bits(),
            "A1: bypass L[{i}] not bit-exact: got {}, expected {}",
            out_l[i],
            input[i]
        );
        assert_eq!(
            out_r[i].to_bits(),
            silent[i].to_bits(),
            "A1: bypass R[{i}] not bit-exact"
        );
    }
}

// =============================================================================
// A2: eq_active_thd — all bands at 0dB gain, THD must be < f32::EPSILON
// =============================================================================

#[test]
fn a2_eq_active_thd() {
    let mut eq = eq_flat_active();

    // Use enough samples for filter to reach steady state
    let num_samples = SAMPLE_RATE as usize * 2;
    let input = sine_wave(1000.0, SAMPLE_RATE, num_samples);
    let silent = vec![0.0f32; num_samples];
    let mut out_l = vec![0.0f32; num_samples];
    let mut out_r = vec![0.0f32; num_samples];

    eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

    // Measure THD from the last second (steady state)
    let tail_start = SAMPLE_RATE as usize;
    let tail = &out_l[tail_start..];

    let spectrum = power_spectrum(tail);
    let n = tail.len();

    // Find the fundamental bin (1kHz)
    let fundamental_bin = (1000.0 * n as f64 / SAMPLE_RATE as f64).round() as usize;

    // Fundamental energy (bin +/- 2 to capture spectral leakage from windowing)
    let fundamental_energy: f64 = spectrum
        [fundamental_bin.saturating_sub(2)..=(fundamental_bin + 2).min(spectrum.len() - 1)]
        .iter()
        .sum();

    // Harmonic energy: 2nd through 10th harmonics
    let mut harmonic_energy = 0.0_f64;
    for h in 2..=10 {
        let bin = fundamental_bin * h;
        if bin + 2 >= spectrum.len() {
            break;
        }
        let energy: f64 = spectrum[bin.saturating_sub(2)..=(bin + 2).min(spectrum.len() - 1)]
            .iter()
            .sum();
        harmonic_energy += energy;
    }

    let thd = if fundamental_energy > 0.0 {
        (harmonic_energy / fundamental_energy).sqrt()
    } else {
        0.0
    };

    assert!(
        thd < f32::EPSILON as f64,
        "A2: THD = {thd:.2e}, must be < f32::EPSILON ({:.2e})",
        f32::EPSILON
    );
}

// =============================================================================
// A3: compressor_bypass_thd — bypass must be bit-exact
// =============================================================================

#[test]
fn a3_compressor_bypass_thd() {
    let mut comp = Compressor::new(SAMPLE_RATE);
    comp.set_param(8, 1.0); // bypass on

    let input = sine_wave(1000.0, SAMPLE_RATE, BLOCK_SIZE);
    let silent = vec![0.0f32; BLOCK_SIZE];
    let mut out_l = vec![0.0f32; BLOCK_SIZE];
    let mut out_r = vec![0.0f32; BLOCK_SIZE];

    comp.process_effect(&input, &silent, &mut out_l, &mut out_r);

    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            input[i].to_bits(),
            "A3: compressor bypass L[{i}] not bit-exact"
        );
        assert_eq!(
            out_r[i].to_bits(),
            silent[i].to_bits(),
            "A3: compressor bypass R[{i}] not bit-exact"
        );
    }
}

// =============================================================================
// A4: compressor_below_threshold_thd — below threshold must be bit-exact
// =============================================================================

#[test]
fn a4_compressor_below_threshold_thd() {
    let mut comp = Compressor::new(SAMPLE_RATE);
    // Threshold = 0 dB, makeup = 0 dB (defaults), knee = 0 (hard)
    comp.set_param(0, 0.0); // threshold = 0dB
    comp.set_param(4, 0.0); // knee = 0 (hard knee)
    comp.set_param(5, 0.0); // makeup = 0dB

    // Input at -30 dBFS — well below 0dB threshold
    let input = sine_wave_dbfs(1000.0, -30.0, SAMPLE_RATE, BLOCK_SIZE);
    let silent = vec![0.0f32; BLOCK_SIZE];
    let mut out_l = vec![0.0f32; BLOCK_SIZE];
    let mut out_r = vec![0.0f32; BLOCK_SIZE];

    comp.process_effect(&input, &silent, &mut out_l, &mut out_r);

    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            input[i].to_bits(),
            "A4: compressor below-threshold L[{i}] not bit-exact: got {:.10e}, expected {:.10e}",
            out_l[i],
            input[i]
        );
    }
}

// =============================================================================
// A5: reverb_dry_thd — dry_wet=0.0 must be bit-exact passthrough
// =============================================================================

#[test]
fn a5_reverb_dry_thd() {
    let mut rev = Reverb::new(SAMPLE_RATE);
    rev.set_param(7, 0.0); // dry_wet = 0.0 (fully dry)

    let input = sine_wave(1000.0, SAMPLE_RATE, BLOCK_SIZE);
    let silent = vec![0.0f32; BLOCK_SIZE];
    let mut out_l = vec![0.0f32; BLOCK_SIZE];
    let mut out_r = vec![0.0f32; BLOCK_SIZE];

    rev.process_effect(&input, &silent, &mut out_l, &mut out_r);

    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            input[i].to_bits(),
            "A5: reverb dry L[{i}] not bit-exact"
        );
        assert_eq!(
            out_r[i].to_bits(),
            silent[i].to_bits(),
            "A5: reverb dry R[{i}] not bit-exact"
        );
    }
}

// =============================================================================
// A6: frequency_response_flat — EQ bypass, FFT of output must equal FFT of input
// =============================================================================

#[test]
fn a6_frequency_response_flat() {
    let mut eq = eq_bypassed();

    // Use white-noise-like signal (deterministic pseudo-random via linear ramp)
    let input: Vec<f32> = (0..BLOCK_SIZE)
        .map(|i| ((i as f64 * 0.7123 + 0.31).sin() * 0.8) as f32)
        .collect();
    let silent = vec![0.0f32; BLOCK_SIZE];
    let mut out_l = vec![0.0f32; BLOCK_SIZE];
    let mut out_r = vec![0.0f32; BLOCK_SIZE];

    eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

    // Bypass = bit-exact copy, so FFT of output == FFT of input trivially
    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            input[i].to_bits(),
            "A6: bypass output[{i}] not bit-exact"
        );
    }

    // Confirm via FFT as well
    let spec_in = power_spectrum(&input);
    let spec_out = power_spectrum(&out_l);

    for (bin, (si, so)) in spec_in.iter().zip(spec_out.iter()).enumerate() {
        assert_eq!(
            si.to_bits(),
            so.to_bits(),
            "A6: spectrum bin {bin} mismatch: input={si:.6e}, output={so:.6e}"
        );
    }
}

// =============================================================================
// A7: silence_noise_floor — all-zero input through EQ must produce all-zero output
// =============================================================================

#[test]
fn a7_silence_noise_floor() {
    let mut eq = eq_flat_active();

    let zeros = vec![0.0f32; BLOCK_SIZE];
    let mut out_l = vec![0.0f32; BLOCK_SIZE];
    let mut out_r = vec![0.0f32; BLOCK_SIZE];

    eq.process_effect(&zeros, &zeros, &mut out_l, &mut out_r);

    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            0.0f32.to_bits(),
            "A7: noise floor L[{i}] = {:.2e}, expected exactly 0.0",
            out_l[i]
        );
        assert_eq!(
            out_r[i].to_bits(),
            0.0f32.to_bits(),
            "A7: noise floor R[{i}] = {:.2e}, expected exactly 0.0",
            out_r[i]
        );
    }
}

// =============================================================================
// A8: channel_crosstalk — sine on L only, R must remain zero
// =============================================================================

#[test]
fn a8_channel_crosstalk() {
    let mut eq = eq_flat_active();

    // Process enough for steady state, then check a final block
    let warmup = SAMPLE_RATE as usize; // 1 second warmup
    let input = sine_wave(1000.0, SAMPLE_RATE, warmup + BLOCK_SIZE);
    let silent = vec![0.0f32; warmup + BLOCK_SIZE];
    let mut out_l = vec![0.0f32; warmup + BLOCK_SIZE];
    let mut out_r = vec![0.0f32; warmup + BLOCK_SIZE];

    eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

    // R channel must be bit-exact zero (no crosstalk from L)
    for i in warmup..(warmup + BLOCK_SIZE) {
        assert_eq!(
            out_r[i].to_bits(),
            0.0f32.to_bits(),
            "A8: crosstalk R[{i}] = {:.2e}, expected 0.0",
            out_r[i]
        );
    }
}

// =============================================================================
// A9: dynamic_range — full-scale and -120dBFS both preserved through bypass
// =============================================================================

#[test]
fn a9_dynamic_range() {
    let mut eq = eq_bypassed();

    // Full-scale sine (0 dBFS)
    let loud = sine_wave(1000.0, SAMPLE_RATE, BLOCK_SIZE);
    // Very quiet sine (-120 dBFS)
    let quiet = sine_wave_dbfs(1000.0, -120.0, SAMPLE_RATE, BLOCK_SIZE);

    let silent = vec![0.0f32; BLOCK_SIZE];
    let mut out_l = vec![0.0f32; BLOCK_SIZE];
    let mut out_r = vec![0.0f32; BLOCK_SIZE];

    // Process loud signal
    eq.process_effect(&loud, &silent, &mut out_l, &mut out_r);
    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            loud[i].to_bits(),
            "A9: full-scale L[{i}] not bit-exact"
        );
    }

    // Process quiet signal
    eq.process_effect(&quiet, &silent, &mut out_l, &mut out_r);
    for i in 0..BLOCK_SIZE {
        assert_eq!(
            out_l[i].to_bits(),
            quiet[i].to_bits(),
            "A9: -120dBFS L[{i}] not bit-exact"
        );
    }

    // Verify the quiet signal is actually non-zero (not lost to noise)
    let quiet_rms = rms(&quiet);
    assert!(
        quiet_rms > 0.0,
        "A9: -120dBFS signal has zero RMS, signal lost"
    );
}

// =============================================================================
// A10: eq_frequency_sweep — Peak +6dB at 1kHz, verify output matches
//      reference biquad sample-by-sample
// =============================================================================

#[test]
fn a10_eq_frequency_sweep() {
    use moonlitt_eq::Biquad;

    let sr = SAMPLE_RATE;
    let center_freq = 1000.0;
    let gain_db = 6.0;
    let q = 1.0;

    // Compute reference coefficients from the cookbook
    let coeffs = BiquadCoeffs::design(FilterType::Peak, sr as f64, center_freq, gain_db, q);

    // 20 log-spaced frequencies from 100Hz to 16kHz
    let freqs: Vec<f64> = (0..20)
        .map(|i| {
            let t = i as f64 / 19.0;
            100.0 * (16000.0 / 100.0_f64).powf(t)
        })
        .collect();

    let num_samples = BLOCK_SIZE;

    for &freq in &freqs {
        // Skip frequencies too close to Nyquist
        if freq >= sr as f64 / 2.0 {
            continue;
        }

        let input = sine_wave(freq, sr, num_samples);
        let silent = vec![0.0f32; num_samples];
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        // Process through the ParametricEq
        let mut eq = ParametricEq::new(sr);
        eq.set_band(
            0,
            Band {
                filter_type: FilterType::Peak,
                frequency: center_freq,
                gain_db,
                q,
                enabled: true,
            },
        );
        eq.process_effect(&input, &silent, &mut out_l, &mut out_r);

        // Compute reference output using the same biquad coefficients
        let mut ref_biquad = Biquad::new();
        ref_biquad.set_coeffs(coeffs);

        for i in 0..num_samples {
            let ref_out = ref_biquad.process(input[i] as f64) as f32;
            assert_eq!(
                out_l[i].to_bits(),
                ref_out.to_bits(),
                "A10: freq={freq:.0}Hz sample[{i}] mismatch: EQ output={:.10e}, \
                 reference biquad={:.10e}",
                out_l[i],
                ref_out
            );
        }
    }
}
