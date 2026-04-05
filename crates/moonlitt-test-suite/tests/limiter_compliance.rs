//! Limiter DSP compliance tests.
//!
//! Verifies timing precision, true peak detection, ceiling enforcement,
//! lookahead latency, and oversampling alias rejection.

use moonlitt_core::AudioBackend;
use moonlitt_effects::Limiter;
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

/// RMS of a buffer (linear).
#[allow(dead_code)]
fn rms(buf: &[f32]) -> f64 {
    let sum_sq: f64 = buf.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / buf.len() as f64).sqrt()
}

/// RMS in dBFS.
#[allow(dead_code)]
fn rms_dbfs(buf: &[f32]) -> f64 {
    let r = rms(buf);
    if r < 1e-12 {
        -120.0
    } else {
        20.0 * r.log10()
    }
}

// =============================================================================
// L1: True peak never exceeds ceiling
// =============================================================================
//
// Set threshold=-6, ceiling=-1, oversampling=2x. Feed loud signal (amplitude
// 2.0, 440Hz sine). Process 10 settling blocks + 1 measurement block (4410
// samples each). Every output sample must have abs <= ceiling_linear + 0.01.

#[test]
fn l1_true_peak_never_exceeds_ceiling() {
    let mut lim = Limiter::new(SR);
    lim.set_param(0, -6.0); // threshold = -6 dB
    lim.set_param(1, -1.0); // ceiling = -1 dB
    lim.set_param(5, 2.0); // oversampling = 2x

    let ceiling_linear = 10.0_f64.powf(-1.0 / 20.0); // ~0.891

    let block_size = 4410;
    let input = sine_f32(440.0, 2.0, block_size);
    let mut out_l = vec![0.0f32; block_size];
    let mut out_r = vec![0.0f32; block_size];

    // 10 settling blocks
    for _ in 0..10 {
        lim.process_effect(&input, &input, &mut out_l, &mut out_r);
    }

    // 1 measurement block
    lim.process_effect(&input, &input, &mut out_l, &mut out_r);

    let max_peak = out_l
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    let tolerance = 0.01;
    assert!(
        max_peak <= ceiling_linear as f32 + tolerance,
        "L1: max peak {:.6} exceeds ceiling_linear {:.6} + tolerance {:.3}",
        max_peak,
        ceiling_linear,
        tolerance
    );
}

// =============================================================================
// L2: Attack timing precision
// =============================================================================
//
// The limiter's envelope follower smooths gain_reduction in the **dB domain**
// (gr_magnitude). The time constant of the underlying exponential filter
// governs how fast gr_magnitude rises from 0 toward the target GR.
//
// With a constant input above threshold, the envelope approaches its target
// exponentially: after exactly `attack_ms` milliseconds, the smoothed GR
// should reach 63.2% of the final (steady-state) GR.
//
// Since the envelope starts processing at sample 0 of the loud block (on the
// un-delayed input), the 63.2% crossing occurs at sample `attack_samples`
// from the block start. In the output, samples 0..latency are zeros from the
// lookahead buffer. We measure gain_reduction_db = -20*log10(output/input)
// on the non-zero output samples and verify the time constant.

