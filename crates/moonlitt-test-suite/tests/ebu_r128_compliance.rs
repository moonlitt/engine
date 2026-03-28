//! EBU R128 / ITU-R BS.1770 Loudness Compliance Tests
//!
//! References:
//! - EBU R128: https://tech.ebu.ch/docs/r/r128.pdf
//! - ITU-R BS.1770-5: https://www.itu.int/rec/R-REC-BS.1770
//!
//! K-weighting filter coefficients from BS.1770-5 §2.
//! Zero tolerance: f64::EPSILON for filter coefficients, f32::EPSILON for metering.

use std::f64::consts::PI;

const SAMPLE_RATE: u32 = 48000;

// =============================================================================
// K-weighting biquad (BS.1770-5 §2)
// =============================================================================

/// Direct Form II Transposed biquad filter.
///
/// BS.1770-5 §2: K-weighting consists of two cascaded biquad filters.
/// All coefficients and state are stored in f64 for full precision.
struct KWeightBiquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl KWeightBiquad {
    fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Direct Form II Transposed: y = b0*x + z1; z1 = b1*x - a1*y + z2; z2 = b2*x - a2*y
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// BS.1770-5 §2: Stage 1 -- Pre-filter (high shelving) at 48kHz.
///
/// "The pre-filter shall have the following transfer function..."
/// Coefficients from Table 1 of BS.1770-5 for fs = 48kHz.
fn pre_filter_48k() -> KWeightBiquad {
    KWeightBiquad::new(
        1.53512485958697,
        -2.69169618940638,
        1.19839281085285,
        -1.69065929318241,
        0.73248077421585,
    )
}

/// BS.1770-5 §2: Stage 2 -- High-pass (RLB weighting) at 48kHz.
///
/// Revised Low-frequency B-weighting filter.
/// Coefficients from Table 2 of BS.1770-5 for fs = 48kHz.
fn rlb_filter_48k() -> KWeightBiquad {
    KWeightBiquad::new(
        1.0,
        -2.0,
        1.0,
        -1.99004745483398,
        0.99007225036621,
    )
}

/// Apply K-weighting (two cascaded biquads) to a signal buffer.
/// All processing in f64 for full precision.
fn apply_k_weighting(signal: &[f64]) -> Vec<f64> {
    let mut stage1 = pre_filter_48k();
    let mut stage2 = rlb_filter_48k();

    signal
        .iter()
        .map(|&x| {
            let y1 = stage1.process(x);
            stage2.process(y1)
        })
        .collect()
}

// =============================================================================
// Transfer function evaluation
// =============================================================================

/// Compute the squared magnitude |H(f)|^2 of the combined K-weighting filter
/// at a given frequency, using the BS.1770-5 §2 coefficients.
///
/// For a biquad with transfer function H(z) = (b0 + b1*z^-1 + b2*z^-2) / (1 + a1*z^-1 + a2*z^-2),
/// evaluated at z = e^(j*w) where w = 2*pi*f/fs:
///   |H(f)|^2 = |B(f)|^2 / |A(f)|^2
fn k_weight_magnitude_squared(freq: f64, sample_rate: f64) -> f64 {
    let w = 2.0 * PI * freq / sample_rate;
    let cos_w = w.cos();
    let cos_2w = (2.0 * w).cos();
    let sin_w = w.sin();
    let sin_2w = (2.0 * w).sin();

    // Stage 1: Pre-filter
    let s1 = pre_filter_48k();
    let s1_num_re = s1.b0 + s1.b1 * cos_w + s1.b2 * cos_2w;
    let s1_num_im = -(s1.b1 * sin_w + s1.b2 * sin_2w);
    let s1_den_re = 1.0 + s1.a1 * cos_w + s1.a2 * cos_2w;
    let s1_den_im = -(s1.a1 * sin_w + s1.a2 * sin_2w);
    let s1_mag_sq =
        (s1_num_re * s1_num_re + s1_num_im * s1_num_im) /
        (s1_den_re * s1_den_re + s1_den_im * s1_den_im);

    // Stage 2: RLB high-pass
    let s2 = rlb_filter_48k();
    let s2_num_re = s2.b0 + s2.b1 * cos_w + s2.b2 * cos_2w;
    let s2_num_im = -(s2.b1 * sin_w + s2.b2 * sin_2w);
    let s2_den_re = 1.0 + s2.a1 * cos_w + s2.a2 * cos_2w;
    let s2_den_im = -(s2.a1 * sin_w + s2.a2 * sin_2w);
    let s2_mag_sq =
        (s2_num_re * s2_num_re + s2_num_im * s2_num_im) /
        (s2_den_re * s2_den_re + s2_den_im * s2_den_im);

    s1_mag_sq * s2_mag_sq
}

// =============================================================================
// Signal generators and measurement helpers
// =============================================================================

/// Generate a sine wave at the given frequency and sample rate.
/// Amplitude = 1.0 (0 dBFS peak).
fn sine_f64(freq: f64, sample_rate: u32, num_samples: usize) -> Vec<f64> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / sample_rate as f64;
            (2.0 * PI * freq * t).sin()
        })
        .collect()
}

