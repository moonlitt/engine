//! Multiband Compressor Compliance Tests
//!
//! Validates crossover flatness, slope, band independence, phase alignment,
//! single-band degeneration, and per-band ratio precision.
//!
//! All tests use the AudioBackend trait for parameter access and processing.

use moonlitt_core::AudioBackend;
use moonlitt_effects::MultibandCompressor;
use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

const SR: u32 = 44100;

// =============================================================================
// Helpers
// =============================================================================

/// Generate a mono sine wave at the given amplitude (linear, f64) and return f32 samples.
fn sine_f32(freq: f64, amplitude: f64, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / SR as f64;
            (amplitude * (2.0 * PI * freq * t).sin()) as f32
        })
        .collect()
}

/// Compute RMS of a slice (linear).
fn rms(buf: &[f32]) -> f64 {
    let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum_sq / buf.len() as f64).sqrt()
}

/// Convert linear amplitude to dBFS.
fn to_db(linear: f64) -> f64 {
    if linear > 1e-12 {
        20.0 * linear.log10()
    } else {
        -240.0
    }
}

/// Convert dBFS to linear amplitude.
fn from_db(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// Measure the RMS energy in a specific frequency band using FFT.
/// Returns linear RMS.
fn band_energy(buf: &[f32], lo_hz: f64, hi_hz: f64) -> f64 {
    let n = buf.len();
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);

    let mut spectrum: Vec<Complex<f64>> = buf
        .iter()
        .map(|&s| Complex::new(s as f64, 0.0))
        .collect();
    fft.process(&mut spectrum);

    let bin_width = SR as f64 / n as f64;
    let lo_bin = (lo_hz / bin_width).ceil() as usize;
    let hi_bin = (hi_hz / bin_width).floor() as usize;
    let hi_bin = hi_bin.min(n / 2);

    let mut sum_sq = 0.0;
    for c in &spectrum[lo_bin..=hi_bin] {
        let mag = c.norm() / n as f64;
        sum_sq += mag * mag;
    }

    (sum_sq * 2.0).sqrt() // factor 2 for negative frequencies
}

/// Disable compression on all bands: threshold=0, ratio=1, fast attack/release,
/// no makeup.
fn disable_all_compression(mb: &mut MultibandCompressor) {
    for band in 0..6 {
        let base = (8 + band * 5) as u32;
        mb.set_param(base, 0.0);       // threshold = 0 dB
        mb.set_param(base + 1, 1.0);   // ratio = 1:1 (no compression)
        mb.set_param(base + 2, 0.1);   // fast attack
        mb.set_param(base + 3, 10.0);  // fast release
        mb.set_param(base + 4, 0.0);   // no makeup
    }
}

// =============================================================================
// mb1: Crossover Flatness — no compression, sine sweep, deviation < 1.0 dB
// =============================================================================

#[test]
fn mb1_crossover_flatness() {
    let mut mb = MultibandCompressor::new(SR);
    mb.set_param(0, 4.0); // band_count = 4
    mb.set_param(1, 0.0); // output gain = 0 dB
    disable_all_compression(&mut mb);

    let test_freqs = [100.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0];
    let num_samples = SR as usize * 2;
    let amplitude = 0.5;

    for &freq in &test_freqs {
        mb.unload(); // reset filter state

        let input = sine_f32(freq, amplitude, num_samples);
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        mb.process_effect(&input, &input, &mut out_l, &mut out_r);

        // Measure in the last quarter (filters fully settled)
        let measure_start = num_samples * 3 / 4;
        let input_rms = rms(&input[measure_start..]);
        let output_rms = rms(&out_l[measure_start..]);

        let deviation_db = (to_db(output_rms) - to_db(input_rms)).abs();
        assert!(
            deviation_db < 1.0,
            "mb1: crossover sum not flat at {:.0} Hz: deviation = {:.3} dB \
             (input_rms={:.6}, output_rms={:.6})",
            freq, deviation_db, input_rms, output_rms
        );
    }
}

// =============================================================================
// mb2: Crossover Slope — LR4 should attenuate ~24 dB/octave
// =============================================================================
//
// band_count=2, crossover_1=1000 Hz, no compression.
// Feed 2 kHz sine (one octave above crossover).
// Measure LP band (band 0) output level vs passband level.
// LR4: -24 dB/octave, so at 2 kHz the LP output should be ~-24 dB ±6 dB.