#[test]
fn l2_attack_timing_precision() {
    let attack_ms = 5.0;
    let mut lim = Limiter::new(SR);
    lim.set_param(0, -20.0); // threshold = -20 dB
    lim.set_param(4, attack_ms); // attack = 5.0 ms
    lim.set_param(6, 0.0); // auto_release = off
    // Use slow release so it doesn't interfere with attack measurement
    lim.set_param(2, 1000.0); // release = 1000 ms

    let block_size = 4410;
    let latency = lim.latency() as usize;
    let attack_samples = (attack_ms * SR as f64 / 1000.0) as usize;

    // Constant DC signal at +6 dBFS (well above threshold of -20 dB)
    let loud_amplitude = 10.0_f64.powf(6.0 / 20.0); // ~1.995
    let loud_signal: Vec<f32> = vec![loud_amplitude as f32; block_size];
    let silence = vec![0.0f32; block_size];
    let mut out_l = vec![0.0f32; block_size];
    let mut out_r = vec![0.0f32; block_size];

    // Feed silence first to ensure limiter is at unity
    lim.process_effect(&silence, &silence, &mut out_l, &mut out_r);

    // Feed loud signal
    lim.process_effect(&loud_signal, &loud_signal, &mut out_l, &mut out_r);

    // Compute gain reduction in dB at each output sample.
    // GR_db = -20*log10(output / input). For zero output (latency region), skip.
    let final_gr_db = -20.0 * (out_l[block_size - 1] as f64 / loud_amplitude).log10();
    let target_gr_db = 0.632 * final_gr_db;

    // Find the sample where GR first reaches the target.
    // The envelope started processing at sample 0, so crossing should be
    // near sample `attack_samples` (measured from block start).
    let mut crossing_sample: Option<usize> = None;
    for i in latency..block_size {
        let out_val = out_l[i] as f64;
        if out_val.abs() < 1e-12 {
            continue;
        }
        let gr_db = -20.0 * (out_val / loud_amplitude).log10();
        if gr_db >= target_gr_db {
            crossing_sample = Some(i);
            break;
        }
    }

    let crossing = crossing_sample.expect(
        "L2: gain reduction never reached 63.2% of final value",
    );

    // Allow 30% tolerance or minimum 20 samples to account for parameter
    // smoother convergence and nonlinearities in the gain computation path.
    let tolerance = (attack_samples as f64 * 0.30).max(20.0) as usize;
    let diff = if crossing > attack_samples {
        crossing - attack_samples
    } else {
        attack_samples - crossing
    };

    eprintln!(
        "L2: attack_samples={}, crossing={}, final_gr_db={:.2}, target_gr_db={:.2}",
        attack_samples, crossing, final_gr_db, target_gr_db
    );

    assert!(
        diff <= tolerance,
        "L2: attack time constant: expected ~{} samples, got {} (diff {}, tolerance {})",
        attack_samples,
        crossing,
        diff,
        tolerance
    );
}

// =============================================================================
// L3: Release timing precision
// =============================================================================
//
// The envelope follower smooths gain reduction in the dB domain. During
// release, the smoothed GR decays exponentially from its peak toward 0 dB.
// After `release_ms` milliseconds, the GR should have decayed to 36.8%
// (= e^-1) of its peak value.
//
// We use release_ms=200 (8820 samples) which is much longer than the
// lookahead (44 samples). We measure GR from the output after the loud
// content has cleared the delay line.

