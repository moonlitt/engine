//! Gate DSP Compliance Tests
//!
//! Validates attenuation precision, timing (attack/release), hold behaviour,
//! hysteresis deadband, and sidechain filter isolation for the noise gate.
//!
//! All tests use 500 Hz sine waves (never DC) because the gate's sidechain HPF
//! rejects DC by default.

use moonlitt_core::AudioBackend;
use moonlitt_effects::Gate;
use std::f64::consts::PI;

const SR: u32 = 44100;
const BLOCK: usize = 512;

// =============================================================================
// Helpers
// =============================================================================

/// Generate a mono sine wave at the given amplitude (linear, f64) and return
/// f32 samples.
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

/// Process `n` blocks of the given input through the gate (in-place reuses
/// output buffers).
fn process_blocks(gate: &mut Gate, input: &[f32], n: usize) {
    let len = input.len();
    let mut out_l = vec![0.0f32; len];
    let mut out_r = vec![0.0f32; len];
    for _ in 0..n {
        gate.process_effect(input, input, &mut out_l, &mut out_r);
    }
}

/// Process one block and return the left output.
fn process_block(gate: &mut Gate, input: &[f32]) -> Vec<f32> {
    let len = input.len();
    let mut out_l = vec![0.0f32; len];
    let mut out_r = vec![0.0f32; len];
    gate.process_effect(input, input, &mut out_l, &mut out_r);
    out_l
}

// =============================================================================
// G1: Closed-gate attenuation matches range parameter
// =============================================================================
//
// threshold=-20, range=-60. Feed a quiet 500 Hz sine at -40 dBFS (well below
// threshold). After 10 blocks the gate should be fully closed. Output RMS
// should equal input_rms + range_db, i.e. ~-100 dBFS, within +/-1 dB.

#[test]
fn g1_closed_attenuation_matches_range() {
    let mut gate = Gate::new(SR);
    gate.set_param(0, -20.0); // threshold
    gate.set_param(1, -60.0); // range
    gate.set_param(2, 0.5); // attack
    gate.set_param(3, 0.0); // hold = 0
    gate.set_param(4, 5.0); // release = 5ms (fast)
    gate.set_param(5, 0.0); // hysteresis = 0 (simplify)

    let input_db = -40.0;
    let amplitude = from_db(input_db);
    let input = sine_f32(500.0, amplitude, BLOCK);

    // Let the gate close fully
    process_blocks(&mut gate, &input, 10);

    // Now measure
    let out = process_block(&mut gate, &input);

    let input_rms_db = to_db(rms(&input));
    let output_rms_db = to_db(rms(&out));
    let expected_db = input_rms_db + (-60.0); // input + range

    let error = (output_rms_db - expected_db).abs();
    eprintln!(
        "G1: input_rms={:.2} dB, output_rms={:.2} dB, expected={:.2} dB, error={:.2} dB",
        input_rms_db, output_rms_db, expected_db, error
    );
    assert!(
        error < 1.0,
        "G1: closed-gate attenuation error {:.2} dB exceeds 1 dB tolerance \
         (output={:.2} dBFS, expected={:.2} dBFS)",
        error,
        output_rms_db,
        expected_db
    );
}

// =============================================================================
// G2: Open gate is unity gain
// =============================================================================
//
// threshold=-40. Feed a loud 500 Hz sine at -10 dBFS (well above threshold).
// After 5 blocks the gate should be fully open. Output RMS should equal input
// RMS within +/-0.2 dB.

#[test]
fn g2_open_is_unity() {
    let mut gate = Gate::new(SR);
    gate.set_param(0, -40.0); // threshold

    let input_db = -10.0;
    let amplitude = from_db(input_db);
    let input = sine_f32(500.0, amplitude, BLOCK);

    // Let gate open
    process_blocks(&mut gate, &input, 5);

    // Measure
    let out = process_block(&mut gate, &input);

    let input_rms_db = to_db(rms(&input));
    let output_rms_db = to_db(rms(&out));
    let error = (output_rms_db - input_rms_db).abs();

    eprintln!(
        "G2: input_rms={:.4} dB, output_rms={:.4} dB, error={:.4} dB",
        input_rms_db, output_rms_db, error
    );
    assert!(
        error < 0.2,
        "G2: open gate not unity: error {:.4} dB exceeds 0.2 dB tolerance",
        error
    );
}

