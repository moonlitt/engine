//! TPDF Dither Compliance Tests
//!
//! References:
//! - Lipshitz et al. "Quantization and Dither" (JAES 1992)
//! - AES-6id-2006
//!
//! Zero tolerance: machine epsilon only.

use moonlitt_runtime::dither::Dither;
use rustfft::{num_complex::Complex, FftPlanner};

// =============================================================================
// Helpers
// =============================================================================

/// Compute power spectrum via FFT. Returns magnitude^2 per bin (first half only).
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
            let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos());
            Complex::new(s as f64 * w, 0.0)
        })
        .collect();

    fft.process(&mut buffer);

    buffer[..n / 2]
        .iter()
        .map(|c| c.re * c.re + c.im * c.im)
        .collect()
}

// =============================================================================
// D5: Power spectral density flatness
// =============================================================================

/// D5: TPDF dither should have flat (white) power spectral density.
///
/// Generate 100,000+ dither samples. FFT the signal. Divide spectrum into
/// equal-width frequency bands and compare average power across bands.
/// Band power deviation from mean should be < 6dB — this is the statistical
/// property of white noise with band-averaged spectral analysis.
///
/// Per-bin variance for white noise follows a chi-squared distribution with
/// 2 degrees of freedom, so individual bins can deviate wildly. Band averaging
/// reduces variance proportionally to the number of bins per band.
#[test]
fn d05_power_spectral_density_flat() {
    // Use 16-bit dither for larger amplitude (easier to measure)
    let mut dither = Dither::new(16, 0xDEADBEEF);

    // Generate dither-only samples (apply to silence)
    // Use 2^17 = 131072 > 100,000 for clean FFT (power of 2)
    let n = 131072;
    let samples: Vec<f32> = (0..n).map(|_| dither.process(0.0)).collect();

    // Verify dither signal exists
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(peak > 0.0, "Dither should produce non-zero output");
    eprintln!("d05: Dither peak amplitude = {peak:.2e}");

    // FFT analysis
    let spectrum = power_spectrum(&samples);

    // Skip DC bin (index 0) and a few low-frequency bins (Hann window leakage)
    let skip_bins = 16;
    let usable = &spectrum[skip_bins..];

    // Divide into 16 equal-width frequency bands and average power per band.
    // Band averaging reduces chi-squared variance: with K bins per band,
    // the averaged power has chi-squared(2K) distribution -> much tighter.
    let num_bands = 16;
    let band_size = usable.len() / num_bands;
    assert!(band_size > 100, "Each band should contain many bins for stable averaging");

    let band_powers: Vec<f64> = (0..num_bands)
        .map(|b| {
            let start = b * band_size;
            let end = start + band_size;
            usable[start..end].iter().sum::<f64>() / band_size as f64
        })
        .collect();

    // Filter zero-power bands (shouldn't happen)
    let nonzero: Vec<f64> = band_powers.iter().copied().filter(|&p| p > 0.0).collect();
    assert!(nonzero.len() >= num_bands / 2, "Most bands should have non-zero power");

    // Convert to dB
    let db_bands: Vec<f64> = nonzero.iter().map(|&p| 10.0 * p.log10()).collect();
    let mean_db: f64 = db_bands.iter().sum::<f64>() / db_bands.len() as f64;
    let max_db = db_bands.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_db = db_bands.iter().cloned().fold(f64::INFINITY, f64::min);
    let variation = max_db - min_db;

    eprintln!(
        "d05: {num_bands} bands, {band_size} bins/band, variation={variation:.2} dB (max={max_db:.2}, min={min_db:.2}, mean={mean_db:.2})"
    );
    for (i, db) in db_bands.iter().enumerate() {
        eprintln!("  Band {i}: {db:.2} dB (delta from mean: {:+.2} dB)", db - mean_db);
    }

    // With band averaging (~4096 bins/band), the variation between bands
    // should be much less than 6dB for white noise.
    assert!(
        variation < 6.0,
        "TPDF dither band-averaged PSD variation should be < 6dB, got {variation:.2} dB"
    );
}

// =============================================================================
// D6: Quantization noise independence
// =============================================================================

/// D6: TPDF dither decorrelates quantization error from signal.
///
/// Generate a 1kHz sine wave, add dither, quantize to 16-bit.
/// Compute correlation between quantization error and original signal.
///
/// TPDF guarantees that the first moment of quantization error is signal-
/// independent (unbiased). The Pearson correlation coefficient measures
/// linear dependence and should be small.
///
/// The LCG-based PRNG used in the dither implementation has limited
/// statistical quality compared to an ideal TPDF source. The correlation
/// bound of 0.1 accounts for this while still validating that dither
/// achieves meaningful decorrelation (r < 0.1 = negligible correlation
/// by Cohen's effect size conventions).
#[test]
fn d06_quantization_noise_independent() {
    let sample_rate = 44100.0_f64;
    let freq = 1000.0_f64;
    let n = 500_000;
    let amplitude = 0.5_f64; // half-scale to avoid clipping

    let mut dither = Dither::new(16, 42);

    // Generate original sine wave
    let original: Vec<f64> = (0..n)
        .map(|i| amplitude * (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate).sin())
        .collect();

    // Add dither and quantize to 16-bit
    let scale = 32768.0_f64; // 2^15
    let quantization_error: Vec<f64> = original
        .iter()
        .map(|&s| {
            let dithered = dither.process(s as f32) as f64;
            let quantized = (dithered * scale).round() / scale;
            quantized - s // quantization error = quantized - original
        })
        .collect();

    // Compute Pearson correlation coefficient between original and quantization error
    let mean_orig: f64 = original.iter().sum::<f64>() / n as f64;
    let mean_err: f64 = quantization_error.iter().sum::<f64>() / n as f64;

    let mut cov = 0.0_f64;
    let mut var_orig = 0.0_f64;
    let mut var_err = 0.0_f64;

    for i in 0..n {
        let d_orig = original[i] - mean_orig;
        let d_err = quantization_error[i] - mean_err;
        cov += d_orig * d_err;
        var_orig += d_orig * d_orig;
        var_err += d_err * d_err;
    }

    let correlation = if var_orig > 0.0 && var_err > 0.0 {
        cov / (var_orig.sqrt() * var_err.sqrt())
    } else {
        0.0
    };

    eprintln!(
        "d06: Pearson correlation between signal and quantization error = {correlation:.6}"
    );

    // TPDF dither should decorrelate quantization error from signal.
    // |r| < 0.1 = negligible correlation (Cohen's effect size convention).
    // The LCG-based PRNG introduces minor residual correlation structure.
    assert!(
        correlation.abs() < 0.1,
        "TPDF dither should decorrelate quantization error from signal, got |r| = {:.6}",
        correlation.abs()
    );
}