#[test]
fn l3_release_timing_precision() {
    let release_ms = 200.0;
    let mut lim = Limiter::new(SR);
    lim.set_param(0, -20.0); // threshold = -20 dB
    lim.set_param(2, release_ms); // release = 200 ms
    lim.set_param(6, 0.0); // auto_release = off

    let block_size = 44100; // 1 second, enough for 200ms release
    let latency = lim.latency() as usize;
    let release_samples = (release_ms * SR as f64 / 1000.0) as usize;

    // +6 dBFS loud constant DC signal
    let loud_amplitude = 10.0_f64.powf(6.0 / 20.0);
    let loud_signal: Vec<f32> = vec![loud_amplitude as f32; block_size];
    let mut out_l = vec![0.0f32; block_size];
    let mut out_r = vec![0.0f32; block_size];

    // Feed loud signal for 10 blocks so the limiter fully engages
    for _ in 0..10 {
        lim.process_effect(&loud_signal, &loud_signal, &mut out_l, &mut out_r);
    }

    // Measure the fully-engaged GR in dB (last sample of last loud block)
    let engaged_gr_db = -20.0 * (out_l[block_size - 1] as f64 / loud_amplitude).log10();

    // Feed a quiet constant signal (below threshold, so GR target = 0)
    let quiet_amplitude = 10.0_f64.powf(-40.0 / 20.0); // -40 dBFS
    let quiet_signal: Vec<f32> = vec![quiet_amplitude as f32; block_size];

    lim.process_effect(&quiet_signal, &quiet_signal, &mut out_l, &mut out_r);

    // After `latency` samples, the delayed signal is quiet, so:
    //   output = quiet_amplitude * gain_linear
    //   gain_linear = 10^(-smoothed_gr / 20)
    //   GR_db = -20 * log10(output / quiet_amplitude)
    //
    // The envelope releases exponentially: GR(t) ≈ engaged_gr * e^(-t/tau).
    // After tau = release_samples, GR should be engaged_gr * e^(-1) ≈ 36.8%.
    let target_gr_db = engaged_gr_db * (-1.0_f64).exp(); // 36.8% of peak GR

    let mut crossing_sample: Option<usize> = None;
    for i in latency..block_size {
        let out_val = out_l[i] as f64;
        if out_val.abs() < 1e-15 {
            continue;
        }
        let gr_db = -20.0 * (out_val / quiet_amplitude).log10();
        // GR is decaying; find when it first drops below the target
        if gr_db <= target_gr_db {
            crossing_sample = Some(i);
            break;
        }
    }

    let crossing = crossing_sample.expect(
        "L3: gain reduction never decayed to 36.8% of engaged value",
    );

    // Allow 25% tolerance, minimum 50 samples
    let tolerance = (release_samples as f64 * 0.25).max(50.0) as usize;
    let diff = if crossing > release_samples {
        crossing - release_samples
    } else {
        release_samples - crossing
    };

    eprintln!(
        "L3: release_samples={}, crossing={}, engaged_gr_db={:.2}, target_gr_db={:.2}",
        release_samples, crossing, engaged_gr_db, target_gr_db
    );

    assert!(
        diff <= tolerance,
        "L3: release time constant: expected ~{} samples, got {} (diff {}, tolerance {})",
        release_samples,
        crossing,
        diff,
        tolerance
    );
}

// =============================================================================
// L4: Auto-release adapts to signal content
// =============================================================================
//
// auto_release=1. Process two different signals:
// - Signal A: sparse impulses (transient) — should release fast
// - Signal B: continuous loud sine (sustained) — should release slower
// Verify release_A < release_B.

#[test]
fn l4_auto_release_adapts() {
    let block_size = 4410;

    // --- Helper: measure effective release time ---
    // Feed loud signal blocks, then quiet signal, find 63.2% recovery sample.
    fn measure_release(lim: &mut Limiter, loud_blocks: &[Vec<f32>], block_size: usize) -> usize {
        let mut out_l = vec![0.0f32; block_size];
        let mut out_r = vec![0.0f32; block_size];

        // Feed loud blocks
        for block in loud_blocks {
            lim.process_effect(block, block, &mut out_l, &mut out_r);
        }

        // Record the gain at end of loud processing
        // Use the last block's input to compute gain
        let last_input = loud_blocks.last().unwrap();
        let last_input_rms = {
            let sum: f64 = last_input.iter().map(|&s| (s as f64).powi(2)).sum();
            (sum / last_input.len() as f64).sqrt()
        };
        let last_output_rms = {
            let sum: f64 = out_l.iter().map(|&s| (s as f64).powi(2)).sum();
            (sum / out_l.len() as f64).sqrt()
        };
        let compressed_gain = last_output_rms / (last_input_rms + 1e-12);

        // Feed quiet signal and measure recovery
        let quiet_amplitude = 10.0_f64.powf(-40.0 / 20.0);
        let quiet: Vec<f32> = vec![quiet_amplitude as f32; block_size];
        lim.process_effect(&quiet, &quiet, &mut out_l, &mut out_r);

        let target_gain = compressed_gain + 0.632 * (1.0 - compressed_gain);

        for i in 0..block_size {
            let gain = out_l[i] as f64 / quiet_amplitude;
            if gain >= target_gain {
                return i;
            }
        }

        block_size // did not recover
    }

    // Signal A: sparse impulses (transient-like)
    let mut signal_a_blocks: Vec<Vec<f32>> = Vec::new();
    for _ in 0..10 {
        let mut block = vec![0.0f32; block_size];
        // Single impulse at the start of each block
        block[0] = 1.0;
        signal_a_blocks.push(block);
    }

    // Signal B: continuous loud sine (sustained)
    let mut signal_b_blocks: Vec<Vec<f32>> = Vec::new();
    for block_idx in 0..10u64 {
        let offset = block_idx as usize * block_size;
        let block: Vec<f32> = (0..block_size)
            .map(|i| {
                let t = (offset + i) as f64 / SR as f64;
                (1.0 * (2.0 * PI * 440.0 * t).sin()) as f32
            })
            .collect();
        signal_b_blocks.push(block);
    }

    // Limiter A: auto_release on, transient signal
    let mut lim_a = Limiter::new(SR);
    lim_a.set_param(0, -10.0); // threshold
    lim_a.set_param(1, -1.0); // ceiling
    lim_a.set_param(6, 1.0); // auto_release = on

    // Limiter B: auto_release on, sustained signal
    let mut lim_b = Limiter::new(SR);
    lim_b.set_param(0, -10.0);
    lim_b.set_param(1, -1.0);
    lim_b.set_param(6, 1.0);

    let release_a = measure_release(&mut lim_a, &signal_a_blocks, block_size);
    let release_b = measure_release(&mut lim_b, &signal_b_blocks, block_size);

    eprintln!(
        "L4: release_A (transient) = {} samples, release_B (sustained) = {} samples",
        release_a, release_b
    );

    assert!(
        release_a < release_b,
        "L4: auto-release should adapt — transient release ({}) should be shorter than sustained ({})",
        release_a,
        release_b
    );
}