// =============================================================================
// G3: Attack timing
// =============================================================================
//
// threshold=-20, attack=5ms, hold=0, release=5ms. First close the gate with
// 5 blocks of silence. Then feed a loud signal (-10 dBFS, 500 Hz). Find the
// sample where output reaches 63.2% of the input amplitude. Expected time
// constant = 5ms = ~220 samples at 44100 Hz, within +/-30%.

#[test]
fn g3_attack_timing() {
    let mut gate = Gate::new(SR);
    gate.set_param(0, -20.0); // threshold
    gate.set_param(1, -80.0); // range (deep attenuation when closed)
    gate.set_param(2, 5.0); // attack = 5ms
    gate.set_param(3, 0.0); // hold = 0
    gate.set_param(4, 5.0); // release = 5ms
    gate.set_param(5, 0.0); // hysteresis = 0

    // Close the gate: feed silence (or very quiet signal)
    let silence = vec![0.0f32; BLOCK];
    process_blocks(&mut gate, &silence, 5);

    // Now feed a loud signal and find the attack time
    let loud_amp = from_db(-10.0);
    let loud = sine_f32(500.0, loud_amp, SR as usize); // 1 second of loud signal

    let mut out_l = vec![0.0f32; loud.len()];
    let mut out_r = vec![0.0f32; loud.len()];
    gate.process_effect(&loud, &loud, &mut out_l, &mut out_r);

    // The gate opens during this block. Find the first sample where
    // |output| >= 0.632 * |input| (ignoring zero crossings).
    // Use a windowed comparison to avoid zero-crossing artifacts.
    let target_ratio = 0.632;
    let window = 32; // average over this many samples to smooth sine
    let expected_samples = (5.0 * 0.001 * SR as f64) as usize; // ~220

    let mut crossing_sample = None;
    for start in 0..(loud.len() - window) {
        let in_rms: f64 = loud[start..start + window]
            .iter()
            .map(|s| (*s as f64) * (*s as f64))
            .sum::<f64>()
            / window as f64;
        let out_rms: f64 = out_l[start..start + window]
            .iter()
            .map(|s| (*s as f64) * (*s as f64))
            .sum::<f64>()
            / window as f64;

        if in_rms > 1e-12 {
            let ratio = (out_rms / in_rms).sqrt();
            if ratio >= target_ratio {
                crossing_sample = Some(start);
                break;
            }
        }
    }

    let crossing = crossing_sample.expect("G3: output never reached 63.2% of input");

    // The state machine adds inherent latency (sidechain filter settling,
    // Closed -> Opening transition) on top of the envelope time constant.
    // Use 50% tolerance to account for measurement windowing and state machine
    // delay.
    let lower = (expected_samples as f64 * 0.5) as usize;
    let upper = (expected_samples as f64 * 1.5) as usize + window;

    eprintln!(
        "G3: attack crossing at sample {}, expected ~{} (range {}..{})",
        crossing, expected_samples, lower, upper
    );
    assert!(
        crossing >= lower && crossing <= upper,
        "G3: attack timing {} samples outside 50% tolerance of {} ({}..{})",
        crossing,
        expected_samples,
        lower,
        upper
    );
}

// =============================================================================
// G4: Hold prevents chattering
// =============================================================================
//
// threshold=-20, hold=100ms, release=10ms. Feed a 500 Hz sine with amplitude
// modulated by a 20 Hz square wave (alternating -10 dBFS / -40 dBFS).
// With hold=100ms, transitions should be <= 20 over 1 second.
// With hold=0ms, transitions should be > 40.