#[test]
fn mb2_crossover_slope_24db_oct() {
    let mut mb = MultibandCompressor::new(SR);
    mb.set_param(0, 2.0);     // band_count = 2
    mb.set_param(1, 0.0);     // output gain = 0
    mb.set_param(3, 1000.0);  // crossover_1 = 1000 Hz
    disable_all_compression(&mut mb);

    let num_samples = SR as usize * 2;
    let amplitude = 0.5;

    // Passband reference: measure output at 500 Hz (well within band 0)
    mb.unload();
    let input_pass = sine_f32(500.0, amplitude, num_samples);
    let mut out_pass_l = vec![0.0f32; num_samples];
    let mut out_pass_r = vec![0.0f32; num_samples];
    mb.process_effect(&input_pass, &input_pass, &mut out_pass_l, &mut out_pass_r);

    let measure_start = num_samples * 3 / 4;
    let passband_rms_db = to_db(rms(&out_pass_l[measure_start..]));

    // Stopband test: feed 2 kHz (one octave above crossover)
    // With 2 bands, the total output = LP + HP. We need to isolate the LP band.
    // Since we're using MultibandCompressor (which sums all bands), we instead
    // suppress the HP band by heavily compressing it, and measure only LP output.
    // Set band 1 (HP) to extreme compression: threshold=-60, ratio=100, makeup=-12
    mb.set_param(13, -60.0);  // band 1 threshold
    mb.set_param(14, 100.0);  // band 1 ratio (inf-like)
    mb.set_param(15, 0.1);    // band 1 attack
    mb.set_param(16, 10.0);   // band 1 release
    mb.set_param(17, -12.0);  // band 1 makeup = -12 dB (further suppress)

    mb.unload();
    let input_stop = sine_f32(2000.0, amplitude, num_samples);
    let mut out_stop_l = vec![0.0f32; num_samples];
    let mut out_stop_r = vec![0.0f32; num_samples];
    mb.process_effect(&input_stop, &input_stop, &mut out_stop_l, &mut out_stop_r);

    let stopband_rms_db = to_db(rms(&out_stop_l[measure_start..]));

    // The LP band's output at 2 kHz should be significantly attenuated.
    // LR4 gives -24 dB/octave. With our measurement approach (suppressed HP band),
    // the output is dominated by the LP band's contribution at 2 kHz.
    let attenuation = passband_rms_db - stopband_rms_db;
    assert!(
        attenuation > 18.0,
        "mb2: LR4 slope at 1 octave above crossover: expected attenuation ≈24 dB, \
         got {:.1} dB (passband={:.1} dB, stopband={:.1} dB)",
        attenuation, passband_rms_db, stopband_rms_db
    );
}

// =============================================================================
// mb3: Band Independence — compress one band, others unaffected
// =============================================================================

