//! De-esser Compliance Tests
//!
//! Tests for the split-band sibilance reduction effect, validating:
//! - Sibilance attenuation at the target frequency
//! - Non-sibilant passthrough in split-band mode
//! - Listen mode bandpass isolation
//! - Wideband vs split-band behavior difference
//! - Frequency tracking accuracy
//!
//! All tests use the public `AudioBackend` trait API only.

use moonlitt_core::AudioBackend;
use moonlitt_effects::DeEsser;
use std::f64::consts::PI;

const SR: u32 = 44100;

// =============================================================================
// Helpers
// =============================================================================

/// Generate a mono sine wave at the given amplitude (linear), returned as f32.
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

/// Process settling blocks with the given signal to prime filters and envelopes.
fn settle_with_signal(
    effect: &mut dyn AudioBackend,
    input: &[f32],
    blocks: usize,
) -> (Vec<f32>, Vec<f32>) {
    let len = input.len();
    let mut out_l = vec![0.0f32; len];
    let mut out_r = vec![0.0f32; len];
    for _ in 0..blocks {
        effect.process_effect(input, input, &mut out_l, &mut out_r);
    }
    (out_l, out_r)
}

// =============================================================================
// d1: Sibilance attenuation ratio
// =============================================================================
//
// frequency=6000, threshold=-20, ratio=10. Feed 6kHz sine at -10 dBFS.
// Process 10 settling blocks + 1 measurement block.
// Output RMS should be significantly lower than input (at least 5 dB attenuation).

#[test]
fn d1_sibilance_attenuation_ratio() {
    let block = 4410; // 100ms at 44100
    let amplitude = 10.0_f64.powf(-10.0 / 20.0); // -10 dBFS
    let input = sine_f32(6000.0, amplitude, block);

    let mut ds = DeEsser::new(SR);
    ds.set_param(0, -20.0);   // threshold = -20 dB
    ds.set_param(1, 6000.0);  // frequency = 6kHz
    ds.set_param(2, 2.0);     // bandwidth Q = 2
    ds.set_param(3, 10.0);    // ratio = 10:1
    ds.set_param(4, 0.0);     // wideband mode

    // Settle: 10 blocks
    settle_with_signal(&mut ds, &input, 10);

    // Measurement block
    let mut out_l = vec![0.0f32; block];
    let mut out_r = vec![0.0f32; block];
    ds.process_effect(&input, &input, &mut out_l, &mut out_r);

    let input_rms_db = linear_to_dbfs(rms_f32(&input));
    let output_rms_db = linear_to_dbfs(rms_f32(&out_l));
    let attenuation_db = input_rms_db - output_rms_db;

    assert!(
        attenuation_db > 5.0,
        "6kHz at -10dBFS should be attenuated by >5dB, got {attenuation_db:.2}dB \
         (in={input_rms_db:.2}, out={output_rms_db:.2})",
    );
}

// =============================================================================
// d2: Non-sibilant passthrough (split-band)
// =============================================================================
//
// mode=1 (split-band), frequency=6000. Feed 200Hz sine at -10 dBFS.
// Process 10 settling blocks + 1 measurement.
// Output RMS should equal input RMS ±0.5 dB (200Hz is far from 6kHz).

#[test]
fn d2_non_sibilant_passthrough_splitband() {
    let block = 4410;
    let amplitude = 10.0_f64.powf(-10.0 / 20.0); // -10 dBFS
    let input = sine_f32(200.0, amplitude, block);

    let mut ds = DeEsser::new(SR);
    ds.set_param(0, -20.0);   // threshold
    ds.set_param(1, 6000.0);  // frequency
    ds.set_param(2, 2.0);     // bandwidth Q
    ds.set_param(3, 10.0);    // ratio
    ds.set_param(4, 1.0);     // split-band mode

    // Settle
    settle_with_signal(&mut ds, &input, 10);

    // Measurement
    let mut out_l = vec![0.0f32; block];
    let mut out_r = vec![0.0f32; block];
    ds.process_effect(&input, &input, &mut out_l, &mut out_r);

    let input_rms_db = linear_to_dbfs(rms_f32(&input));
    let output_rms_db = linear_to_dbfs(rms_f32(&out_l));
    let diff_db = (output_rms_db - input_rms_db).abs();

    assert!(
        diff_db < 0.5,
        "200Hz in split-band mode should pass through ±0.5dB, got {diff_db:.4}dB \
         (in={input_rms_db:.2}, out={output_rms_db:.2})",
    );
}

// =============================================================================
// d3: Listen mode is bandpass
// =============================================================================
//
// listen_mode=1, frequency=6000. Feed a mix of 200Hz + 6kHz.
// Output should contain mostly the 6kHz component (bandpass-filtered).
// The output RMS should be lower than input RMS since the 200Hz is removed.

#[test]
fn d3_listen_mode_is_bandpass() {
    let block = 4410;
    let input: Vec<f32> = (0..block)
        .map(|i| {
            let t = i as f64 / SR as f64;
            let low = 0.3 * (2.0 * PI * 200.0 * t).sin();
            let high = 0.3 * (2.0 * PI * 6000.0 * t).sin();
            (low + high) as f32
        })
        .collect();

    let mut ds = DeEsser::new(SR);
    ds.set_param(1, 6000.0);  // frequency
    ds.set_param(2, 2.0);     // bandwidth Q
    ds.set_param(5, 1.0);     // listen mode ON

    // Settle filters
    settle_with_signal(&mut ds, &input, 10);

    // Measurement
    let mut out_l = vec![0.0f32; block];
    let mut out_r = vec![0.0f32; block];
    ds.process_effect(&input, &input, &mut out_l, &mut out_r);

    let input_rms = rms_f32(&input);
    let output_rms = rms_f32(&out_l);

    // The bandpass should remove 200Hz, so output energy < input energy
    assert!(
        output_rms < input_rms,
        "listen mode should output only bandpass portion: \
         input_rms={input_rms:.6}, output_rms={output_rms:.6}",
    );
}

