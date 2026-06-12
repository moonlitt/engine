//! Auto-Filter Compliance Tests
//!
//! Validates envelope frequency tracking, resonance peak, filter type
//! frequency response, and LFO sweep periodicity.
//!
//! All tests use the AudioBackend trait for parameter access and processing.

use moonlitt_core::AudioBackend;
use moonlitt_effects::AutoFilter;
use std::f64::consts::PI;

const SR: u32 = 44100;

// =============================================================================
// Helpers
// =============================================================================

/// Convert linear amplitude to dBFS.
fn to_db(linear: f64) -> f64 {
    if linear > 1e-12 {
        20.0 * linear.log10()
    } else {
        -240.0
    }
}

/// Generate deterministic pseudo-noise. Consistent across runs.
fn pseudo_noise(num_samples: usize, amplitude: f64) -> Vec<f32> {
    // Simple hash-based noise: deterministic, broadband, no external dependency
    let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
    (0..num_samples)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let normalized = (state as f64 / u64::MAX as f64) * 2.0 - 1.0;
            (normalized * amplitude) as f32
        })
        .collect()
}

/// Measure RMS energy in a frequency band using a simple DFT approach.
/// Computes the Goertzel magnitude for a given frequency.
fn goertzel_magnitude(buf: &[f32], freq: f64) -> f64 {
    let n = buf.len();
    let k = (freq * n as f64 / SR as f64).round();
    let w = 2.0 * PI * k / n as f64;
    let coeff = 2.0 * w.cos();
    let mut s1 = 0.0f64;
    let mut s2 = 0.0f64;
    for &sample in buf {
        let s0 = sample as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    (power.max(0.0).sqrt()) / n as f64
}

/// Measure high-frequency energy (above `cutoff_hz`) using band-limited RMS.
/// Uses a simple first-order highpass approximation via differencing.
fn hf_energy(buf: &[f32], _cutoff_hz: f64) -> f64 {
    // High-frequency energy: sum of squared differences (approximates HPF)
    // More differences = more HF content
    if buf.len() < 2 {
        return 0.0;
    }
    let sum_sq: f64 = buf
        .windows(2)
        .map(|w| {
            let diff = w[1] as f64 - w[0] as f64;
            diff * diff
        })
        .sum();
    (sum_sq / (buf.len() - 1) as f64).sqrt()
}

// =============================================================================
// af1: Envelope Frequency Tracking
// =============================================================================
//
// source=0(env), filter_type=0(LP), min_freq=200, max_freq=8000,
// sensitivity=1.0, attack=1ms, release=10ms.
// Feed alternating blocks: loud signal then quiet.
// Loud blocks should have more HF content (filter opened by envelope).

#[test]
fn af1_envelope_frequency_tracking() {
    let mut af = AutoFilter::new(SR);
    af.set_param(0, 0.0); // source = Envelope
    af.set_param(1, 0.0); // filter_type = LP
    af.set_param(2, 200.0); // min_freq = 200 Hz
    af.set_param(3, 8000.0); // max_freq = 8000 Hz
    af.set_param(4, 1.0); // resonance = moderate
    af.set_param(5, 1.0); // sensitivity = max
    af.set_param(6, 1.0); // attack = 1 ms
    af.set_param(7, 10.0); // release = 10 ms
    af.set_param(10, 1.0); // dry_wet = 100%

    let block_size = SR as usize / 10; // 100ms blocks
    let loud_amplitude = 0.8;
    let quiet_amplitude = 0.01;

    // Generate broadband signal (noise-like with many frequencies)
    let loud_input = pseudo_noise(block_size, loud_amplitude);
    let quiet_input = pseudo_noise(block_size, quiet_amplitude);

    // Process 5 loud blocks, then 5 quiet blocks
    let mut loud_outputs = Vec::new();
    let mut quiet_outputs = Vec::new();

    for _ in 0..5 {
        let mut out_l = vec![0.0f32; block_size];
        let mut out_r = vec![0.0f32; block_size];
        af.process_effect(&loud_input, &loud_input, &mut out_l, &mut out_r);
        loud_outputs.push(out_l);
    }

    for _ in 0..5 {
        let mut out_l = vec![0.0f32; block_size];
        let mut out_r = vec![0.0f32; block_size];
        af.process_effect(&quiet_input, &quiet_input, &mut out_l, &mut out_r);
        quiet_outputs.push(out_l);
    }

    // Measure HF content in the last loud block vs last quiet block
    let loud_hf = hf_energy(&loud_outputs[4], 3000.0);
    let quiet_hf = hf_energy(&quiet_outputs[4], 3000.0);

    // Account for amplitude difference: normalize HF by input RMS
    let loud_normalized = loud_hf / loud_amplitude;
    let quiet_normalized = quiet_hf / quiet_amplitude.max(1e-10);

    assert!(
        loud_normalized > quiet_normalized,
        "af1: loud blocks should have more HF content (filter opened by envelope): \
         loud_hf_normalized={:.6}, quiet_hf_normalized={:.6}",
        loud_normalized,
        quiet_normalized
    );
}

// =============================================================================
// af2: Resonance Peak — high Q produces a spectral peak at cutoff
// =============================================================================
//
// source=1(LFO), lfo_rate=0.05 (near minimum, nearly static), filter_type=0(LP),
// min_freq=1000, max_freq=1000 (static cutoff), resonance=15.
// Feed white noise. FFT output should show peak near 1 kHz.

#[test]
fn af2_resonance_peak() {
    let mut af = AutoFilter::new(SR);
    af.set_param(0, 1.0); // source = LFO
    af.set_param(1, 0.0); // filter_type = LP
    af.set_param(2, 1000.0); // min_freq = 1000 Hz
    af.set_param(3, 1000.0); // max_freq = 1000 Hz (static)
    af.set_param(4, 15.0); // resonance = 15 (high Q)
    af.set_param(5, 1.0); // sensitivity = 1.0
    af.set_param(8, 0.05); // lfo_rate = near minimum (nearly static)
    af.set_param(10, 1.0); // dry_wet = 100%

    let num_samples = SR as usize * 2;
    let input = pseudo_noise(num_samples, 0.3);
    let mut out_l = vec![0.0f32; num_samples];
    let mut out_r = vec![0.0f32; num_samples];

    af.process_effect(&input, &input, &mut out_l, &mut out_r);

    // Measure energy at 1 kHz vs nearby frequencies
    let measure_start = num_samples / 2;
    let out_slice = &out_l[measure_start..];

    let mag_1k = goertzel_magnitude(out_slice, 1000.0);
    let mag_300 = goertzel_magnitude(out_slice, 300.0);
    let mag_3k = goertzel_magnitude(out_slice, 3000.0);

    let peak_db = to_db(mag_1k);
    let ref_db = to_db((mag_300 + mag_3k) / 2.0);

    assert!(
        peak_db - ref_db > 10.0,
        "af2: resonance peak at 1 kHz should be >10 dB above neighbors: \
         peak={:.1} dB, reference={:.1} dB, diff={:.1} dB",
        peak_db,
        ref_db,
        peak_db - ref_db
    );
}

// =============================================================================
// af3: Filter Type Response — LP, HP, BP each shape spectrum differently
// =============================================================================

#[test]
fn af3_filter_type_response() {
    let num_samples = SR as usize * 2;
    let input = pseudo_noise(num_samples, 0.3);
    let measure_start = num_samples / 2;

    // Helper to create a static-cutoff auto-filter at 2 kHz
    let create_af = |filter_type: f64| -> AutoFilter {
        let mut af = AutoFilter::new(SR);
        af.set_param(0, 1.0); // source = LFO
        af.set_param(1, filter_type);
        af.set_param(2, 2000.0); // min_freq = 2000 Hz
        af.set_param(3, 2000.0); // max_freq = 2000 Hz (static)
        af.set_param(4, 1.0); // resonance = moderate
        af.set_param(5, 1.0); // sensitivity
        af.set_param(8, 0.05); // lfo_rate = nearly static
        af.set_param(10, 1.0); // dry_wet = 100%
        af
    };

    // --- Lowpass (type=0) ---
    {
        let mut af = create_af(0.0);
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        af.process_effect(&input, &input, &mut out_l, &mut out_r);

        let out_slice = &out_l[measure_start..];
        let low_energy = goertzel_magnitude(out_slice, 500.0);
        let high_energy = goertzel_magnitude(out_slice, 8000.0);

        let diff_db = to_db(low_energy) - to_db(high_energy);
        assert!(
            diff_db > 10.0,
            "af3 LP: below cutoff should be >10 dB above high freq: diff={:.1} dB",
            diff_db
        );
    }

    // --- Highpass (type=1) ---
    {
        let mut af = create_af(1.0);
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        af.process_effect(&input, &input, &mut out_l, &mut out_r);

        let out_slice = &out_l[measure_start..];
        let low_energy = goertzel_magnitude(out_slice, 500.0);
        let high_energy = goertzel_magnitude(out_slice, 8000.0);

        let diff_db = to_db(high_energy) - to_db(low_energy);
        assert!(
            diff_db > 10.0,
            "af3 HP: above cutoff should be >10 dB above low freq: diff={:.1} dB",
            diff_db
        );
    }

    // --- Bandpass (type=2) ---
    {
        let mut af = create_af(2.0);
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];
        af.process_effect(&input, &input, &mut out_l, &mut out_r);

        let out_slice = &out_l[measure_start..];
        let center_energy = goertzel_magnitude(out_slice, 2000.0);
        let low_energy = goertzel_magnitude(out_slice, 500.0);
        let high_energy = goertzel_magnitude(out_slice, 8000.0);

        let center_db = to_db(center_energy);
        let low_db = to_db(low_energy);
        let high_db = to_db(high_energy);

        assert!(
            center_db > low_db,
            "af3 BP: center (2kHz) should be above 500Hz: center={:.1} dB, low={:.1} dB",
            center_db,
            low_db
        );
        assert!(
            center_db > high_db,
            "af3 BP: center (2kHz) should be above 8kHz: center={:.1} dB, high={:.1} dB",
            center_db,
            high_db
        );
    }
}