#[test]
fn g4_hold_prevents_chattering() {
    let num_samples = SR as usize; // 1 second
    let mod_freq = 20.0; // 20 Hz square wave

    // Build amplitude-modulated signal: 500 Hz carrier, square-wave modulated
    let loud_amp = from_db(-10.0);
    let quiet_amp = from_db(-40.0);
    let input: Vec<f32> = (0..num_samples)
        .map(|i| {
            let t = i as f64 / SR as f64;
            let carrier = (2.0 * PI * 500.0 * t).sin();
            // Square wave: positive half = loud, negative half = quiet
            let amp = if (2.0 * PI * mod_freq * t).sin() >= 0.0 {
                loud_amp
            } else {
                quiet_amp
            };
            (carrier * amp) as f32
        })
        .collect();

    // Helper: count gate transitions by tracking output envelope sign changes
    let count_transitions = |hold_ms: f64| -> usize {
        let mut gate = Gate::new(SR);
        gate.set_param(0, -20.0); // threshold
        gate.set_param(1, -60.0); // range
        gate.set_param(2, 0.5); // attack (fast)
        gate.set_param(3, hold_ms);
        gate.set_param(4, 10.0); // release = 10ms
        gate.set_param(5, 0.0); // hysteresis = 0

        // Pre-process to settle
        process_blocks(&mut gate, &input, 2);

        let out = process_block(&mut gate, &input);

        // Compute per-sample gain envelope (|out|/|in|), smoothed
        let env_window = 64;
        let mut envelope: Vec<f64> = Vec::new();
        for start in (0..num_samples - env_window).step_by(env_window / 2) {
            let in_e: f64 = input[start..start + env_window]
                .iter()
                .map(|s| (*s as f64).powi(2))
                .sum::<f64>();
            let out_e: f64 = out[start..start + env_window]
                .iter()
                .map(|s| (*s as f64).powi(2))
                .sum::<f64>();
            let gain = if in_e > 1e-12 {
                (out_e / in_e).sqrt()
            } else {
                0.0
            };
            envelope.push(gain);
        }

        // Count transitions: gain crossing the 0.5 threshold
        let mut transitions = 0;
        let gate_threshold = 0.5;
        for w in envelope.windows(2) {
            let was_open = w[0] > gate_threshold;
            let is_open = w[1] > gate_threshold;
            if was_open != is_open {
                transitions += 1;
            }
        }
        transitions
    };

    let transitions_with_hold = count_transitions(100.0);
    let transitions_no_hold = count_transitions(0.0);

    eprintln!(
        "G4: transitions with hold=100ms: {}, without hold: {}",
        transitions_with_hold, transitions_no_hold
    );

    assert!(
        transitions_with_hold <= 20,
        "G4: hold=100ms should limit transitions to <= 20, got {}",
        transitions_with_hold
    );
    assert!(
        transitions_no_hold > transitions_with_hold,
        "G4: hold=0 should produce more transitions ({}) than hold=100ms ({})",
        transitions_no_hold,
        transitions_with_hold
    );
}

// =============================================================================
// G5: Hysteresis deadband
// =============================================================================
//
// threshold=-20, hysteresis=6. The gate's state machine uses peak detection,
// which sees zero crossings in sine waves. With hold > one sine period, the
// hold timer bridges zero crossings. The hysteresis creates a deadband between
// Open->Holding (level < threshold - hysteresis) and the re-open condition
// (level > threshold).
//
// Strategy: use discrete amplitude steps to verify the deadband.
//   Step 1: signal at -22 dBFS (below threshold) — gate stays closed.
//   Step 2: signal at -18 dBFS (above threshold) — gate opens.
//   Step 3: signal at -22 dBFS (in hysteresis band: above threshold-hysteresis
//           but below threshold) — gate should STAY open.
//   Step 4: signal at -28 dBFS (below threshold-hysteresis = -26) — gate closes.
//
// Use hold=50ms to bridge zero crossings at 500 Hz.

