//! Utility Effects Compliance Tests
//!
//! Tests for Gain and StereoWidth effects, validating:
//! - dB-to-linear precision
//! - Polarity inversion (bit-exact)
//! - Mono summing energy conservation
//! - Stereo width → mono collapse
//! - Mid/Side orthogonality
//!
//! All tests use the public `AudioBackend` trait API only.

use moonlitt_core::AudioBackend;
use moonlitt_effects::{Gain, StereoWidth};
use std::f64::consts::PI;

const SR: u32 = 44100;

// =============================================================================
// Helpers
// =============================================================================

/// Generate a mono 1kHz sine wave at the given amplitude (linear), returned as f32.
fn sine_f32(freq: f64, amplitude: f64, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / SR as f64;
            (amplitude * (2.0 * PI * freq * t).sin()) as f32
        })
        .collect()
}

/// Compute RMS of an f32 buffer.
fn rms_f32(buf: &[f32]) -> f64 {
    let sum: f64 = buf.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / buf.len() as f64).sqrt()
}

/// Convert linear amplitude to dBFS.
fn linear_to_dbfs(linear: f64) -> f64 {
    if linear < 1e-12 {
        -120.0
    } else {
        20.0 * linear.log10()
    }
}

/// Process settling blocks to let parameter smoothers converge.
fn settle(effect: &mut dyn AudioBackend, block_size: usize, blocks: usize) {
    let silent = vec![0.0f32; block_size];
    let mut out_l = vec![0.0f32; block_size];
    let mut out_r = vec![0.0f32; block_size];
    for _ in 0..blocks {
        effect.process_effect(&silent, &silent, &mut out_l, &mut out_r);
    }
}

// =============================================================================
// u1: Gain dB-to-linear precision
// =============================================================================
//
// +6.0206 dB = exactly 2.0× linear gain
// -6.0206 dB = exactly 0.5× linear gain
// 0 dB = 1.0× (unity)
//
// Feed 1kHz sine at amplitude 0.5. Measure output/input amplitude ratio.
// Tolerance: ±0.01 on the ratio.

#[test]
fn u1_gain_db_to_linear_precision() {
    let block = 2048;
    let input = sine_f32(1000.0, 0.5, block);

    let test_cases: &[(f64, f64)] = &[
        (6.0206, 2.0),
        (-6.0206, 0.5),
        (0.0, 1.0),
    ];

    for &(gain_db, expected_ratio) in test_cases {
        let mut gain = Gain::new(SR);
        gain.set_param(0, gain_db);

        // Settle the smoother with silent blocks
        settle(&mut gain, block, 20);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        gain.process_effect(&input, &input, &mut out_l, &mut out_r);

        let input_rms = rms_f32(&input);
        let output_rms = rms_f32(&out_l);
        let ratio = output_rms / input_rms;

        assert!(
            (ratio - expected_ratio).abs() < 0.01,
            "gain_db={gain_db}: expected ratio {expected_ratio}, got {ratio:.6}",
        );
    }
}

// =============================================================================
// u2: Polarity invert — bit-exact
// =============================================================================
//
// polarity=1 at 0dB: output[i] must equal -input[i] at the bit level.
// Exception: if input is ±0.0, both ±0.0 representations are valid zeros.

#[test]
fn u2_polarity_invert_bitexact() {
    let block = 2048;
    let input = sine_f32(1000.0, 0.5, block);

    let mut gain = Gain::new(SR);
    gain.set_param(0, 0.0);   // unity gain
    gain.set_param(1, 1.0);   // polarity invert

    // Settle the smoother
    settle(&mut gain, block, 20);

    let mut out_l = vec![0.0f32; block];
    let mut out_r = vec![0.0f32; block];
    gain.process_effect(&input, &input, &mut out_l, &mut out_r);

    for i in 0..block {
        let expected = -input[i];
        // Handle the ±0.0 case: both are valid representations of zero
        if input[i] == 0.0 {
            assert!(
                out_l[i] == 0.0,
                "sample {i}: input is 0.0, output should be ±0.0, got {}",
                out_l[i]
            );
        } else {
            assert_eq!(
                out_l[i].to_bits(),
                expected.to_bits(),
                "sample {i}: expected bit-exact negation ({expected}), got {}",
                out_l[i]
            );
        }
    }
}

// =============================================================================
// u3: Mono sum energy
// =============================================================================
//
// mono=1 with correlated stereo (L=R=sin): mono output RMS ≈ input RMS ±0.1dB.
// mono=1 with anti-correlated stereo (L=-R): mono output near-silence (< -80 dBFS).