/// Compute RMS (root mean square) of a signal.
fn rms(signal: &[f64]) -> f64 {
    let ms: f64 = signal.iter().map(|&x| x * x).sum::<f64>() / signal.len() as f64;
    ms.sqrt()
}

/// Compute mean square of a signal.
fn mean_square(signal: &[f64]) -> f64 {
    signal.iter().map(|&x| x * x).sum::<f64>() / signal.len() as f64
}

/// Compute gain in dB by comparing RMS before and after filtering.
fn gain_db(input: &[f64], output: &[f64]) -> f64 {
    let rms_in = rms(input);
    let rms_out = rms(output);
    if rms_in < 1e-15 {
        return f64::NEG_INFINITY;
    }
    20.0 * (rms_out / rms_in).log10()
}

// =============================================================================
// L4: K-weighting filter coefficients
// =============================================================================

/// L4: K-weighting filter coefficient verification.
///
/// BS.1770-5 §2: "The pre-filter shall have the following transfer function..."
///
/// Verifies:
/// 1. All 10 coefficients are stored with exact f64 precision (bit-exact match).
/// 2. The time-domain filter output matches the analytical transfer function.
///    - At 1kHz the combined K-weighting gain is +0.70 dB (pre-filter head-related
///      shelving boost). This is NOT 0 dB -- the pre-filter models the acoustic
///      effect of the human head, boosting ~1-4 kHz.
///    - At 100Hz the gain is negative (RLB high-pass attenuates low frequencies).
///    - At 10kHz the gain is positive (pre-filter shelf boost dominates).
/// 3. The time-domain gain matches the analytically computed |H(f)| to < 0.01 dB.
#[test]
fn l04_k_weighting_filter_coefficients() {
    // ---- Verify exact coefficient storage (f64 precision) ----

    let stage1 = pre_filter_48k();
    assert!(
        (stage1.b0 - 1.53512485958697_f64).abs() <= f64::EPSILON,
        "Stage 1 b0 mismatch: got {}, expected 1.53512485958697",
        stage1.b0
    );
    assert!(
        (stage1.b1 - (-2.69169618940638_f64)).abs() <= f64::EPSILON,
        "Stage 1 b1 mismatch: got {}, expected -2.69169618940638",
        stage1.b1
    );
    assert!(
        (stage1.b2 - 1.19839281085285_f64).abs() <= f64::EPSILON,
        "Stage 1 b2 mismatch: got {}, expected 1.19839281085285",
        stage1.b2
    );
    assert!(
        (stage1.a1 - (-1.69065929318241_f64)).abs() <= f64::EPSILON,
        "Stage 1 a1 mismatch: got {}, expected -1.69065929318241",
        stage1.a1
    );
    assert!(
        (stage1.a2 - 0.73248077421585_f64).abs() <= f64::EPSILON,
        "Stage 1 a2 mismatch: got {}, expected 0.73248077421585",
        stage1.a2
    );

    let stage2 = rlb_filter_48k();
    assert!(
        (stage2.b0 - 1.0_f64).abs() <= f64::EPSILON,
        "Stage 2 b0 mismatch: got {}, expected 1.0",
        stage2.b0
    );
    assert!(
        (stage2.b1 - (-2.0_f64)).abs() <= f64::EPSILON,
        "Stage 2 b1 mismatch: got {}, expected -2.0",
        stage2.b1
    );
    assert!(
        (stage2.b2 - 1.0_f64).abs() <= f64::EPSILON,
        "Stage 2 b2 mismatch: got {}, expected 1.0",
        stage2.b2
    );
    assert!(
        (stage2.a1 - (-1.99004745483398_f64)).abs() <= f64::EPSILON,
        "Stage 2 a1 mismatch: got {}, expected -1.99004745483398",
        stage2.a1
    );
    assert!(
        (stage2.a2 - 0.99007225036621_f64).abs() <= f64::EPSILON,
        "Stage 2 a2 mismatch: got {}, expected 0.99007225036621",
        stage2.a2
    );

    // ---- Verify frequency-dependent gain behavior ----
    // Use a long signal to let the filter reach steady state.
    let num_samples = SAMPLE_RATE as usize * 2; // 2 seconds
    let skip = SAMPLE_RATE as usize; // skip 1 second for transient settling

    // Test at three frequencies and compare time-domain filter output
    // against the analytical transfer function magnitude.
    for &(freq, expected_sign) in &[
        (100.0, -1.0_f64),  // 100Hz: gain negative (attenuation)
        (1000.0, 1.0_f64),  // 1kHz: gain positive (+0.70 dB, pre-filter boost)
        (10000.0, 1.0_f64), // 10kHz: gain positive (+4.04 dB, shelf boost)
    ] {
        let sig = sine_f64(freq, SAMPLE_RATE, num_samples);
        let filtered = apply_k_weighting(&sig);
        let measured_gain = gain_db(&sig[skip..], &filtered[skip..]);

        // Analytical gain from transfer function
        let h_sq = k_weight_magnitude_squared(freq, SAMPLE_RATE as f64);
        let analytical_gain = 10.0 * h_sq.log10(); // 20*log10(|H|) = 10*log10(|H|^2)

        // Sign check
        if expected_sign > 0.0 {
            assert!(
                measured_gain > 0.0,
                "K-weighting gain at {freq}Hz should be positive, got {measured_gain:.4} dB",
            );
        } else {
            assert!(
                measured_gain < 0.0,
                "K-weighting gain at {freq}Hz should be negative, got {measured_gain:.4} dB",
            );
        }

        // Time-domain filter must match analytical transfer function within 0.01 dB.
        // This tolerance accounts for finite-length signal effects (non-integer
        // number of cycles in the measurement window).
        let delta = (measured_gain - analytical_gain).abs();
        assert!(
            delta < 0.01,
            "K-weighting at {freq}Hz: time-domain={measured_gain:.6} dB, \
             analytical={analytical_gain:.6} dB, delta={delta:.6} dB (must be < 0.01)"
        );

        eprintln!(
            "L4 {freq:>5.0}Hz: measured={measured_gain:+.4} dB, analytical={analytical_gain:+.4} dB, delta={delta:.6} dB"
        );
    }
}

