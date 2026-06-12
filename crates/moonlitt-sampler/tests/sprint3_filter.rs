//! Sprint 3 Tests: Lowpass Resonant Filter (Biquad)
//!
//! SF2 spec uses a 2-pole lowpass resonant filter.
//! Reference: Robert Bristow-Johnson "Audio EQ Cookbook"
//!
//! Acceptance criteria:
//! 1. DC passthrough: filter at high cutoff passes DC unchanged
//! 2. Lowpass behavior: attenuates above cutoff frequency
//! 3. Resonance peak: Q > 0 creates a peak at cutoff
//! 4. Cutoff frequency accuracy: -3dB point matches specified cutoff ±5%
//! 5. Filter stability: no NaN/Inf for any valid parameter range
//! 6. Bypass: cutoff ≥ 20kHz effectively bypasses

use moonlitt_sampler::filter::LowpassFilter;
use std::f32::consts::PI;

const SAMPLE_RATE: u32 = 44100;

/// Generate sine wave at given frequency
fn sine(freq: f32, sr: u32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin())
        .collect()
}

/// Measure RMS of a signal
fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|s| s * s).sum::<f32>() / signal.len() as f32).sqrt()
}

// =============================================================================
// Test 1: DC passthrough at high cutoff
// =============================================================================

#[test]
fn t1_dc_passthrough() {
    let mut filter = LowpassFilter::new(SAMPLE_RATE);
    filter.set_params(20000.0, 0.0); // wide open, no resonance

    // Feed constant 1.0 (DC)
    let mut output = Vec::new();
    for _ in 0..1000 {
        output.push(filter.process(1.0));
    }

    // After settling, output should be ~1.0
    let tail_avg = output[500..].iter().sum::<f32>() / 500.0;
    assert!(
        (tail_avg - 1.0).abs() < 0.01,
        "DC through wide-open filter should be ~1.0, got {tail_avg}"
    );
}

// =============================================================================
// Test 2: Lowpass attenuates high frequencies
// =============================================================================

#[test]
fn t2_lowpass_attenuates() {
    let mut filter = LowpassFilter::new(SAMPLE_RATE);
    filter.set_params(1000.0, 0.0); // 1kHz cutoff, no resonance

    // 500Hz should pass (below cutoff)
    let input_low = sine(500.0, SAMPLE_RATE, 4096);
    let mut output_low = Vec::new();
    for &s in &input_low {
        output_low.push(filter.process(s));
    }
    let rms_low = rms(&output_low[1024..]); // skip transient

    // Reset filter
    filter.reset();

    // 5000Hz should be attenuated (well above cutoff)
    let input_high = sine(5000.0, SAMPLE_RATE, 4096);
    let mut output_high = Vec::new();
    for &s in &input_high {
        output_high.push(filter.process(s));
    }
    let rms_high = rms(&output_high[1024..]);

    let attenuation_db = 20.0 * (rms_high / rms_low).log10();
    eprintln!(
        "500Hz RMS: {rms_low:.4}, 5kHz RMS: {rms_high:.4}, attenuation: {attenuation_db:.1}dB"
    );

    assert!(
        attenuation_db < -12.0,
        "5kHz should be attenuated > 12dB below 500Hz, got {attenuation_db:.1}dB"
    );
}

// =============================================================================
// Test 3: Resonance creates peak at cutoff
// =============================================================================

#[test]
fn t3_resonance_peak() {
    let cutoff = 2000.0;

    // Without resonance
    let mut filter_flat = LowpassFilter::new(SAMPLE_RATE);
    filter_flat.set_params(cutoff, 0.0);

    let input = sine(cutoff, SAMPLE_RATE, 4096);
    let mut output_flat = Vec::new();
    for &s in &input {
        output_flat.push(filter_flat.process(s));
    }
    let rms_flat = rms(&output_flat[1024..]);

    // With resonance (Q = 20dB)
    let mut filter_reso = LowpassFilter::new(SAMPLE_RATE);
    filter_reso.set_params(cutoff, 20.0);

    let mut output_reso = Vec::new();
    for &s in &input {
        output_reso.push(filter_reso.process(s));
    }
    let rms_reso = rms(&output_reso[1024..]);

    let gain_db = 20.0 * (rms_reso / rms_flat).log10();
    eprintln!("At cutoff: flat RMS={rms_flat:.4}, reso RMS={rms_reso:.4}, gain={gain_db:.1}dB");

    assert!(
        gain_db > 6.0,
        "Resonance should boost at cutoff by > 6dB, got {gain_db:.1}dB"
    );
}