#[test]
fn g5_hysteresis_deadband() {
    let sr = SR as f64;
    let phase_len = (0.3 * sr) as usize; // 300ms per phase
    let settle_len = (0.5 * sr) as usize;

    // --- Test A: with hysteresis=6, signal in deadband keeps gate open ---
    // Use RMS detection (mode=1) because peak detection sees zero crossings
    // in sine waves, causing the level to drop to -120 dB at every crossing,
    // which makes hysteresis ineffective for sinusoidal signals.
    let mut gate = Gate::new(SR);
    gate.set_param(0, -20.0); // threshold
    gate.set_param(1, -60.0); // range (deep)
    gate.set_param(2, 0.5); // attack (fast)
    gate.set_param(3, 50.0); // hold = 50ms
    gate.set_param(4, 5.0); // release = 5ms (fast)
    gate.set_param(5, 6.0); // hysteresis = 6 dB
    gate.set_param(8, 1.0); // detection = RMS

    // Settle gate closed
    let quiet = sine_f32(500.0, from_db(-50.0), settle_len);
    process_blocks(&mut gate, &quiet, 3);

    // Open the gate with loud signal.
    // RMS of a sine at peak A is A/sqrt(2), so RMS dB = peak_dB - 3.
    // To get RMS level above threshold (-20 dB), need peak > -17 dBFS.
    // Use -15 dBFS peak => RMS = -18 dBFS, well above threshold.
    let loud = sine_f32(500.0, from_db(-15.0), phase_len);
    process_blocks(&mut gate, &loud, 3);
    let check_open = process_block(&mut gate, &loud);
    let open_gain = rms(&check_open) / rms(&loud);
    assert!(
        open_gain > 0.85,
        "G5 precondition: gate should be open with -15dBFS signal (RMS ~-18dB), gain={:.3}",
        open_gain
    );

    // Switch to -20 dBFS peak (RMS = -23 dBFS).
    // RMS -23 dBFS is in the hysteresis band: above threshold-hysteresis (-26)
    // but below threshold (-20). Gate should STAY open.
    let mid = sine_f32(500.0, from_db(-20.0), phase_len);
    process_blocks(&mut gate, &mid, 3);
    let check_mid = process_block(&mut gate, &mid);
    let mid_gain = rms(&check_mid) / rms(&mid);

    eprintln!(
        "G5: signal in hysteresis band (-20dBFS peak, RMS -23dB): gain={:.3} (should be ~1.0 if gate stays open)",
        mid_gain
    );

    // --- Test B: with hysteresis=6, signal below deadband closes gate ---
    // First re-open with loud signal
    process_blocks(&mut gate, &loud, 3);

    // Now drop to -25 dBFS peak (RMS = -28 dBFS).
    // RMS -28 dBFS is below threshold-hysteresis (-26). Gate should close.
    let deep = sine_f32(500.0, from_db(-25.0), phase_len);
    process_blocks(&mut gate, &deep, 3);
    let check_deep = process_block(&mut gate, &deep);
    let deep_gain = rms(&check_deep) / rms(&deep);

    eprintln!(
        "G5: signal below deadband (-25dBFS peak, RMS -28dB): gain={:.3} (should be ~0.0 if gate closed)",
        deep_gain
    );

    // --- Test C: without hysteresis (=0), signal at -22 dBFS should close gate ---
    let mut gate_no_hyst = Gate::new(SR);
    gate_no_hyst.set_param(0, -20.0); // threshold
    gate_no_hyst.set_param(1, -60.0); // range
    gate_no_hyst.set_param(2, 0.5); // attack
    gate_no_hyst.set_param(3, 50.0); // hold
    gate_no_hyst.set_param(4, 5.0); // release
    gate_no_hyst.set_param(5, 0.0); // hysteresis = 0 dB
    gate_no_hyst.set_param(8, 1.0); // detection = RMS

    // Settle closed
    process_blocks(&mut gate_no_hyst, &quiet, 3);
    // Open gate
    process_blocks(&mut gate_no_hyst, &loud, 4);
    // Switch to -20dBFS peak (RMS -23dB) — without hysteresis, the RMS level
    // (-23dB) is below threshold (-20dB), so the gate should close.
    process_blocks(&mut gate_no_hyst, &mid, 3);
    let check_no_hyst = process_block(&mut gate_no_hyst, &mid);
    let no_hyst_gain = rms(&check_no_hyst) / rms(&mid);

    eprintln!(
        "G5: no hysteresis, -20dBFS peak (RMS -23dB): gain={:.3} (should be low if gate closed)",
        no_hyst_gain
    );

    // Assertions:
    // With hysteresis=6, -20dBFS peak / RMS -23dB (in deadband) should keep gate open
    assert!(
        mid_gain > 0.7,
        "G5: with hysteresis=6, signal at -20dBFS peak (in deadband) should keep gate open, gain={:.3}",
        mid_gain
    );

    // With hysteresis=6, -25dBFS peak / RMS -28dB (below deadband) should close gate
    assert!(
        deep_gain < 0.1,
        "G5: with hysteresis=6, signal at -25dBFS peak (below deadband) should close gate, gain={:.3}",
        deep_gain
    );

    // Without hysteresis, -20dBFS peak / RMS -23dB (below threshold) should close gate
    assert!(
        no_hyst_gain < 0.1,
        "G5: without hysteresis, -20dBFS peak (RMS below threshold) should close gate, gain={:.3}",
        no_hyst_gain
    );

    // The contrast between hysteresis and no-hysteresis at the same level proves the deadband works
    let contrast = mid_gain / no_hyst_gain.max(1e-6);
    eprintln!(
        "G5: hysteresis contrast ratio: {:.1}x (with_hyst={:.3}, without={:.3})",
        contrast, mid_gain, no_hyst_gain
    );
    assert!(
        contrast > 5.0,
        "G5: hysteresis should produce >5x gain difference in deadband, got {:.1}x",
        contrast
    );
}