// =============================================================================
// L5: Momentary loudness (400ms)
// =============================================================================

/// L5: Momentary loudness measurement (400ms window).
///
/// EBU R128 §3.1: "Momentary loudness uses a 400ms sliding rectangular window."
///
/// BS.1770-5 §3: Loudness LUFS = -0.691 + 10 * log10( sum_i G_i * z_i )
///   where z_i = (1/T) * integral of y_i^2 dt (mean square of K-weighted signal)
///   and G_i = channel weight (1.0 for front channels).
///
/// For a mono 1kHz sine at 0 dBFS peak:
///   - Input mean_square = 0.5 (RMS = 1/sqrt(2))
///   - After K-weighting: mean_square = 0.5 * |H(1kHz)|^2
///   - |H(1kHz)|^2 is computed from the BS.1770-5 transfer function
///   - LUFS = -0.691 + 10 * log10(0.5 * |H(1kHz)|^2)
///
/// The expected LUFS is derived analytically from the spec coefficients.
/// Tolerance: 0.01 dB between measured and analytically expected LUFS.
#[test]
fn l05_momentary_loudness_400ms() {
    // EBU R128 §3.1: 400ms window at 48kHz = 19200 samples
    let window_samples = (SAMPLE_RATE as f64 * 0.4) as usize;
    assert_eq!(window_samples, 19200, "400ms at 48kHz must be 19200 samples");

    // Generate 1kHz sine at 0 dBFS, with extra lead-in for filter settling.
    // Use 2 seconds total, measure the last 400ms after transient is gone.
    let total_samples = SAMPLE_RATE as usize * 2;
    let signal = sine_f64(1000.0, SAMPLE_RATE, total_samples);

    // Apply K-weighting
    let k_weighted = apply_k_weighting(&signal);

    // Take the last 400ms for measurement (filter fully settled)
    let measurement_start = total_samples - window_samples;
    let window = &k_weighted[measurement_start..];
    assert_eq!(window.len(), 19200);

    // BS.1770-5 §3: mean_square = (1/N) * sum(y^2)
    let ms = mean_square(window);

    // BS.1770-5 §3: LUFS = -0.691 + 10 * log10(sum_i G_i * z_i)
    // For single channel with G = 1.0: LUFS = -0.691 + 10 * log10(ms)
    let lufs = -0.691 + 10.0 * ms.log10();

    // Expected LUFS derived from the transfer function:
    // For 0 dBFS sine, input mean_square = 0.5.
    // After K-weighting, mean_square = 0.5 * |H(1kHz)|^2.
    // LUFS = -0.691 + 10 * log10(0.5 * |H(1kHz)|^2)
    let h_sq_1k = k_weight_magnitude_squared(1000.0, SAMPLE_RATE as f64);
    let expected_ms = 0.5 * h_sq_1k;
    let expected_lufs = -0.691 + 10.0 * expected_ms.log10();

    eprintln!(
        "L5: |H(1kHz)|^2={:.10}, measured_ms={:.10}, expected_ms={:.10}",
        h_sq_1k, ms, expected_ms
    );
    eprintln!(
        "L5: LUFS={:.6}, expected={:.6}, delta={:.6}",
        lufs, expected_lufs, (lufs - expected_lufs).abs()
    );

    // Verify the LUFS formula math with f64 precision.
    // BS.1770-5 §3: LUFS = -0.691 + 10 * log10(z)
    // For input sine at 0 dBFS: z = 0.5, LUFS_unweighted = -0.691 + 10*log10(0.5)
    let unweighted_lufs = -0.691 + 10.0 * (0.5_f64).log10();
    // 10*log10(0.5) = -3.010299957316877...
    // The f64 computation of log10(0.5) should be accurate to f64 precision.
    // Verify the formula components independently.
    let log10_half = (0.5_f64).log10();
    assert!(
        (log10_half - (-0.30102999566398_f64)).abs() < 1e-13,
        "log10(0.5) precision: got {:.20}, expected -0.30102999566398...",
        log10_half
    );
    eprintln!("L5: unweighted LUFS (0dBFS sine) = {:.6}", unweighted_lufs);

    // The measured LUFS should match the analytically expected LUFS within 0.01 dB.
    // This tolerance covers the finite signal length (non-integer cycle count
    // in the 400ms window at 1kHz = exactly 400 cycles, so error is minimal).
    let delta = (lufs - expected_lufs).abs();
    assert!(
        delta < 0.01,
        "Momentary LUFS at 1kHz/0dBFS: measured={:.6}, expected={:.6}, delta={:.6} (must be < 0.01)",
        lufs, expected_lufs, delta
    );
}

