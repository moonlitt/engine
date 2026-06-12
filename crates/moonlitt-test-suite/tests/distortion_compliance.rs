//! Distortion Compliance Tests — Saturator + Bitcrusher
//!
//! Validates harmonic character, aliasing rejection, DC blocking,
//! quantization noise floor, and rate-reduction imaging for the
//! `moonlitt-effects` distortion module.
//!
//! Self-contained file with inline helpers.

use moonlitt_core::AudioBackend;
use moonlitt_effects::{Bitcrusher, Saturator};
use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

const SR: u32 = 44100;

// =============================================================================
// Helpers
// =============================================================================

/// Generate a mono sine wave at the given frequency and amplitude (linear).
fn sine_f32(freq: f64, amplitude: f64, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / SR as f64;
            (amplitude * (2.0 * PI * freq * t).sin()) as f32
        })
        .collect()
}

/// RMS of a buffer (f64 precision).
#[allow(dead_code)]
fn rms(buf: &[f32]) -> f64 {
    let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum_sq / buf.len() as f64).sqrt()
}

/// RMS in dBFS.
#[allow(dead_code)]
fn rms_dbfs(buf: &[f32]) -> f64 {
    let r = rms(buf);
    if r < 1e-30 {
        -300.0
    } else {
        20.0 * r.log10()
    }
}

/// Compute power spectrum using FFT. Returns magnitude^2 per bin (first half).
/// Applies a Hann window to reduce spectral leakage.
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

    buffer[..n / 2].iter().map(|c| c.norm_sqr()).collect()
}

/// Convert FFT bin index to frequency.
#[allow(dead_code)]
fn bin_to_freq(bin: usize, fft_len: usize) -> f64 {
    bin as f64 * SR as f64 / fft_len as f64
}

/// Convert frequency to nearest FFT bin index.
fn freq_to_bin(freq: f64, fft_len: usize) -> usize {
    (freq * fft_len as f64 / SR as f64).round() as usize
}

/// Magnitude in dB for a power-spectrum bin (10*log10 since already squared).
fn bin_db(spectrum: &[f64], bin: usize) -> f64 {
    let val = spectrum[bin].max(1e-30);
    10.0 * val.log10()
}

/// Sum power in a frequency band [lo_hz, hi_hz] and return dB.
fn band_power_db(spectrum: &[f64], fft_len: usize, lo_hz: f64, hi_hz: f64) -> f64 {
    let lo_bin = freq_to_bin(lo_hz, fft_len);
    let hi_bin = freq_to_bin(hi_hz, fft_len).min(spectrum.len() - 1);
    let sum: f64 = spectrum[lo_bin..=hi_bin].iter().sum();
    10.0 * sum.max(1e-30).log10()
}

/// Helper: configure Saturator with common defaults, then apply overrides.
/// Returns a Saturator with smoothers settled.
fn make_saturator(overrides: &[(u32, f64)]) -> Saturator {
    let mut sat = Saturator::new(SR);
    // Set sensible test defaults: tone=0.5, output=0dB, mix=1.0, high_cut=20kHz
    sat.set_param(2, 0.5); // tone
    sat.set_param(3, 0.0); // output_db
    sat.set_param(6, 1.0); // mix
    sat.set_param(7, 20000.0); // high_cut
    sat.set_param(8, 0.0); // bypass off

    for &(id, val) in overrides {
        sat.set_param(id, val);
    }

    // Settle smoothers by processing silence
    let silence = vec![0.0f32; 4096];
    let mut dummy_l = vec![0.0f32; 4096];
    let mut dummy_r = vec![0.0f32; 4096];
    for _ in 0..10 {
        sat.process_effect(&silence, &silence, &mut dummy_l, &mut dummy_r);
    }

    sat
}

/// Helper: configure Bitcrusher with overrides and settled smoothers.
fn make_bitcrusher(overrides: &[(u32, f64)]) -> Bitcrusher {
    let mut bc = Bitcrusher::new(SR);
    for &(id, val) in overrides {
        bc.set_param(id, val);
    }

    // Settle smoothers
    let silence = vec![0.0f32; 4096];
    let mut dummy_l = vec![0.0f32; 4096];
    let mut dummy_r = vec![0.0f32; 4096];
    for _ in 0..10 {
        bc.process_effect(&silence, &silence, &mut dummy_l, &mut dummy_r);
    }

    bc
}