// =============================================================================
// L5: Lookahead latency matches reported value
// =============================================================================
//
// Default params (lookahead=1.0ms). Feed impulse (1.0 at sample 0, rest
// zeros). Find the peak output sample index. It should equal limiter.latency()
// +/- 2 samples.

#[test]
fn l5_lookahead_latency_matches() {
    let mut lim = Limiter::new(SR);
    // Use defaults (lookahead = 1.0ms, oversampling = 1x)

    let block_size = 4410;
    let reported_latency = lim.latency() as usize;

    // Create impulse: 1.0 at sample 0, rest zeros
    let mut impulse = vec![0.0f32; block_size];
    impulse[0] = 1.0;
    let silence = vec![0.0f32; block_size];
    let mut out_l = vec![0.0f32; block_size];
    let mut out_r = vec![0.0f32; block_size];

    lim.process_effect(&impulse, &silence, &mut out_l, &mut out_r);

    // Find peak sample index
    let (peak_idx, _peak_val) = out_l
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
        .unwrap();

    let diff = if peak_idx > reported_latency {
        peak_idx - reported_latency
    } else {
        reported_latency - peak_idx
    };

    assert!(
        diff <= 2,
        "L5: peak at sample {}, reported latency {}, diff {} > 2",
        peak_idx,
        reported_latency,
        diff
    );
}

// =============================================================================
// L6: Ceiling hard clip — extreme signal
// =============================================================================
//
// threshold=-10, ceiling=-3. Feed extreme signal (amplitude 10.0, 440Hz).
// Process 20 blocks. In the last block, EVERY sample must have
// abs <= ceiling_linear. No exceptions.

#[test]
fn l6_ceiling_hard_clip() {
    let mut lim = Limiter::new(SR);
    lim.set_param(0, -10.0); // threshold = -10 dB
    lim.set_param(1, -3.0); // ceiling = -3 dB

    let ceiling_linear = 10.0_f64.powf(-3.0 / 20.0); // ~0.7079

    let block_size = 4410;
    let input = sine_f32(440.0, 10.0, block_size);
    let mut out_l = vec![0.0f32; block_size];
    let mut out_r = vec![0.0f32; block_size];

    // Process 20 blocks
    for _ in 0..20 {
        lim.process_effect(&input, &input, &mut out_l, &mut out_r);
    }

    // Check every sample of the last block
    for (i, &sample) in out_l.iter().enumerate() {
        assert!(
            sample.abs() <= ceiling_linear as f32 + f32::EPSILON,
            "L6: sample {} has abs value {:.8} which exceeds ceiling_linear {:.8}",
            i,
            sample.abs(),
            ceiling_linear
        );
    }

    // Also check right channel
    for (i, &sample) in out_r.iter().enumerate() {
        assert!(
            sample.abs() <= ceiling_linear as f32 + f32::EPSILON,
            "L6: right channel sample {} has abs value {:.8} which exceeds ceiling_linear {:.8}",
            i,
            sample.abs(),
            ceiling_linear
        );
    }
}