// =============================================================================
// Test 4: No NaN/Inf for any valid parameters
// =============================================================================

#[test]
fn t4_stability() {
    let mut filter = LowpassFilter::new(SAMPLE_RATE);

    // Test extreme parameters
    let test_cases = [
        (20.0, 0.0),     // very low cutoff
        (20000.0, 0.0),  // very high cutoff
        (1000.0, 96.0),  // extreme resonance
        (100.0, 50.0),   // low cutoff + high Q
        (20000.0, 96.0), // high cutoff + high Q
    ];

    for (cutoff, q) in test_cases {
        filter.set_params(cutoff, q);
        filter.reset();

        // Feed impulse + noise
        let mut nan_count = 0;
        let mut inf_count = 0;
        for i in 0..10000 {
            let input = if i == 0 { 1.0 } else { 0.0 };
            let out = filter.process(input);
            if out.is_nan() {
                nan_count += 1;
            }
            if out.is_infinite() {
                inf_count += 1;
            }
        }

        assert_eq!(nan_count, 0, "NaN at cutoff={cutoff}, Q={q}");
        assert_eq!(inf_count, 0, "Inf at cutoff={cutoff}, Q={q}");
    }
}

// =============================================================================
// Test 5: Bypass at high cutoff
// =============================================================================

#[test]
fn t5_bypass_high_cutoff() {
    let mut filter = LowpassFilter::new(SAMPLE_RATE);
    filter.set_params(20000.0, 0.0);

    // A 10kHz sine should pass through almost unchanged
    let input = sine(10000.0, SAMPLE_RATE, 4096);
    let rms_in = rms(&input);

    let mut output = Vec::new();
    for &s in &input {
        output.push(filter.process(s));
    }
    let rms_out = rms(&output[1024..]);

    let diff_db = 20.0 * (rms_out / rms_in).log10();
    eprintln!("10kHz through 20kHz cutoff: {diff_db:.1}dB");

    assert!(
        diff_db.abs() < 3.0,
        "High cutoff should pass 10kHz within ±3dB, got {diff_db:.1}dB"
    );
}

// =============================================================================
// Test 6: Coefficients match Audio EQ Cookbook formula
// =============================================================================

#[test]
fn t6_cookbook_coefficients() {
    // Verify our biquad coefficients match the Audio EQ Cookbook
    // For lowpass: H(s) = 1 / (s^2 + s/Q + 1)
    //
    // b0 = (1 - cos(w0)) / 2
    // b1 = 1 - cos(w0)
    // b2 = (1 - cos(w0)) / 2
    // a0 = 1 + alpha
    // a1 = -2 * cos(w0)
    // a2 = 1 - alpha
    // where w0 = 2*pi*fc/fs, alpha = sin(w0) / (2*Q)

    let fc = 1000.0f64;
    let q_db = 10.0f64;
    let fs = SAMPLE_RATE as f64;

    let q_linear = 10.0f64.powf(q_db / 20.0);
    let w0 = 2.0 * std::f64::consts::PI * fc / fs;
    let alpha = w0.sin() / (2.0 * q_linear);

    let b0 = (1.0 - w0.cos()) / 2.0;
    let b1 = 1.0 - w0.cos();
    let b2 = b0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * w0.cos();
    let a2 = 1.0 - alpha;

    // Normalize
    let b0n = b0 / a0;
    let b1n = b1 / a0;
    let b2n = b2 / a0;
    let a1n = a1 / a0;
    let a2n = a2 / a0;

    // Get our filter's coefficients
    let filter = LowpassFilter::new(SAMPLE_RATE);
    let (fb0, fb1, fb2, fa1, fa2) = filter.coefficients(fc as f32, q_db as f32);

    let eps = 1e-6;
    assert!(
        (fb0 as f64 - b0n).abs() < eps,
        "b0: got {fb0}, expected {b0n}"
    );
    assert!(
        (fb1 as f64 - b1n).abs() < eps,
        "b1: got {fb1}, expected {b1n}"
    );
    assert!(
        (fb2 as f64 - b2n).abs() < eps,
        "b2: got {fb2}, expected {b2n}"
    );
    assert!(
        (fa1 as f64 - a1n).abs() < eps,
        "a1: got {fa1}, expected {a1n}"
    );
    assert!(
        (fa2 as f64 - a2n).abs() < eps,
        "a2: got {fa2}, expected {a2n}"
    );
}