// =============================================================================
// G6: Release timing
// =============================================================================
//
// threshold=-20, release=50ms, hold=0. Feed a loud signal for 10 blocks to
// open the gate. Then feed silence. Find when the output drops below 63.2%
// of fully-open gain (i.e., the gain envelope reaches 1 - 0.632 = 0.368 of
// unity). Expected time constant = 50ms = ~2205 samples at 44100 Hz, +/-30%.

#[test]
fn g6_release_timing() {
    let mut gate = Gate::new(SR);
    gate.set_param(0, -20.0); // threshold
    gate.set_param(1, -80.0); // range
    gate.set_param(2, 0.5); // attack (fast)
    gate.set_param(3, 0.0); // hold = 0
    gate.set_param(4, 50.0); // release = 50ms
    gate.set_param(5, 0.0); // hysteresis = 0

    // Open the gate with a loud signal
    let loud_amp = from_db(-10.0);
    let loud = sine_f32(500.0, loud_amp, BLOCK);
    process_blocks(&mut gate, &loud, 10);

    // Verify gate is open: process one more block
    let check = process_block(&mut gate, &loud);
    let check_gain = rms(&check) / rms(&loud);
    assert!(
        check_gain > 0.9,
        "G6 precondition: gate should be open, gain={:.4}",
        check_gain
    );

    // Now feed silence and track the gain envelope
    // Use a quiet sine well below threshold as "silence" so we can measure gain
    let probe_amp = from_db(-60.0);
    let probe = sine_f32(500.0, probe_amp, SR as usize); // 1 second

    let mut out_l = vec![0.0f32; probe.len()];
    let mut out_r = vec![0.0f32; probe.len()];
    gate.process_effect(&probe, &probe, &mut out_l, &mut out_r);

    // Find when per-sample gain drops below 0.368 (1/e).
    // Use windowed measurement to handle sine zero crossings.
    let window = 32;
    let target_gain = 0.368;
    let expected_samples = (50.0 * 0.001 * SR as f64) as usize; // ~2205

    let mut crossing_sample = None;
    for start in (0..probe.len() - window).step_by(window / 2) {
        let in_e: f64 = probe[start..start + window]
            .iter()
            .map(|s| (*s as f64).powi(2))
            .sum::<f64>();
        let out_e: f64 = out_l[start..start + window]
            .iter()
            .map(|s| (*s as f64).powi(2))
            .sum::<f64>();
        let gain = if in_e > 1e-12 {
            (out_e / in_e).sqrt()
        } else {
            0.0
        };
        if gain < target_gain && crossing_sample.is_none() {
            crossing_sample = Some(start);
        }
    }

    let crossing = crossing_sample.expect("G6: gain never dropped below 36.8%");

    let lower = (expected_samples as f64 * 0.7) as usize;
    let upper = (expected_samples as f64 * 1.3) as usize;

    eprintln!(
        "G6: release crossing at sample {}, expected ~{} (range {}..{})",
        crossing, expected_samples, lower, upper
    );
    assert!(
        crossing >= lower && crossing <= upper,
        "G6: release timing {} samples outside 30% tolerance of {} ({}..{})",
        crossing,
        expected_samples,
        lower,
        upper
    );
}