/// Process a mono signal through an effect (left channel only) in blocks.
/// Respects MAX_BLOCK_SIZE = 4096 used by the Saturator's internal buffers.
fn process_mono(effect: &mut dyn AudioBackend, input: &[f32]) -> Vec<f32> {
    const BLOCK: usize = 4096;
    let len = input.len();
    let mut out_l = vec![0.0f32; len];
    let silent = vec![0.0f32; BLOCK];
    let mut tmp_r = vec![0.0f32; BLOCK];

    let mut pos = 0;
    while pos < len {
        let end = (pos + BLOCK).min(len);
        let block_len = end - pos;
        effect.process_effect(
            &input[pos..end],
            &silent[..block_len],
            &mut out_l[pos..end],
            &mut tmp_r[..block_len],
        );
        pos = end;
    }
    out_l
}

// =============================================================================
// S1: Tube — asymmetry raises even harmonic ratio
// =============================================================================
//
// Tube mode (x/(1+|x|)) is an odd function, so with zero asymmetry it
// produces only odd harmonics. The Saturator's asymmetry parameter adds a
// bias (x += asym * |x|) that breaks symmetry, raising even-order harmonics.
//
// We compare H2/H3 ratio with asymmetry=0.8 versus asymmetry=0. The ratio
// should increase by at least 6 dB, proving the asymmetry mechanism works.

#[test]
fn s1_tube_even_harmonics() {
    let num_samples = SR as usize;
    let input = sine_f32(1000.0, 0.5, num_samples);
    let fft_len = num_samples;

    // Symmetric (no asymmetry) — baseline
    let mut sat_sym = make_saturator(&[
        (0, 12.0), // drive = 12 dB
        (1, 0.0),  // mode = Tube
        (4, 1.0),  // oversampling = 2x
        (5, 0.0),  // asymmetry = 0 (symmetric)
    ]);
    let out_sym = process_mono(&mut sat_sym, &input);
    let spec_sym = power_spectrum(&out_sym);

    let h2_sym = bin_db(&spec_sym, freq_to_bin(2000.0, fft_len));
    let h3_sym = bin_db(&spec_sym, freq_to_bin(3000.0, fft_len));
    let ratio_sym = h2_sym - h3_sym;

    // Asymmetric — should boost even harmonics
    let mut sat_asym = make_saturator(&[
        (0, 12.0), // drive = 12 dB
        (1, 0.0),  // mode = Tube
        (4, 1.0),  // oversampling = 2x
        (5, 0.8),  // asymmetry = 0.8
    ]);
    let out_asym = process_mono(&mut sat_asym, &input);
    let spec_asym = power_spectrum(&out_asym);

    let h2_asym = bin_db(&spec_asym, freq_to_bin(2000.0, fft_len));
    let h3_asym = bin_db(&spec_asym, freq_to_bin(3000.0, fft_len));
    let ratio_asym = h2_asym - h3_asym;

    let improvement = ratio_asym - ratio_sym;

    eprintln!(
        "S1 Tube: sym H2/H3 = {:.1} dB, asym H2/H3 = {:.1} dB, improvement = {:.1} dB",
        ratio_sym, ratio_asym, improvement
    );
    eprintln!("  symmetric:  H2 = {:.1} dB, H3 = {:.1} dB", h2_sym, h3_sym);
    eprintln!(
        "  asymmetric: H2 = {:.1} dB, H3 = {:.1} dB",
        h2_asym, h3_asym
    );

    // Asymmetry should raise the H2/H3 ratio by at least 6 dB.
    assert!(
        improvement > 6.0,
        "S1: asymmetry should raise H2/H3 ratio by > 6 dB, got {:.1} dB",
        improvement
    );
}

// =============================================================================
// S2: Transistor — odd harmonic character
// =============================================================================
//
// Transistor mode uses tanh(x), a symmetric (odd) function. With zero
// asymmetry, only odd harmonics are generated. The 3rd harmonic (3kHz)
// should exceed the 2nd harmonic (2kHz) by at least 3dB.

#[test]
fn s2_transistor_odd_harmonics() {
    let mut sat = make_saturator(&[
        (0, 24.0), // drive = 24 dB
        (1, 2.0),  // mode = Transistor
        (4, 1.0),  // oversampling = 2x
        (5, 0.0),  // asymmetry = 0 (symmetric clipping)
    ]);

    let num_samples = SR as usize;
    let input = sine_f32(1000.0, 0.5, num_samples);
    let output = process_mono(&mut sat, &input);

    let spectrum = power_spectrum(&output);
    let fft_len = num_samples;

    let h2_bin = freq_to_bin(2000.0, fft_len);
    let h3_bin = freq_to_bin(3000.0, fft_len);

    let h2_db = bin_db(&spectrum, h2_bin);
    let h3_db = bin_db(&spectrum, h3_bin);

    eprintln!(
        "S2 Transistor: H2 = {:.1} dB, H3 = {:.1} dB, diff = {:.1} dB",
        h2_db,
        h3_db,
        h3_db - h2_db
    );

    assert!(
        h3_db - h2_db >= 3.0,
        "S2: 3rd harmonic ({:.1} dB) should exceed 2nd ({:.1} dB) by >= 3 dB, got {:.1} dB",
        h3_db,
        h2_db,
        h3_db - h2_db
    );
}