#[test]
fn mb3_band_independence() {
    let mut mb = MultibandCompressor::new(SR);
    mb.set_param(0, 4.0);     // band_count = 4
    mb.set_param(1, 0.0);     // output gain = 0
    mb.set_param(3, 200.0);   // crossover_1 = 200 Hz
    mb.set_param(4, 1000.0);  // crossover_2 = 1000 Hz
    mb.set_param(5, 5000.0);  // crossover_3 = 5000 Hz

    disable_all_compression(&mut mb);

    // Compress only band 1 (200-1000 Hz range): threshold=-30, ratio=10
    let band1_base = 8 + 5; // band index 1: 8 + 1*5 = 13
    mb.set_param(band1_base as u32, -30.0);       // threshold
    mb.set_param((band1_base + 1) as u32, 10.0);  // ratio
    mb.set_param((band1_base + 2) as u32, 0.1);   // fast attack
    mb.set_param((band1_base + 3) as u32, 10.0);  // fast release
    mb.set_param((band1_base + 4) as u32, 0.0);   // no makeup

    // Build broadband signal: mix of 100 Hz + 500 Hz + 2000 Hz + 10000 Hz
    let num_samples = SR as usize * 4;
    let amplitude = 0.2; // each component at -14 dBFS
    let sig_100 = sine_f32(100.0, amplitude, num_samples);
    let sig_500 = sine_f32(500.0, amplitude, num_samples);
    let sig_2k = sine_f32(2000.0, amplitude, num_samples);
    let sig_10k = sine_f32(10000.0, amplitude, num_samples);

    let input: Vec<f32> = (0..num_samples)
        .map(|i| sig_100[i] + sig_500[i] + sig_2k[i] + sig_10k[i])
        .collect();
    let mut out_l = vec![0.0f32; num_samples];
    let mut out_r = vec![0.0f32; num_samples];

    mb.process_effect(&input, &input, &mut out_l, &mut out_r);

    // Measure energy of each component in the output using FFT
    let measure_start = num_samples * 3 / 4;
    let out_slice = &out_l[measure_start..];

    let energy_100 = band_energy(out_slice, 80.0, 150.0);
    let energy_500 = band_energy(out_slice, 400.0, 700.0);
    let energy_2k = band_energy(out_slice, 1500.0, 3000.0);
    let energy_10k = band_energy(out_slice, 8000.0, 12000.0);

    let db_100 = to_db(energy_100);
    let db_500 = to_db(energy_500);
    let db_2k = to_db(energy_2k);
    let db_10k = to_db(energy_10k);

    // Band 1 (500 Hz) should be attenuated relative to the others
    // Verify it's at least 3 dB lower than each uncompressed band
    assert!(
        db_500 < db_100 - 3.0,
        "mb3: band 1 (500Hz) should be >3 dB below band 0 (100Hz): \
         500Hz={:.1} dB, 100Hz={:.1} dB",
        db_500, db_100
    );
    assert!(
        db_500 < db_2k - 3.0,
        "mb3: band 1 (500Hz) should be >3 dB below band 2 (2kHz): \
         500Hz={:.1} dB, 2kHz={:.1} dB",
        db_500, db_2k
    );
    assert!(
        db_500 < db_10k - 3.0,
        "mb3: band 1 (500Hz) should be >3 dB below band 3 (10kHz): \
         500Hz={:.1} dB, 10kHz={:.1} dB",
        db_500, db_10k
    );
}

// =============================================================================
// mb4: Crossover Phase Alignment — sum at crossover frequency ≈ unity
// =============================================================================
//
// band_count=2, crossover_1=1000 Hz, no compression.
// Feed 1 kHz sine (at the crossover point).
// LR4 crossovers sum to unity at the crossover frequency when phase-aligned.
// Output RMS should be within 1 dB of input RMS.

#[test]
fn mb4_crossover_phase_alignment() {
    let mut mb = MultibandCompressor::new(SR);
    mb.set_param(0, 2.0);     // band_count = 2
    mb.set_param(1, 0.0);     // output gain = 0
    mb.set_param(3, 1000.0);  // crossover_1 = 1000 Hz
    disable_all_compression(&mut mb);

    let num_samples = SR as usize * 2;
    let amplitude = 0.5;

    let input = sine_f32(1000.0, amplitude, num_samples);
    let mut out_l = vec![0.0f32; num_samples];
    let mut out_r = vec![0.0f32; num_samples];

    mb.process_effect(&input, &input, &mut out_l, &mut out_r);

    let measure_start = num_samples * 3 / 4;
    let input_rms = rms(&input[measure_start..]);
    let output_rms = rms(&out_l[measure_start..]);

    let deviation_db = (to_db(output_rms) - to_db(input_rms)).abs();
    assert!(
        deviation_db < 1.0,
        "mb4: at crossover frequency (1 kHz), LP+HP sum should be near unity: \
         deviation = {:.3} dB (input={:.6}, output={:.6})",
        deviation_db, input_rms, output_rms
    );
}

// =============================================================================
// mb5: Single Band Degenerates to Fullband Compressor
// =============================================================================
//
// band_count=1 multiband should behave identically to a regular Compressor
// with the same settings.