// =============================================================================
// af4: LFO Sweep Period — 2 Hz LFO should produce ~4 peaks in 2 seconds
// =============================================================================
//
// source=1(LFO), rate=2.0, filter_type=0(LP), min_freq=200, max_freq=8000.
// Feed constant white noise for 2 seconds.
// Divide output into 100ms blocks. Measure HF energy per block.
// The HF energy should oscillate with ≈ 2 Hz period (4 peaks in 2 seconds ±1).

#[test]
fn af4_lfo_sweep_period() {
    let mut af = AutoFilter::new(SR);
    af.set_param(0, 1.0); // source = LFO
    af.set_param(1, 0.0); // filter_type = LP
    af.set_param(2, 200.0); // min_freq = 200 Hz
    af.set_param(3, 8000.0); // max_freq = 8000 Hz
    af.set_param(4, 1.0); // resonance = moderate
    af.set_param(5, 1.0); // sensitivity = 1.0
    af.set_param(8, 2.0); // lfo_rate = 2.0 Hz
    af.set_param(9, 0.0); // lfo_shape = Sine
    af.set_param(10, 1.0); // dry_wet = 100%

    let total_samples = SR as usize * 2; // 2 seconds
    let input = pseudo_noise(total_samples, 0.3);
    let mut out_l = vec![0.0f32; total_samples];
    let mut out_r = vec![0.0f32; total_samples];

    af.process_effect(&input, &input, &mut out_l, &mut out_r);

    // Divide into 100ms blocks (20 blocks total for 2 seconds)
    let block_size = SR as usize / 10; // 4410 samples = 100ms
    let num_blocks = total_samples / block_size;

    let hf_values: Vec<f64> = (0..num_blocks)
        .map(|b| {
            let start = b * block_size;
            let end = start + block_size;
            hf_energy(&out_l[start..end], 3000.0)
        })
        .collect();

    // Count peaks (local maxima) in the HF energy sequence
    let mut peak_count = 0;
    for i in 1..(hf_values.len() - 1) {
        if hf_values[i] > hf_values[i - 1] && hf_values[i] > hf_values[i + 1] {
            peak_count += 1;
        }
    }

    // At 2 Hz over 2 seconds, we expect ~4 peaks (2 cycles × 2 peaks per cycle
    // for LP filter responding to the absolute value of the LFO sweep).
    // Actually for a sine LFO going 0..1..0, the LP filter opens once per cycle,
    // so we expect ~2 HF energy peaks per second = 4 total.
    // Allow ±2 for boundary effects and filter settling.
    assert!(
        (2..=6).contains(&peak_count),
        "af4: expected ~4 HF energy peaks in 2 seconds at 2Hz LFO, got {}: {:?}",
        peak_count,
        hf_values
    );
}