// =============================================================================
// S3: Oversampling alias rejection
// =============================================================================
//
// Heavy distortion (36dB drive) on an 8kHz sine produces harmonic content
// above Nyquist. Without oversampling, these alias back into the audible
// band. With 2x oversampling, the anti-alias filter should attenuate this
// energy significantly.
//
// Measure energy in the 15-20kHz band. 2x should have > 10dB less aliasing
// energy than 1x.

#[test]
fn s3_oversampling_alias_rejection() {
    let num_samples = SR as usize;
    let input = sine_f32(8000.0, 0.5, num_samples);

    // 1x oversampling (no anti-aliasing)
    let mut sat_1x = make_saturator(&[
        (0, 36.0), // drive = 36 dB
        (1, 2.0),  // mode = Transistor
        (4, 0.0),  // oversampling = 1x
        (5, 0.0),  // asymmetry = 0
    ]);
    let out_1x = process_mono(&mut sat_1x, &input);
    let spec_1x = power_spectrum(&out_1x);

    // 2x oversampling
    let mut sat_2x = make_saturator(&[
        (0, 36.0),
        (1, 2.0),
        (4, 1.0), // oversampling = 2x
        (5, 0.0),
    ]);
    let out_2x = process_mono(&mut sat_2x, &input);
    let spec_2x = power_spectrum(&out_2x);

    let fft_len = num_samples;
    let alias_1x = band_power_db(&spec_1x, fft_len, 15000.0, 20000.0);
    let alias_2x = band_power_db(&spec_2x, fft_len, 15000.0, 20000.0);

    let reduction = alias_1x - alias_2x;

    eprintln!(
        "S3 Aliasing: 1x = {:.1} dB, 2x = {:.1} dB, reduction = {:.1} dB",
        alias_1x, alias_2x, reduction
    );

    assert!(
        reduction > 10.0,
        "S3: 2x oversampling should reduce 15-20kHz alias energy by > 10 dB, got {:.1} dB",
        reduction
    );
}

// =============================================================================
// S4: Asymmetry DC blocking
// =============================================================================
//
// Asymmetric waveshaping introduces a DC offset in the output. The Saturator's
// DC blocker (1-pole HPF at 5 Hz) should remove this. After processing 1 second
// of 1kHz sine with asymmetry=0.8, the mean of the output should be < 0.01.

#[test]
fn s4_asymmetry_dc_blocked() {
    let mut sat = make_saturator(&[
        (0, 24.0), // drive = 24 dB
        (1, 2.0),  // mode = Transistor
        (4, 1.0),  // oversampling = 2x
        (5, 0.8),  // asymmetry = 0.8 (strong asymmetry)
    ]);

    let num_samples = SR as usize;
    let input = sine_f32(1000.0, 0.5, num_samples);
    let output = process_mono(&mut sat, &input);

    // DC component = mean of all samples
    let dc: f64 = output.iter().map(|&s| s as f64).sum::<f64>() / output.len() as f64;

    eprintln!("S4 DC offset: {:.6} (threshold: 0.01)", dc.abs());

    assert!(
        dc.abs() < 0.01,
        "S4: DC component should be < 0.01 after blocking, got {:.6}",
        dc.abs()
    );
}

// =============================================================================
// S5: Drive=0 low THD — signal stays in linear region
// =============================================================================
//
// With drive=0dB and a low-amplitude 1kHz sine (0.1), the signal stays in
// the linear region of tanh(x) ≈ x. THD should be low: harmonic energy
// (H2..H5) should be at least 40 dB below the fundamental.
//
// We measure THD via FFT rather than RMS comparison, since the tone filter
// and DC blocker alter the overall gain while preserving spectral purity.

#[test]
fn s5_drive_zero_thd() {
    let mut sat = make_saturator(&[
        (0, 0.0), // drive = 0 dB
        (1, 2.0), // mode = Transistor
        (3, 0.0), // output = 0 dB
        (4, 1.0), // oversampling = 2x
        (5, 0.0), // asymmetry = 0
        (6, 1.0), // mix = 1.0
    ]);

    let num_samples = SR as usize;
    let input = sine_f32(1000.0, 0.1, num_samples);
    let output = process_mono(&mut sat, &input);

    let spectrum = power_spectrum(&output);
    let fft_len = num_samples;

    let fund_power = spectrum[freq_to_bin(1000.0, fft_len)];

    // Sum harmonic power (H2 through H5)
    let harmonic_power: f64 = (2..=5)
        .map(|h| {
            let bin = freq_to_bin(h as f64 * 1000.0, fft_len);
            if bin < spectrum.len() {
                spectrum[bin]
            } else {
                0.0
            }
        })
        .sum();

    // THD = sqrt(harmonic_power / fundamental_power), in dB
    let thd_db = if harmonic_power > 0.0 && fund_power > 0.0 {
        10.0 * (harmonic_power / fund_power).log10()
    } else {
        -100.0
    };

    eprintln!(
        "S5 Low-drive THD: {:.1} dB (harmonic energy relative to fundamental)",
        thd_db
    );

    // At 0dB drive with amplitude 0.1, tanh(0.1) ≈ 0.0997.
    // The distortion is ~0.03%, so THD should be well below -40 dB.
    assert!(
        thd_db < -40.0,
        "S5: THD should be < -40 dB at 0dB drive with low-level input, got {:.1} dB",
        thd_db
    );
}