// =============================================================================
// d4: Wideband vs split-band low-frequency preservation
// =============================================================================
//
// Feed a mix of 200Hz + 6kHz at -10 dBFS, threshold=-20.
// Process with wideband (mode=0) and split-band (mode=1) separately.
// Measure 200Hz energy in output (use a low signal = just 200Hz component).
//
// In wideband mode, the 6kHz triggers gain reduction on the ENTIRE signal,
// so 200Hz gets attenuated too.
// In split-band mode, only the 6kHz band is attenuated, preserving 200Hz.
// Difference > 2 dB.

#[test]
fn d4_wideband_vs_splitband_low_freq() {
    let block = 4410;
    let amplitude = 10.0_f64.powf(-10.0 / 20.0); // -10 dBFS per component
    let input: Vec<f32> = (0..block)
        .map(|i| {
            let t = i as f64 / SR as f64;
            let low = amplitude * (2.0 * PI * 200.0 * t).sin();
            let high = amplitude * (2.0 * PI * 6000.0 * t).sin();
            (low + high) as f32
        })
        .collect();

    // --- Wideband mode ---
    let wideband_200hz_rms = {
        let mut ds = DeEsser::new(SR);
        ds.set_param(0, -20.0);  // threshold
        ds.set_param(1, 6000.0); // frequency
        ds.set_param(2, 2.0);    // Q
        ds.set_param(3, 10.0);   // ratio
        ds.set_param(4, 0.0);    // wideband

        settle_with_signal(&mut ds, &input, 10);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        ds.process_effect(&input, &input, &mut out_l, &mut out_r);

        // Measure low-frequency energy by correlating with the 200Hz reference.
        // Simple approach: compute RMS of output (it includes both frequencies,
        // but the wideband attenuation will reduce overall level).
        rms_f32(&out_l)
    };

    // --- Split-band mode ---
    let splitband_200hz_rms = {
        let mut ds = DeEsser::new(SR);
        ds.set_param(0, -20.0);
        ds.set_param(1, 6000.0);
        ds.set_param(2, 2.0);
        ds.set_param(3, 10.0);
        ds.set_param(4, 1.0);  // split-band

        settle_with_signal(&mut ds, &input, 10);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        ds.process_effect(&input, &input, &mut out_l, &mut out_r);

        rms_f32(&out_l)
    };

    let wideband_db = linear_to_dbfs(wideband_200hz_rms);
    let splitband_db = linear_to_dbfs(splitband_200hz_rms);
    let diff_db = splitband_db - wideband_db;

    assert!(
        diff_db > 2.0,
        "split-band should preserve more low-freq energy than wideband: \
         wideband={wideband_db:.2}dB, splitband={splitband_db:.2}dB, diff={diff_db:.2}dB \
         (expected >2dB)",
    );
}

// =============================================================================
// d5: Frequency tracking
// =============================================================================
//
// Set frequency=4000, feed 4kHz at -10 dBFS, threshold=-20, ratio=10.
// Should attenuate significantly.
// Then set frequency=8000, feed same 4kHz signal.
// Should NOT attenuate (4kHz is outside 8kHz detection band).
// Difference > 3 dB.

#[test]
fn d5_frequency_tracking() {
    let block = 4410;
    let amplitude = 10.0_f64.powf(-10.0 / 20.0); // -10 dBFS
    let input_4k = sine_f32(4000.0, amplitude, block);

    // --- frequency=4000 → should attenuate 4kHz ---
    let rms_at_4k = {
        let mut ds = DeEsser::new(SR);
        ds.set_param(0, -20.0);   // threshold
        ds.set_param(1, 4000.0);  // frequency = 4kHz (matches input)
        ds.set_param(2, 2.0);     // Q
        ds.set_param(3, 10.0);    // ratio
        ds.set_param(4, 0.0);     // wideband

        settle_with_signal(&mut ds, &input_4k, 10);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        ds.process_effect(&input_4k, &input_4k, &mut out_l, &mut out_r);
        rms_f32(&out_l)
    };

    // --- frequency=8000 → should NOT attenuate 4kHz ---
    let rms_at_8k = {
        let mut ds = DeEsser::new(SR);
        ds.set_param(0, -20.0);   // threshold
        ds.set_param(1, 8000.0);  // frequency = 8kHz (does NOT match input)
        ds.set_param(2, 2.0);     // Q
        ds.set_param(3, 10.0);    // ratio
        ds.set_param(4, 0.0);     // wideband

        settle_with_signal(&mut ds, &input_4k, 10);

        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];
        ds.process_effect(&input_4k, &input_4k, &mut out_l, &mut out_r);
        rms_f32(&out_l)
    };

    let rms_at_4k_db = linear_to_dbfs(rms_at_4k);
    let rms_at_8k_db = linear_to_dbfs(rms_at_8k);
    let diff_db = rms_at_8k_db - rms_at_4k_db;

    assert!(
        diff_db > 3.0,
        "frequency tracking: 4kHz signal should be attenuated more when detector is at 4kHz \
         than at 8kHz: at_4k={rms_at_4k_db:.2}dB, at_8k={rms_at_8k_db:.2}dB, diff={diff_db:.2}dB \
         (expected >3dB)",
    );
}