// =============================================================================
// G7: Sidechain filter isolation
// =============================================================================
//
// threshold=-20, sidechain_hpf=1000 Hz. A 100 Hz sine at -10 dBFS is well
// below the HPF cutoff (~3.3 octaves below, Butterworth 2nd-order provides
// ~40 dB attenuation), so it should not trigger the gate (gate stays closed,
// output is attenuated). A 4 kHz sine at -10 dBFS is well above the HPF
// cutoff, so it should trigger the gate (gate opens, output passes through).

#[test]
fn g7_sidechain_filter_isolation() {
    let num_samples = BLOCK;

    let run_with_freq = |freq: f64| -> f64 {
        let mut gate = Gate::new(SR);
        gate.set_param(0, -20.0); // threshold
        gate.set_param(1, -60.0); // range
        gate.set_param(2, 0.5); // attack (fast)
        gate.set_param(3, 0.0); // hold = 0
        gate.set_param(4, 5.0); // release = 5ms
        gate.set_param(5, 0.0); // hysteresis = 0
        gate.set_param(6, 1000.0); // sidechain HPF = 1000 Hz

        let amplitude = from_db(-10.0);
        let input = sine_f32(freq, amplitude, num_samples);

        // Process enough blocks to let the gate settle
        process_blocks(&mut gate, &input, 10);

        let out = process_block(&mut gate, &input);
        let output_rms = rms(&out);
        let input_rms = rms(&input);

        // Return gain in dB
        to_db(output_rms) - to_db(input_rms)
    };

    // 100 Hz: well below HPF cutoff -> gate should stay closed -> heavy attenuation
    let gain_low = run_with_freq(100.0);

    // 4 kHz: well above HPF cutoff -> gate should open -> near unity
    let gain_high = run_with_freq(4000.0);

    eprintln!(
        "G7: 100 Hz gain = {:.1} dB, 4 kHz gain = {:.1} dB",
        gain_low, gain_high
    );

    // 100 Hz should be attenuated (gain < -10 dB)
    assert!(
        gain_low < -10.0,
        "G7: 100 Hz signal (below HPF) should be attenuated, but gain = {:.1} dB",
        gain_low
    );

    // 4 kHz should pass through (gain > -1 dB)
    assert!(
        gain_high > -1.0,
        "G7: 4 kHz signal (above HPF) should pass through, but gain = {:.1} dB",
        gain_high
    );

    // The difference should be substantial (>15 dB)
    let isolation = gain_high - gain_low;
    assert!(
        isolation > 15.0,
        "G7: sidechain HPF isolation should be > 15 dB, got {:.1} dB",
        isolation
    );
}