// =============================================================================
// L7: Oversampling reduces inter-sample overshoot
// =============================================================================
//
// Create a signal known to have inter-sample peaks: alternating [0.8, -0.8].
// Process with 1x and 2x oversampling separately (threshold=-3, ceiling=-1).
// The 2x version should have lower peak output than 1x, because it catches
// the inter-sample peaks.

#[test]
fn l7_oversampling_reduces_intersample_overshoot() {
    let block_size = 4410;

    // Alternating samples create inter-sample peaks
    let inter_sample_signal: Vec<f32> = (0..block_size)
        .map(|i| if i % 2 == 0 { 0.8 } else { -0.8 })
        .collect();

    // --- 1x oversampling ---
    let mut lim_1x = Limiter::new(SR);
    lim_1x.set_param(0, -3.0); // threshold = -3 dB
    lim_1x.set_param(1, -1.0); // ceiling = -1 dB
    lim_1x.set_param(5, 1.0); // oversampling = 1x

    let mut out_1x_l = vec![0.0f32; block_size];
    let mut out_1x_r = vec![0.0f32; block_size];

    // Settle
    for _ in 0..10 {
        lim_1x.process_effect(
            &inter_sample_signal,
            &inter_sample_signal,
            &mut out_1x_l,
            &mut out_1x_r,
        );
    }
    // Measurement block
    lim_1x.process_effect(
        &inter_sample_signal,
        &inter_sample_signal,
        &mut out_1x_l,
        &mut out_1x_r,
    );

    let peak_1x = out_1x_l
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    // --- 2x oversampling ---
    let mut lim_2x = Limiter::new(SR);
    lim_2x.set_param(0, -3.0);
    lim_2x.set_param(1, -1.0);
    lim_2x.set_param(5, 2.0); // oversampling = 2x

    let mut out_2x_l = vec![0.0f32; block_size];
    let mut out_2x_r = vec![0.0f32; block_size];

    // Settle
    for _ in 0..10 {
        lim_2x.process_effect(
            &inter_sample_signal,
            &inter_sample_signal,
            &mut out_2x_l,
            &mut out_2x_r,
        );
    }
    // Measurement block
    lim_2x.process_effect(
        &inter_sample_signal,
        &inter_sample_signal,
        &mut out_2x_l,
        &mut out_2x_r,
    );

    let peak_2x = out_2x_l
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    eprintln!(
        "L7: peak_1x = {:.6}, peak_2x = {:.6}",
        peak_1x, peak_2x
    );

    assert!(
        peak_2x <= peak_1x,
        "L7: 2x oversampled peak ({:.6}) should be <= 1x peak ({:.6})",
        peak_2x,
        peak_1x
    );
}

// =============================================================================
// L8: Bypass is bit-exact
// =============================================================================
//
// bypass=1. Feed any signal. Output must equal input bit-exact (compare with ==).

#[test]
fn l8_bypass_thd() {
    let mut lim = Limiter::new(SR);
    lim.set_param(7, 1.0); // bypass = on

    let block_size = 4410;
    let input = sine_f32(1000.0, 0.9, block_size);
    let input_r = sine_f32(1500.0, 0.7, block_size);
    let mut out_l = vec![0.0f32; block_size];
    let mut out_r = vec![0.0f32; block_size];

    lim.process_effect(&input, &input_r, &mut out_l, &mut out_r);

    // Bit-exact comparison
    assert_eq!(
        input, out_l,
        "L8: left channel bypass output is not bit-exact"
    );
    assert_eq!(
        input_r, out_r,
        "L8: right channel bypass output is not bit-exact"
    );
}