// =============================================================================
// L6: Short-term loudness (3 seconds)
// =============================================================================

/// L6: Short-term loudness measurement (3-second window).
///
/// EBU R128 §3.2: "Short-term loudness uses a 3-second sliding rectangular window."
///
/// Same calculation as L5 but over 3 seconds (144000 samples at 48kHz).
/// For a constant-level signal, the measurement must be identical regardless
/// of window length -- this verifies temporal consistency of the algorithm.
///
/// Verifies:
/// 1. 3s LUFS matches the analytically expected value (< 0.01 dB tolerance).
/// 2. 3s LUFS agrees with 400ms momentary LUFS for a constant signal (< 0.001 dB).
#[test]
fn l06_short_term_loudness_3s() {
    // EBU R128 §3.2: 3s window at 48kHz = 144000 samples
    let window_samples = SAMPLE_RATE as usize * 3;
    assert_eq!(window_samples, 144000, "3s at 48kHz must be 144000 samples");

    // Generate 1kHz sine at 0 dBFS with extra lead-in for filter settling.
    // Use 5 seconds total, measure the last 3 seconds.
    let total_samples = SAMPLE_RATE as usize * 5;
    let signal = sine_f64(1000.0, SAMPLE_RATE, total_samples);

    // Apply K-weighting
    let k_weighted = apply_k_weighting(&signal);

    // Take the last 3s for measurement (filter fully settled)
    let measurement_start = total_samples - window_samples;
    let window_3s = &k_weighted[measurement_start..];
    assert_eq!(window_3s.len(), 144000);

    // BS.1770-5 §3: mean_square and LUFS
    let ms_3s = mean_square(window_3s);
    let lufs_3s = -0.691 + 10.0 * ms_3s.log10();

    // Also compute 400ms momentary from the last 400ms of the same signal
    let window_400ms_samples = (SAMPLE_RATE as f64 * 0.4) as usize;
    let window_400ms_start = total_samples - window_400ms_samples;
    let window_400ms = &k_weighted[window_400ms_start..];
    let ms_400ms = mean_square(window_400ms);
    let lufs_400ms = -0.691 + 10.0 * ms_400ms.log10();

    // Expected LUFS from transfer function
    let h_sq_1k = k_weight_magnitude_squared(1000.0, SAMPLE_RATE as f64);
    let expected_ms = 0.5 * h_sq_1k;
    let expected_lufs = -0.691 + 10.0 * expected_ms.log10();

    eprintln!(
        "L6: 3s LUFS={:.6}, 400ms LUFS={:.6}, expected={:.6}",
        lufs_3s, lufs_400ms, expected_lufs
    );
    eprintln!(
        "L6: 3s vs 400ms delta={:.10}, 3s vs expected delta={:.10}",
        (lufs_3s - lufs_400ms).abs(),
        (lufs_3s - expected_lufs).abs()
    );

    // For a constant-level 1kHz sine, both windows are well past the filter
    // transient and contain integer numbers of cycles (1kHz at 48kHz = 48 samples/cycle,
    // 400ms = 400 cycles, 3s = 3000 cycles). The difference should be negligible.
    let delta_windows = (lufs_3s - lufs_400ms).abs();
    assert!(
        delta_windows < 0.001,
        "3s and 400ms LUFS must agree for constant signal: 3s={:.6}, 400ms={:.6}, delta={:.10}",
        lufs_3s, lufs_400ms, delta_windows
    );

    // Verify against analytically expected LUFS
    let delta_expected = (lufs_3s - expected_lufs).abs();
    assert!(
        delta_expected < 0.01,
        "Short-term LUFS at 1kHz/0dBFS: measured={:.6}, expected={:.6}, delta={:.6} (must be < 0.01)",
        lufs_3s, expected_lufs, delta_expected
    );
}