// =============================================================================
// S6: Bitcrusher — quantization noise floor
// =============================================================================
//
// At 8-bit depth with no rate reduction, no dither, and 100% wet, feeding a
// 1kHz sine at -6 dBFS (amplitude ~0.5):
//
//   Ideal SQNR for full-scale sine: 6.02 * 8 + 1.76 = 49.9 dB
//   At -6 dBFS the effective SQNR drops by ~6 dB → ~44 dB
//
// We accept SNR in the range 38-55 dB to account for signal-dependent
// quantization noise and the rounding behavior of the implementation.

#[test]
fn s6_bit_depth_quantization_noise() {
    let mut bc = make_bitcrusher(&[
        (0, 8.0), // bit_depth = 8
        (1, 1.0), // rate_reduction = 1 (none)
        (2, 0.0), // dither = 0
        (3, 1.0), // dry_wet = 1.0
        (4, 0.0), // jitter = 0
        (5, 0.0), // bypass off
    ]);

    let num_samples = SR as usize;
    let amplitude = 10.0_f64.powf(-6.0 / 20.0); // -6 dBFS (~0.5)
    let input = sine_f32(1000.0, amplitude, num_samples);
    let output = process_mono(&mut bc, &input);

    // Compute quantization noise = output - input
    let noise: Vec<f32> = output
        .iter()
        .zip(input.iter())
        .map(|(&o, &i)| o - i)
        .collect();

    let signal_power: f64 = input.iter().map(|&s| (s as f64).powi(2)).sum::<f64>();
    let noise_power: f64 = noise.iter().map(|&s| (s as f64).powi(2)).sum::<f64>();

    let snr_db = 10.0 * (signal_power / noise_power).log10();

    eprintln!(
        "S6 Quantization SNR: {:.1} dB (expected 38-55 dB for 8-bit @ -6 dBFS)",
        snr_db
    );

    assert!(
        snr_db > 38.0 && snr_db < 55.0,
        "S6: SNR should be 38-55 dB for 8-bit quantization, got {:.1} dB",
        snr_db
    );
}

// =============================================================================
// S7: Bitcrusher — rate reduction imaging
// =============================================================================
//
// With rate_reduction=4, the effective sample rate is SR/4 = 11025 Hz.
// A 1kHz sine will produce imaging (spectral copies) around multiples of
// the reduced sample rate. At ~11025 Hz, there should be visible imaging
// components.
//
// Verify that energy around 11025 +/- 1000 Hz is > -40 dB relative to
// the fundamental (1kHz).

#[test]
fn s7_rate_reduction_imaging() {
    let mut bc = make_bitcrusher(&[
        (0, 24.0), // bit_depth = 24 (minimize quantization artifacts)
        (1, 4.0),  // rate_reduction = 4
        (2, 0.0),  // dither = 0
        (3, 1.0),  // dry_wet = 1.0
        (4, 0.0),  // jitter = 0
        (5, 0.0),  // bypass off
    ]);

    let num_samples = SR as usize;
    let input = sine_f32(1000.0, 0.5, num_samples);
    let output = process_mono(&mut bc, &input);

    let spectrum = power_spectrum(&output);
    let fft_len = num_samples;

    // Fundamental magnitude
    let fund_bin = freq_to_bin(1000.0, fft_len);
    let fund_db = bin_db(&spectrum, fund_bin);

    // Imaging energy around sr/rate_reduction = 11025 Hz, +/- 1000 Hz
    let image_db = band_power_db(&spectrum, fft_len, 10025.0, 12025.0);

    let relative_db = image_db - fund_db;

    eprintln!(
        "S7 Rate reduction imaging: fundamental = {:.1} dB, image band = {:.1} dB, \
         relative = {:.1} dB (threshold: > -40 dB)",
        fund_db, image_db, relative_db
    );

    assert!(
        relative_db > -40.0,
        "S7: imaging energy around 11025 Hz should be > -40 dB relative to fundamental, \
         got {:.1} dB",
        relative_db
    );
}
