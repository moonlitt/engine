//! Compressor Dynamics Compliance Tests
//!
//! References:
//! - Giannoulis et al. "Digital Dynamic Range Compressor Design" (JAES 2012)
//! - AES17 §6.3 (THD+N), §6.6 (Dynamic Range)
//!
//! Zero tolerance: all assertions use machine epsilon.

use moonlitt_effects::{Compressor, EnvelopeFollower};
use moonlitt_core::AudioBackend;
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

// =============================================================================
// C6: Release Timing — envelope follower time constant
// =============================================================================
//
// The EnvelopeFollower uses coeff = exp(-1 / (ms * 0.001 * sr)).
// After N = (ms * 0.001 * sr) samples of feeding 0 from a peak level,
// the envelope level should be exactly peak * e^(-1).
//
// This is mathematically exact because:
//   level[n] = coeff * level[n-1]  (when input < level, i.e., release)
//   level[N] = peak * coeff^N = peak * exp(-1/N)^N = peak * exp(-1)
//
// Tolerance: 0 samples (the sample index at which the level crosses the
// threshold must match the expected time constant exactly).

#[test]
fn c6_release_timing() {
    let release_ms = 10.0;
    let sr_f64 = SR as f64;
    let release_samples = (release_ms * 0.001 * sr_f64) as usize; // 441

    let mut env = EnvelopeFollower::new(sr_f64);
    env.set_attack_ms(0.1); // very fast attack
    env.set_release_ms(release_ms);

    // Phase 1: ramp up to peak level 1.0
    let ramp_samples = (sr_f64 * 0.1) as usize; // 100ms, plenty for attack to converge
    for _ in 0..ramp_samples {
        env.process(1.0);
    }

    let peak = env.level();
    // Verify we are at (very close to) 1.0
    assert!(
        (peak - 1.0).abs() < 1e-10,
        "envelope should have converged to 1.0, got {:.15}",
        peak
    );

    // Phase 2: release — feed 0.0 and find the sample where level crosses
    // peak * e^(-1)
    let target = peak * (-1.0_f64).exp(); // peak * 0.36787944...

    let mut crossing_sample: Option<usize> = None;
    for i in 0..=(release_samples * 3) {
        let level = env.process(0.0);
        // The level decreases monotonically; find first sample <= target
        if level <= target && crossing_sample.is_none() {
            crossing_sample = Some(i + 1); // i is 0-indexed, sample count = i+1
        }
    }

    let crossing = crossing_sample.expect("envelope never crossed target level");

    // The crossing should happen at exactly release_samples (441 for 10ms @ 44100)
    // because after N samples: level = peak * coeff^N = peak * exp(-1)
    assert_eq!(
        crossing, release_samples,
        "release time constant: expected crossing at sample {}, got {} (0 tolerance)",
        release_samples, crossing
    );
}

// =============================================================================
// C7: Ratio Precision — gain computation accuracy
// =============================================================================
//
// With threshold=-20dB, ratio=4.0, knee=0, input at -10dBFS:
//   excess = -10 - (-20) = 10 dB
//   output_db = -20 + 10/4 = -17.5 dBFS
//   gain_reduction = 7.5 dB
//   gain_db = output_db - input_db = -17.5 - (-10) = -7.5 dB
//
// Test the static gain computation function directly: relative error < f64::EPSILON.
// Then verify through full pipeline with settled envelope.

#[test]
fn c7_ratio_precision_static() {
    // Test the pure gain computation (no envelope dynamics)
    let mut comp = Compressor::new(SR);
    comp.set_param(0, -20.0); // threshold
    comp.set_param(1, 4.0); // ratio
    comp.set_param(4, 0.0); // knee = 0 (hard knee)

    let input_db = -10.0;
    let gain_db = comp.compute_gain_db(input_db);

    // Expected: output = threshold + (input - threshold) / ratio
    //         = -20 + (-10 - (-20)) / 4 = -20 + 2.5 = -17.5
    // gain = output - input = -17.5 - (-10) = -7.5
    let expected_gain_db = -7.5;

    let abs_error = (gain_db - expected_gain_db).abs();
    // For f64 arithmetic: threshold + (input - threshold) / ratio - input
    // = -20.0 + 10.0 / 4.0 - (-10.0) = -20.0 + 2.5 + 10.0 = -7.5
    // All values are exact in f64 binary representation, so error should be 0.
    assert!(
        abs_error <= f64::EPSILON,
        "C7: gain_db = {:.18}, expected = {:.18}, abs_error = {:.2e} > EPSILON ({:.2e})",
        gain_db,
        expected_gain_db,
        abs_error,
        f64::EPSILON
    );
}