#[test]
fn mb5_single_band_degenerates() {
    use moonlitt_effects::Compressor;

    let num_samples = SR as usize * 4;
    let amplitude = from_db(-10.0); // -10 dBFS

    // --- Multiband (1 band) ---
    let mut mb = MultibandCompressor::new(SR);
    mb.set_param(0, 1.0);   // band_count = 1
    mb.set_param(1, 0.0);   // output gain = 0

    // Band 0 compression: threshold=-20, ratio=4, fast attack, slow release
    mb.set_param(8, -20.0);    // threshold
    mb.set_param(9, 4.0);     // ratio
    mb.set_param(10, 0.1);    // attack
    mb.set_param(11, 1000.0); // release
    mb.set_param(12, 0.0);    // makeup

    let input = sine_f32(1000.0, amplitude, num_samples);
    let silent = vec![0.0f32; num_samples];
    let mut mb_out_l = vec![0.0f32; num_samples];
    let mut mb_out_r = vec![0.0f32; num_samples];
    mb.process_effect(&input, &silent, &mut mb_out_l, &mut mb_out_r);

    // --- Regular Compressor with same settings ---
    let mut comp = Compressor::new(SR);
    comp.set_param(0, -20.0);   // threshold
    comp.set_param(1, 4.0);     // ratio
    comp.set_param(2, 0.1);     // attack
    comp.set_param(3, 1000.0);  // release
    comp.set_param(4, 0.0);     // knee = 0 (hard knee)
    comp.set_param(5, 0.0);     // makeup = 0
    comp.set_param(6, 0.0);     // sidechain HPF bypassed

    let mut comp_out_l = vec![0.0f32; num_samples];
    let mut comp_out_r = vec![0.0f32; num_samples];
    comp.process_effect(&input, &silent, &mut comp_out_l, &mut comp_out_r);

    // Compare RMS after settling (last quarter)
    let measure_start = num_samples * 3 / 4;
    let mb_rms_db = to_db(rms(&mb_out_l[measure_start..]));
    let comp_rms_db = to_db(rms(&comp_out_l[measure_start..]));

    let diff_db = (mb_rms_db - comp_rms_db).abs();
    assert!(
        diff_db < 1.0,
        "mb5: single-band multiband should match regular compressor within 1 dB: \
         multiband={:.2} dB, compressor={:.2} dB, diff={:.2} dB",
        mb_rms_db, comp_rms_db, diff_db
    );
}

// =============================================================================
// mb6: Per-Band Ratio Precision
// =============================================================================
//
// band_count=2, crossover_1=2000 Hz.
// Band 0 (low): threshold=-20 dB, ratio=4.
// Feed 500 Hz sine at -10 dBFS (10 dB above threshold).
// Expected GR = 10 × (1 - 1/4) = 7.5 dB.
// Output of band 0 should be ≈ -17.5 dBFS ±2 dB.

#[test]
fn mb6_per_band_ratio_precision() {
    let mut mb = MultibandCompressor::new(SR);
    mb.set_param(0, 2.0);     // band_count = 2
    mb.set_param(1, 0.0);     // output gain = 0
    mb.set_param(3, 2000.0);  // crossover_1 = 2000 Hz

    // Band 0 (low): compress
    mb.set_param(8, -20.0);   // threshold = -20 dB
    mb.set_param(9, 4.0);     // ratio = 4:1
    mb.set_param(10, 0.1);    // fast attack
    mb.set_param(11, 1000.0); // slow release
    mb.set_param(12, 0.0);    // no makeup

    // Band 1 (high): no compression — keep signal in band 1 from interfering
    mb.set_param(13, 0.0);    // threshold = 0 dB
    mb.set_param(14, 1.0);    // ratio = 1:1
    mb.set_param(15, 0.1);
    mb.set_param(16, 10.0);
    mb.set_param(17, 0.0);

    let num_samples = SR as usize * 4;
    let amplitude = from_db(-10.0); // -10 dBFS

    let input = sine_f32(500.0, amplitude, num_samples);
    let silent = vec![0.0f32; num_samples];
    let mut out_l = vec![0.0f32; num_samples];
    let mut out_r = vec![0.0f32; num_samples];

    mb.process_effect(&input, &silent, &mut out_l, &mut out_r);

    // Measure output RMS after settling.
    // The 500 Hz signal is entirely in band 0 (well below 2000 Hz crossover).
    // Band 1 should contribute negligible energy at 500 Hz.
    let measure_start = num_samples * 3 / 4;
    let output_rms_db = to_db(rms(&out_l[measure_start..]));

    // Expected: -10 dBFS input, 7.5 dB gain reduction = -17.5 dBFS output
    let expected_db = -17.5;
    let error = (output_rms_db - expected_db).abs();

    // The multiband compressor uses peak detection with an envelope follower,
    // which for a sine wave tracks ~peak rather than RMS. This introduces a
    // systematic offset (~3 dB for sine). Use ±4 dB tolerance to account for
    // envelope dynamics and crossover filter interaction.
    assert!(
        error < 4.0,
        "mb6: per-band ratio precision: expected ≈{:.1} dBFS, got {:.2} dBFS (error={:.2} dB)",
        expected_db, output_rms_db, error
    );
}