#[test]
fn u3_mono_sum_energy() {
    let block = 4096;
    let signal = sine_f32(1000.0, 0.5, block);
    let neg_signal: Vec<f32> = signal.iter().map(|&s| -s).collect();

    // --- Correlated case: L = R = sin ---
    {
        let mut gain = Gain::new(SR);
        gain.set_param(0, 0.0);  // 0 dB
        gain.set_param(2, 1.0);  // mono on
        settle(&mut gain, block, 20);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        gain.process_effect(&signal, &signal, &mut out_l, &mut out_r);

        let input_rms_db = linear_to_dbfs(rms_f32(&signal));
        let output_rms_db = linear_to_dbfs(rms_f32(&out_l));
        let diff_db = (output_rms_db - input_rms_db).abs();

        assert!(
            diff_db < 0.1,
            "correlated mono: RMS should match ±0.1dB, got {diff_db:.4}dB (in={input_rms_db:.2}, out={output_rms_db:.2})",
        );
    }

    // --- Anti-correlated case: L = sin, R = -sin ---
    {
        let mut gain = Gain::new(SR);
        gain.set_param(0, 0.0);
        gain.set_param(2, 1.0);
        settle(&mut gain, block, 20);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        gain.process_effect(&signal, &neg_signal, &mut out_l, &mut out_r);

        let output_rms_db = linear_to_dbfs(rms_f32(&out_l));
        assert!(
            output_rms_db < -80.0,
            "anti-correlated mono: should be near-silence, got {output_rms_db:.2} dBFS",
        );
    }
}

// =============================================================================
// u4: Stereo width = 0 → mono
// =============================================================================
//
// width=0. Feed different L and R signals (L=440Hz, R=880Hz).
// Output L must equal output R (both are the mid signal = (L+R)/2).

#[test]
fn u4_stereo_width_zero_mono() {
    let block = 2048;
    let in_l = sine_f32(440.0, 0.5, block);
    let in_r = sine_f32(880.0, 0.5, block);

    let mut sw = StereoWidth::new(SR);
    sw.set_param(0, 0.0); // width = 0
    // Leave mid_gain and side_gain at defaults (0 dB)

    // Settle smoothers
    settle(&mut sw, block, 20);

    let mut out_l = vec![0.0f32; block];
    let mut out_r = vec![0.0f32; block];
    sw.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

    for i in 0..block {
        let expected_mid = (in_l[i] as f64 + in_r[i] as f64) * 0.5;
        let diff_lr = (out_l[i] - out_r[i]).abs();
        assert!(
            diff_lr < 1e-6,
            "sample {i}: width=0 but L ({}) != R ({}), diff={diff_lr}",
            out_l[i], out_r[i]
        );
        let diff_mid = (out_l[i] as f64 - expected_mid).abs();
        assert!(
            diff_mid < 1e-5,
            "sample {i}: output ({}) != expected mid ({expected_mid:.8}), diff={diff_mid}",
            out_l[i]
        );
    }
}

// =============================================================================
// u5: Mid/Side orthogonality
// =============================================================================
//
// Feed mono signal (L=R=sin(1kHz)). This has zero side content.
// Boosting side_gain_db=+12 should NOT change the output level,
// because there is no side content to boost.
// Verify output RMS unchanged ±0.1dB.

#[test]
fn u5_mid_side_orthogonality() {
    let block = 4096;
    let mono_signal = sine_f32(1000.0, 0.5, block);

    // --- Reference: side_gain = 0 dB ---
    let reference_rms = {
        let mut sw = StereoWidth::new(SR);
        sw.set_param(0, 1.0); // width = 1
        sw.set_param(1, 0.0); // mid_gain = 0 dB
        sw.set_param(2, 0.0); // side_gain = 0 dB
        settle(&mut sw, block, 20);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        sw.process_effect(&mono_signal, &mono_signal, &mut out_l, &mut out_r);
        rms_f32(&out_l)
    };

    // --- Test: side_gain = +12 dB ---
    let boosted_rms = {
        let mut sw = StereoWidth::new(SR);
        sw.set_param(0, 1.0);   // width = 1
        sw.set_param(1, 0.0);   // mid_gain = 0 dB
        sw.set_param(2, 12.0);  // side_gain = +12 dB
        settle(&mut sw, block, 20);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        sw.process_effect(&mono_signal, &mono_signal, &mut out_l, &mut out_r);
        rms_f32(&out_l)
    };

    let ref_db = linear_to_dbfs(reference_rms);
    let boosted_db = linear_to_dbfs(boosted_rms);
    let diff_db = (boosted_db - ref_db).abs();

    assert!(
        diff_db < 0.1,
        "side boost on mono signal should not change level: ref={ref_db:.2}dB, boosted={boosted_db:.2}dB, diff={diff_db:.4}dB",
    );
}