#[test]
fn c7_ratio_precision_pipeline() {
    // Verify through full process_effect pipeline using DC input.
    // DC eliminates all RMS measurement artifacts and envelope oscillation —
    // the detected level is constant, so the envelope converges instantly.
    let mut comp = Compressor::new(SR);
    comp.set_param(0, -20.0); // threshold = -20 dB
    comp.set_param(1, 4.0); // ratio = 4:1
    comp.set_param(2, 0.1); // attack = 0.1ms (very fast)
    comp.set_param(3, 1000.0); // release = 1s (slow, holds)
    comp.set_param(4, 0.0); // knee = 0
    comp.set_param(5, 0.0); // makeup = 0
    comp.set_param(6, 0.0); // HPF bypassed (identity filter)

    // DC input at -10 dBFS
    let amplitude = 10.0_f64.powf(-10.0 / 20.0); // ≈ 0.316228
    let settle_time = SR as usize; // 1 second is more than enough for DC
    let input: Vec<f32> = vec![amplitude as f32; settle_time];
    let silent = vec![0.0f32; settle_time];
    let mut out_l = vec![0.0f32; settle_time];
    let mut out_r = vec![0.0f32; settle_time];

    comp.process_effect(&input, &silent, &mut out_l, &mut out_r);

    // With DC input and bypassed HPF, the detected level is constant at the
    // input amplitude. The envelope converges within a few attack time constants
    // (0.1ms attack → ~5 samples). After settling, every output sample should
    // be input * 10^(gain_db/20).
    //
    // Expected: gain_db = -7.5 dB → linear gain = 10^(-7.5/20) ≈ 0.42170
    // Output = amplitude * linear_gain
    let expected_gain_db = -7.5;
    let expected_linear_gain = 10.0_f64.powf(expected_gain_db / 20.0);
    let expected_output = (amplitude * expected_linear_gain) as f32;

    // Check the last sample (fully settled).
    // Compare in linear domain — f32::EPSILON is the correct bound here.
    // (dB comparison involves log10 which introduces additional rounding,
    // so we verify in the native domain of the audio path.)
    let last_output = out_l[settle_time - 1];

    let rel_error = ((last_output - expected_output) as f64 / expected_output as f64).abs();
    assert!(
        rel_error <= f32::EPSILON as f64,
        "C7 pipeline: output {:.10} vs expected {:.10}, rel error {:.2e} > f32::EPSILON ({:.2e})",
        last_output,
        expected_output,
        rel_error,
        f32::EPSILON
    );

    // Log the dB-domain error for diagnostics (not the assertion target)
    let actual_gain_linear = last_output as f64 / amplitude;
    let actual_gain_db = 20.0 * actual_gain_linear.log10();
    let error_db = (actual_gain_db - expected_gain_db).abs();
    eprintln!(
        "C7 pipeline: gain = {:.8} dB (error {:.2e} dB), linear rel_error = {:.2e}",
        actual_gain_db, error_db, rel_error
    );
}

// =============================================================================
// C8: Makeup Gain — below-threshold signal with makeup applied
// =============================================================================
//
// When the input is below threshold, gain_reduction = 0.
// With makeup = +6 dB, the output should be input * 10^(6/20).
// Tolerance: relative error < f32::EPSILON (output is f32).

#[test]
fn c8_makeup_gain() {
    let mut comp = Compressor::new(SR);
    comp.set_param(0, 0.0); // threshold = 0 dB (max, nothing triggers)
    comp.set_param(1, 4.0); // ratio = 4:1 (doesn't matter, below threshold)
    comp.set_param(2, 0.1); // attack = very fast
    comp.set_param(3, 10.0); // release = fast
    comp.set_param(4, 0.0); // knee = 0
    comp.set_param(5, 6.0); // makeup = +6 dB
    comp.set_param(6, 20.0); // HPF at 20Hz

    let makeup_linear = 10.0_f64.powf(6.0 / 20.0);

    // Input at -20 dB (well below threshold of 0 dB)
    let amplitude = 10.0_f64.powf(-20.0 / 20.0); // 0.1
    let num_samples = SR as usize * 2; // 2 seconds
    let input = sine_f32(1000.0, amplitude, num_samples);
    let silent = vec![0.0f32; num_samples];
    let mut out_l = vec![0.0f32; num_samples];
    let mut out_r = vec![0.0f32; num_samples];

    comp.process_effect(&input, &silent, &mut out_l, &mut out_r);

    // After the sidechain HPF and envelope settle, check each sample.
    // Skip the first 0.5s to let transients settle.
    let check_start = SR as usize / 2;

    for i in check_start..num_samples {
        let in_val = input[i] as f64;
        let expected = (in_val * makeup_linear) as f32;
        let actual = out_l[i];

        if expected.abs() < 1e-10 {
            // Near zero crossings, absolute comparison
            assert!(
                (actual - expected).abs() <= f32::EPSILON,
                "C8 sample {}: expected {:.10}, got {:.10}",
                i,
                expected,
                actual
            );
        } else {
            let rel_error = ((actual - expected) / expected).abs();
            assert!(
                rel_error <= f32::EPSILON,
                "C8 sample {}: expected {:.10}, got {:.10}, relative error {:.2e} > f32::EPSILON ({:.2e})",
                i,
                expected,
                actual,
                rel_error,
                f32::EPSILON
            );
        }
    }
}
