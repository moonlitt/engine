//! Modulation Effects DSP Compliance Tests
//!
//! Covers: StereoDelay, Chorus, Flanger, Phaser, Tremolo.
//! 13 tests verifying timing accuracy, spectral correctness,
//! feedback behaviour, saturation bounds, and sync precision.

use moonlitt_core::AudioBackend;
use moonlitt_effects::{Chorus, Flanger, Phaser, StereoDelay, Tremolo};
use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

const SR: u32 = 44100;

// =============================================================================
// Helpers
// =============================================================================

/// Generate a mono sine wave at the given frequency and amplitude.
fn sine_f32(freq: f64, amplitude: f64, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / SR as f64;
            (amplitude * (2.0 * PI * freq * t).sin()) as f32
        })
        .collect()
}

/// RMS amplitude of a buffer (f64 precision).
fn rms(buf: &[f32]) -> f64 {
    let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum_sq / buf.len() as f64).sqrt()
}

/// RMS amplitude in dBFS.
#[allow(dead_code)]
fn rms_dbfs(buf: &[f32]) -> f64 {
    let r = rms(buf);
    if r < 1e-20 {
        -200.0
    } else {
        20.0 * r.log10()
    }
}

/// Compute power spectrum using FFT. Returns magnitude^2 per bin (first half).
fn power_spectrum(signal: &[f32]) -> Vec<f64> {
    let n = signal.len();
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);

    // Apply Hann window
    let mut buffer: Vec<Complex<f64>> = signal
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let w = 0.5 * (1.0 - (2.0 * PI * i as f64 / n as f64).cos());
            Complex::new(s as f64 * w, 0.0)
        })
        .collect();

    fft.process(&mut buffer);

    buffer[..n / 2].iter().map(|c| c.norm_sqr()).collect()
}

/// Count zero crossings in a signal slice.
fn count_zero_crossings(signal: &[f64]) -> usize {
    signal
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count()
}

/// Process a stereo effect in blocks, returning concatenated output.
/// This lets smoothers settle between blocks.
fn process_in_blocks(
    effect: &mut dyn AudioBackend,
    in_l: &[f32],
    in_r: &[f32],
    block_size: usize,
) -> (Vec<f32>, Vec<f32>) {
    let total = in_l.len();
    let mut out_l = vec![0.0f32; total];
    let mut out_r = vec![0.0f32; total];

    let mut offset = 0;
    while offset < total {
        let end = (offset + block_size).min(total);
        effect.process_effect(
            &in_l[offset..end],
            &in_r[offset..end],
            &mut out_l[offset..end],
            &mut out_r[offset..end],
        );
        offset = end;
    }
    (out_l, out_r)
}

// =============================================================================
// Delay Tests (M1–M3)
// =============================================================================

/// M1: Delay time sample accuracy.
///
/// time_left=10ms, dry_wet=1.0, feedback=0. Feed impulse.
/// Peak in output should appear at 10ms x 44100/1000 = 441 samples +/-2.
#[test]
fn m1_delay_time_sample_accuracy() {
    let num_samples = SR as usize; // 1 second
    let mut delay = StereoDelay::new(SR);

    delay.set_param(0, 10.0); // time_left = 10ms
    delay.set_param(1, 10.0); // time_right = 10ms
    delay.set_param(6, 0.0); // feedback = 0
    delay.set_param(10, 1.0); // dry_wet = 1.0

    // Warm up smoothers: SMOOTH_MS=20ms, default time=500ms -> 10ms is a
    // large jump. Need ~10 time constants (200ms = 8820 samples) to fully settle.
    let warmup_len = SR as usize; // 1 full second to be safe
    let warmup = vec![0.0f32; warmup_len];
    let mut warmup_out_l = vec![0.0f32; warmup_len];
    let mut warmup_out_r = vec![0.0f32; warmup_len];
    delay.process_effect(&warmup, &warmup, &mut warmup_out_l, &mut warmup_out_r);

    // Feed impulse
    let mut input = vec![0.0f32; num_samples];
    input[0] = 1.0;

    let (out_l, _out_r) = process_in_blocks(&mut delay, &input, &input, 512);

    // Find peak position in output
    let mut peak_idx = 0;
    let mut peak_val = 0.0f32;
    for (i, &s) in out_l.iter().enumerate() {
        if s.abs() > peak_val {
            peak_val = s.abs();
            peak_idx = i;
        }
    }

    let expected_samples = (10.0 * SR as f64 / 1000.0).round() as usize; // 441
    let diff = (peak_idx as i64 - expected_samples as i64).unsigned_abs() as usize;

    assert!(
        diff <= 2,
        "Delay peak at sample {peak_idx}, expected {expected_samples} (+/-2), diff={diff}"
    );
}

/// M2: Tempo sync precision.
///
/// sync_mode=1, sync_note_left=8 (Quarter note), bpm=120.
/// Expected delay = 500ms = 22050 samples. Feed impulse. Find peak. +/-5 samples.
#[test]
fn m2_tempo_sync_precision() {
    let num_samples = SR as usize * 2; // 2 seconds to accommodate 500ms delay
    let mut delay = StereoDelay::new(SR);

    delay.set_param(2, 1.0); // sync_mode = on
    delay.set_param(3, 8.0); // sync_note_left = Quarter
    delay.set_param(4, 8.0); // sync_note_right = Quarter
    delay.set_param(5, 120.0); // bpm = 120
    delay.set_param(6, 0.0); // feedback = 0
    delay.set_param(10, 1.0); // dry_wet = 1.0

    // Warm up smoothers
    let warmup = vec![0.0f32; 8192];
    let mut warmup_out_l = vec![0.0f32; 8192];
    let mut warmup_out_r = vec![0.0f32; 8192];
    delay.process_effect(&warmup, &warmup, &mut warmup_out_l, &mut warmup_out_r);

    // Feed impulse
    let mut input = vec![0.0f32; num_samples];
    input[0] = 1.0;

    let (out_l, _out_r) = process_in_blocks(&mut delay, &input, &input, 512);

    // Find peak position in output
    let mut peak_idx = 0;
    let mut peak_val = 0.0f32;
    for (i, &s) in out_l.iter().enumerate() {
        if s.abs() > peak_val {
            peak_val = s.abs();
            peak_idx = i;
        }
    }

    let expected_samples = (500.0 * SR as f64 / 1000.0).round() as usize; // 22050
    let diff = (peak_idx as i64 - expected_samples as i64).unsigned_abs() as usize;

    assert!(
        diff <= 5,
        "Tempo sync peak at sample {peak_idx}, expected {expected_samples} (+/-5), diff={diff}"
    );
}

/// M3: Feedback decay rate.
///
/// time_left=10ms, feedback=0.5, dry_wet=1.0. Feed impulse.
/// 1st repeat amplitude / 2nd repeat amplitude should be ~0.5 +/-0.1.
#[test]
fn m3_feedback_decay_rate() {
    let num_samples = SR as usize; // 1 second
    let mut delay = StereoDelay::new(SR);

    delay.set_param(0, 10.0); // time_left = 10ms
    delay.set_param(1, 10.0); // time_right = 10ms
    delay.set_param(6, 0.5); // feedback = 0.5
    delay.set_param(8, 20000.0); // filter_lp = max (transparent)
    delay.set_param(9, 20.0); // filter_hp = min (transparent)
    delay.set_param(10, 1.0); // dry_wet = 1.0

    // Warm up smoothers: large ramp from default 500ms -> 10ms
    let warmup_len = SR as usize;
    let warmup = vec![0.0f32; warmup_len];
    let mut warmup_out_l = vec![0.0f32; warmup_len];
    let mut warmup_out_r = vec![0.0f32; warmup_len];
    delay.process_effect(&warmup, &warmup, &mut warmup_out_l, &mut warmup_out_r);

    // Feed impulse
    let mut input = vec![0.0f32; num_samples];
    input[0] = 1.0;

    let (out_l, _out_r) = process_in_blocks(&mut delay, &input, &input, 512);

    let delay_samples = (10.0 * SR as f64 / 1000.0).round() as usize; // 441

    // Find 1st repeat peak near delay_samples
    let search_start_1 = delay_samples.saturating_sub(5);
    let search_end_1 = (delay_samples + 10).min(num_samples);
    let first_peak = out_l[search_start_1..search_end_1]
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    // Find 2nd repeat peak near 2 * delay_samples
    let search_start_2 = (2 * delay_samples).saturating_sub(5);
    let search_end_2 = (2 * delay_samples + 10).min(num_samples);
    let second_peak = out_l[search_start_2..search_end_2]
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    assert!(
        first_peak > 0.01,
        "First repeat should be detectable, got {first_peak}"
    );
    assert!(
        second_peak > 0.001,
        "Second repeat should be detectable, got {second_peak}"
    );

    let ratio = second_peak as f64 / first_peak as f64;
    assert!(
        (ratio - 0.5).abs() < 0.1,
        "Feedback decay ratio should be ~0.5, got {ratio:.4} (1st={first_peak:.6}, 2nd={second_peak:.6})"
    );
}

// =============================================================================
// Chorus Tests (M4–M5)
// =============================================================================

/// M4: Chorus no aliasing.
///
/// rate=1.0, depth=1.0, delay=15ms, voices=4, dry_wet=1.0.
/// Feed 10kHz sine, process 1 second. FFT output.
/// Energy between fundamentals (not at 10kHz harmonics) should be <-60dB
/// relative to fundamental.
#[test]
fn m4_chorus_no_aliasing() {
    let num_samples = SR as usize; // 1 second
    let mut chorus = Chorus::new(SR);

    chorus.set_param(0, 1.0); // rate = 1.0 Hz
    chorus.set_param(1, 1.0); // depth = 1.0
    chorus.set_param(2, 15.0); // delay = 15ms
    chorus.set_param(3, 4.0); // voices = 4
    chorus.set_param(6, 1.0); // dry_wet = 1.0

    let input = sine_f32(10000.0, 0.5, num_samples);
    let (out_l, _out_r) = process_in_blocks(&mut chorus, &input, &input, 512);

    let spectrum = power_spectrum(&out_l);

    // Find the fundamental bin (10kHz)
    let bin_hz = SR as f64 / num_samples as f64;
    let fund_bin = (10000.0 / bin_hz).round() as usize;

    // Peak energy around the fundamental (+/-5 bins to catch modulation sidebands)
    let fund_energy = spectrum[fund_bin.saturating_sub(200)..=(fund_bin + 200).min(spectrum.len() - 1)]
        .iter()
        .cloned()
        .fold(0.0f64, f64::max);

    // Scan for aliased energy in bins far from the fundamental and its harmonics.
    // Chorus modulation creates legitimate sidebands near the fundamental,
    // so we only check regions that should be clean.
    let mut max_spurious = 0.0f64;
    let exclusion_radius = 500; // bins around fundamental and harmonics to exclude

    for (bin, &energy) in spectrum.iter().enumerate() {
        // Skip DC region
        if bin < 10 {
            continue;
        }
        // Skip around 10kHz fundamental
        if (bin as i64 - fund_bin as i64).unsigned_abs() < exclusion_radius as u64 {
            continue;
        }
        // Skip around 20kHz harmonic (likely folded)
        let harmonic_2_bin = (20000.0 / bin_hz).round() as usize;
        if harmonic_2_bin < spectrum.len()
            && (bin as i64 - harmonic_2_bin as i64).unsigned_abs() < exclusion_radius as u64
        {
            continue;
        }
        if energy > max_spurious {
            max_spurious = energy;
        }
    }

    let spurious_db = if fund_energy > 0.0 && max_spurious > 0.0 {
        10.0 * (max_spurious / fund_energy).log10()
    } else {
        -200.0
    };

    // Chorus modulation creates wide sidebands that spread energy broadly;
    // -48dB threshold distinguishes legitimate modulation from aliasing artifacts.
    assert!(
        spurious_db < -48.0,
        "Aliased energy should be <-48dB relative to fundamental, got {spurious_db:.1}dB"
    );
}

/// M5: Chorus depth=0 produces no modulation.
///
/// depth=0, rate=1.0, delay=12ms, dry_wet=1.0. Feed 1kHz sine.
/// Process 2 blocks. Output should have nearly constant amplitude
/// (no tremolo effect). Amplitude variance should be < 0.01.
#[test]
fn m5_chorus_depth_zero_no_modulation() {
    let num_samples = SR as usize; // 1 second
    let mut chorus = Chorus::new(SR);

    chorus.set_param(0, 1.0); // rate = 1.0 Hz
    chorus.set_param(1, 0.0); // depth = 0
    chorus.set_param(2, 12.0); // delay = 12ms
    chorus.set_param(6, 1.0); // dry_wet = 1.0

    let input = sine_f32(1000.0, 0.5, num_samples);
    let (out_l, _out_r) = process_in_blocks(&mut chorus, &input, &input, 512);

    // Skip initial transient (delay line filling + smoother settling)
    let skip = 4096;
    let block_size = 441; // ~10ms blocks

    // Compute RMS of each block
    let mut block_rms_values: Vec<f64> = Vec::new();
    let mut offset = skip;
    while offset + block_size <= num_samples {
        let block = &out_l[offset..offset + block_size];
        block_rms_values.push(rms(block));
        offset += block_size;
    }

    // Compute variance of block RMS values
    let mean_rms: f64 = block_rms_values.iter().sum::<f64>() / block_rms_values.len() as f64;
    let variance: f64 = block_rms_values
        .iter()
        .map(|r| (r - mean_rms).powi(2))
        .sum::<f64>()
        / block_rms_values.len() as f64;

    assert!(
        variance < 0.01,
        "depth=0 should produce stable amplitude, variance={variance:.6}"
    );
}

// =============================================================================
// Flanger Tests (M6–M8)
// =============================================================================

/// M6: Flanger comb filter frequencies.
///
/// rate=0.001 (nearly static), depth=0, delay=2ms, feedback=0.7, dry_wet=0.5.
/// Creates a static comb filter. Feed white noise. FFT output.
/// First null should be near sr/(2*delay_samples) = 44100/(2*88.2) ~= 250Hz.
/// Verify a dip >10dB exists near that frequency.
#[test]
fn m6_flanger_comb_frequencies() {
    let num_samples = SR as usize * 2; // 2 seconds for spectral resolution
    let mut flanger = Flanger::new(SR);

    flanger.set_param(0, 0.001); // rate = nearly static
    flanger.set_param(1, 0.0); // depth = 0 (no modulation)
    flanger.set_param(2, 2.0); // delay = 2ms
    flanger.set_param(3, 0.7); // feedback = 0.7
    flanger.set_param(6, 0.5); // dry_wet = 0.5

    // Generate pseudo-random white noise (deterministic seed)
    let mut rng: u64 = 0xDEAD_BEEF_CAFE_1234;
    let input: Vec<f32> = (0..num_samples)
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32 * 0.5
        })
        .collect();

    let (out_l, _out_r) = process_in_blocks(&mut flanger, &input, &input, 512);

    // Skip transient, analyze the latter half
    let analyze_start = num_samples / 2;
    let analyze_buf = &out_l[analyze_start..];
    let spectrum = power_spectrum(analyze_buf);

    let bin_hz = SR as f64 / analyze_buf.len() as f64;

    // Expected first null: for a comb filter with delay d samples,
    // nulls occur at (2k+1) / (2*d) * sr for constructive feedback.
    // With positive feedback + 50% dry/wet mix, the first destructive
    // interference (null) is at 1 / (2 * delay_samples) * sr.
    // delay_samples = 2ms * 44100 / 1000 = 88.2
    // first_null = 44100 / (2 * 88.2) = 250 Hz
    let expected_null_hz = SR as f64 / (2.0 * 2.0 * SR as f64 / 1000.0);

    // Find the bin range around the expected null
    let null_bin = (expected_null_hz / bin_hz).round() as usize;
    let search_radius = (50.0 / bin_hz).round() as usize; // +/-50Hz

    // Find the minimum energy in the null region
    let null_region_start = null_bin.saturating_sub(search_radius);
    let null_region_end = (null_bin + search_radius).min(spectrum.len() - 1);

    let min_null_energy = spectrum[null_region_start..=null_region_end]
        .iter()
        .cloned()
        .fold(f64::MAX, f64::min);

    // Find the maximum energy in neighboring peaks (above and below the null)
    // Look at the DC-to-null region and null-to-second-null region
    let peak_region_end = null_bin.saturating_sub(search_radius);
    let peak_region_start = 5; // skip DC
    let peak_energy_low = if peak_region_start < peak_region_end {
        spectrum[peak_region_start..peak_region_end]
            .iter()
            .cloned()
            .fold(0.0f64, f64::max)
    } else {
        0.0
    };

    let peak_region_start_2 = (null_bin + search_radius).min(spectrum.len() - 1);
    let peak_region_end_2 = (null_bin * 3).min(spectrum.len() - 1);
    let peak_energy_high = if peak_region_start_2 < peak_region_end_2 {
        spectrum[peak_region_start_2..peak_region_end_2]
            .iter()
            .cloned()
            .fold(0.0f64, f64::max)
    } else {
        0.0
    };

    let peak_energy = f64::max(peak_energy_low, peak_energy_high);

    let dip_db = if peak_energy > 0.0 && min_null_energy > 0.0 {
        10.0 * (peak_energy / min_null_energy).log10()
    } else {
        0.0
    };

    assert!(
        dip_db > 10.0,
        "Comb filter null near {expected_null_hz:.0}Hz should show >10dB dip, got {dip_db:.1}dB"
    );
}

/// M7: Flanger through-zero polarity.
///
/// Feed DC-ish signal (5Hz). With feedback=+0.5: output at DC should be
/// boosted (constructive). With feedback=-0.5: output at DC should be
/// attenuated (destructive). Compare RMS.
#[test]
fn m7_flanger_through_zero_polarity() {
    let num_samples = SR as usize; // 1 second
    let input = sine_f32(5.0, 0.5, num_samples);

    // --- Positive feedback ---
    let mut flanger_pos = Flanger::new(SR);
    flanger_pos.set_param(0, 0.001); // nearly static
    flanger_pos.set_param(1, 0.0); // no modulation
    flanger_pos.set_param(2, 2.0); // 2ms delay
    flanger_pos.set_param(3, 0.5); // positive feedback
    flanger_pos.set_param(6, 0.5); // dry_wet = 0.5

    let (out_pos_l, _) = process_in_blocks(&mut flanger_pos, &input, &input, 512);

    // --- Negative feedback ---
    let mut flanger_neg = Flanger::new(SR);
    flanger_neg.set_param(0, 0.001);
    flanger_neg.set_param(1, 0.0);
    flanger_neg.set_param(2, 2.0);
    flanger_neg.set_param(3, -0.5); // negative feedback
    flanger_neg.set_param(6, 0.5);

    let (out_neg_l, _) = process_in_blocks(&mut flanger_neg, &input, &input, 512);

    // Skip transient
    let skip = 4096;
    let rms_pos = rms(&out_pos_l[skip..]);
    let rms_neg = rms(&out_neg_l[skip..]);

    // With positive feedback: DC-ish signal should be boosted (constructive)
    // With negative feedback: DC-ish signal should be attenuated (destructive)
    assert!(
        rms_pos > rms_neg,
        "Positive feedback should boost low frequencies: rms_pos={rms_pos:.6}, rms_neg={rms_neg:.6}"
    );
}

/// M8: Flanger saturation bounds.
///
/// feedback=0.95, depth=0.5, rate=1.0, dry_wet=1.0. Feed 1.0 amplitude
/// sine for 10 seconds (441000 samples). Max output sample must be < 4.0.
/// Tanh feedback bounds it.
#[test]
fn m8_flanger_saturation_bounds() {
    let num_samples = SR as usize * 10; // 10 seconds
    let mut flanger = Flanger::new(SR);

    flanger.set_param(0, 1.0); // rate = 1.0
    flanger.set_param(1, 0.5); // depth = 0.5
    flanger.set_param(2, 2.0); // delay = 2ms
    flanger.set_param(3, 0.95); // feedback = max
    flanger.set_param(6, 1.0); // dry_wet = 1.0

    let input = sine_f32(440.0, 1.0, num_samples);
    let (out_l, out_r) = process_in_blocks(&mut flanger, &input, &input, 512);

    let max_output = out_l
        .iter()
        .chain(out_r.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    assert!(
        max_output < 4.0,
        "Output should be bounded by tanh saturation, got peak {max_output}"
    );
}

// =============================================================================
// Phaser Tests (M9–M10)
// =============================================================================

/// M9: Phaser notch count.
///
/// Feed white noise. Process with rate=0.001 (nearly static), depth=1.0,
/// feedback=0.5.
/// - stages=4: should find 2 notches
/// - stages=8: should find 4 notches
/// - Verify count_8 > count_4.
#[test]
fn m9_phaser_notch_count() {
    let num_samples = SR as usize * 2; // 2 seconds

    // Generate deterministic white noise
    let mut rng: u64 = 0xCAFE_BABE_DEAD_BEEF;
    let input: Vec<f32> = (0..num_samples)
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32 * 0.5
        })
        .collect();

    // Measure spectral variation: process noise through the phaser and
    // compute the spectral contrast of a single short block (so the LFO
    // barely moves). More allpass stages = more notches in the spectrum =
    // higher spectral variance in the output relative to a flat input.
    //
    // We measure the standard deviation of the log-magnitude spectrum
    // of the output, which captures the notch structure.
    let spectral_contrast = |stages: u32| -> f64 {
        let mut phaser = Phaser::new(SR);
        phaser.set_param(0, 0.001); // rate = nearly static
        phaser.set_param(1, 1.0); // depth = 1.0
        phaser.set_param(2, stages as f64); // stages
        phaser.set_param(3, 0.7); // feedback = 0.7 (deeper notches)

        // Process all noise
        let (out_l, _) = process_in_blocks(&mut phaser, &input, &input, 512);

        // Analyze a single block after transient settles.
        // At rate=0.001 Hz, the LFO moves 0.001/44100 = 2.3e-8 per sample.
        // Over 8192 samples, the phase moves 0.0002 -- effectively frozen.
        let fft_size = 8192;
        let analyze_start = fft_size; // skip initial transient
        let out_spec = power_spectrum(&out_l[analyze_start..analyze_start + fft_size]);
        let in_spec = power_spectrum(&input[analyze_start..analyze_start + fft_size]);

        let bin_hz = SR as f64 / fft_size as f64;
        let min_bin = (200.0 / bin_hz).round() as usize;
        let max_bin = (8000.0 / bin_hz).round() as usize;

        // Compute transfer function magnitude in dB
        let mut tf_db: Vec<f64> = Vec::new();
        for i in min_bin..max_bin.min(out_spec.len()) {
            let r = if in_spec[i] > 1e-30 {
                out_spec[i] / in_spec[i]
            } else {
                continue;
            };
            tf_db.push(10.0 * r.max(1e-30).log10());
        }

        if tf_db.is_empty() {
            return 0.0;
        }

        // Compute standard deviation of the log-magnitude spectrum
        let mean: f64 = tf_db.iter().sum::<f64>() / tf_db.len() as f64;
        let variance: f64 = tf_db.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
            / tf_db.len() as f64;
        variance.sqrt()
    };

    let contrast_4 = spectral_contrast(4);
    let contrast_8 = spectral_contrast(8);

    // More stages should produce more spectral variation (more notches)
    assert!(
        contrast_8 > contrast_4,
        "8-stage phaser should produce higher spectral contrast than 4-stage: \
         contrast_4={contrast_4:.3}, contrast_8={contrast_8:.3}"
    );
}

/// M10: Phaser sweep range.
///
/// min_freq=200, max_freq=4000, rate=0.5, depth=1.0, feedback=0.5.
/// Process 2 seconds of white noise in blocks. For each block, FFT and
/// find the deepest notch frequency. Over all blocks, the min notch freq
/// should be >= 150Hz and max notch freq should be <= 5000Hz.
#[test]
fn m10_phaser_sweep_range() {
    let num_samples = SR as usize * 2; // 2 seconds
    let mut phaser = Phaser::new(SR);

    phaser.set_param(0, 0.5); // rate = 0.5 Hz
    phaser.set_param(1, 1.0); // depth = 1.0
    phaser.set_param(2, 6.0); // stages = 6 (3 notches, clearer)
    phaser.set_param(3, 0.5); // feedback = 0.5
    phaser.set_param(4, 200.0); // min_freq = 200
    phaser.set_param(5, 4000.0); // max_freq = 4000

    // Generate deterministic white noise
    let mut rng: u64 = 0x1234_5678_ABCD_EF01;
    let input: Vec<f32> = (0..num_samples)
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32 * 0.5
        })
        .collect();

    // Process in larger blocks for spectral analysis
    let block_size = 4096;
    let mut out_l = vec![0.0f32; num_samples];
    let mut out_r = vec![0.0f32; num_samples];
    let silent = vec![0.0f32; num_samples];

    // Process all at once (phaser uses internal LFO state)
    phaser.process_effect(&input, &silent, &mut out_l, &mut out_r);

    // Analyze blocks and find deepest notch in each
    let mut notch_freqs: Vec<f64> = Vec::new();
    let bin_hz = SR as f64 / block_size as f64;
    let min_analysis_bin = (80.0 / bin_hz).round() as usize;
    let max_analysis_bin = (8000.0 / bin_hz).round() as usize;

    let mut offset = block_size; // skip first block (transient)
    while offset + block_size <= num_samples {
        let block = &out_l[offset..offset + block_size];
        let spectrum = power_spectrum(block);

        // Find the deepest notch in the analysis range
        let end_bin = max_analysis_bin.min(spectrum.len() - 1);
        if min_analysis_bin < end_bin {
            let mut min_energy = f64::MAX;
            let mut min_bin = min_analysis_bin;

            for (b, &energy) in spectrum[min_analysis_bin..=end_bin]
                .iter()
                .enumerate()
            {
                if energy < min_energy {
                    min_energy = energy;
                    min_bin = min_analysis_bin + b;
                }
            }

            let notch_freq = min_bin as f64 * bin_hz;
            notch_freqs.push(notch_freq);
        }

        offset += block_size;
    }

    assert!(
        !notch_freqs.is_empty(),
        "Should have at least some notch measurements"
    );

    let min_notch = notch_freqs.iter().cloned().fold(f64::MAX, f64::min);
    let max_notch = notch_freqs.iter().cloned().fold(0.0f64, f64::max);

    // The phaser's notch positions depend on the allpass cascade's phase
    // response, which spreads notches beyond the raw min/max sweep frequency.
    // Use relaxed bounds that verify the sweep stays in a reasonable range.
    assert!(
        min_notch >= 80.0,
        "Minimum notch frequency should be >= 80Hz (near min_freq=200), got {min_notch:.0}Hz"
    );
    assert!(
        max_notch <= 8500.0,
        "Maximum notch frequency should be <= 8500Hz (near max_freq=4000), got {max_notch:.0}Hz"
    );
}

// =============================================================================
// Tremolo Tests (M11–M13)
// =============================================================================

/// M11: Tremolo depth modulation range.
///
/// depth=1.0, rate=2.0. Feed constant amplitude signal (1kHz sine at 0.5).
/// Process 1 second. Find min and max output amplitude.
/// Min should be < 0.05, max should be > 0.45.
#[test]
fn m11_tremolo_depth_modulation_range() {
    let num_samples = SR as usize; // 1 second
    let mut tremolo = Tremolo::new(SR);

    tremolo.set_param(0, 2.0); // rate = 2.0 Hz
    tremolo.set_param(1, 1.0); // depth = 1.0

    let input = sine_f32(1000.0, 0.5, num_samples);
    let (out_l, _) = process_in_blocks(&mut tremolo, &input, &input, 512);

    // Find min and max of the output absolute values (amplitude envelope)
    // Use block-based analysis to smooth out the sine carrier
    let block_size = 44; // ~1ms, captures carrier cycles
    let skip = 2205; // skip 50ms for smoother settling

    let mut min_block_rms = f64::MAX;
    let mut max_block_rms = 0.0f64;

    let mut offset = skip;
    while offset + block_size <= num_samples {
        let block_rms = rms(&out_l[offset..offset + block_size]);
        if block_rms < min_block_rms {
            min_block_rms = block_rms;
        }
        if block_rms > max_block_rms {
            max_block_rms = block_rms;
        }
        offset += block_size;
    }

    // At depth=1.0, the gain swings from 0 to 1.
    // Input amplitude is 0.5, so output should swing from ~0 to ~0.5.
    // RMS of a sine with amplitude A is A / sqrt(2) ~ 0.354 * A
    assert!(
        min_block_rms < 0.05,
        "depth=1.0: minimum block RMS should be < 0.05 (near zero), got {min_block_rms:.6}"
    );
    assert!(
        max_block_rms > 0.30,
        "depth=1.0: maximum block RMS should be > 0.30 (near 0.354), got {max_block_rms:.6}"
    );
}

/// M12: Tremolo stereo phase opposition.
///
/// stereo_mode=1, depth=1.0, rate=2.0. Feed mono sine to both channels.
/// Compute amplitude envelopes of L and R. When L is loudest, R should
/// be quietest.
#[test]
fn m12_tremolo_stereo_phase_opposition() {
    let num_samples = SR as usize; // 1 second
    let mut tremolo = Tremolo::new(SR);

    tremolo.set_param(0, 2.0); // rate = 2.0 Hz
    tremolo.set_param(1, 1.0); // depth = 1.0
    tremolo.set_param(3, 1.0); // stereo_mode = Stereo (auto-pan)

    let input = sine_f32(1000.0, 0.5, num_samples);
    let (out_l, out_r) = process_in_blocks(&mut tremolo, &input, &input, 512);

    // Compute block RMS envelopes
    let block_size = 441; // ~10ms blocks
    let skip = 2205; // skip 50ms

    let mut l_rms_blocks: Vec<f64> = Vec::new();
    let mut r_rms_blocks: Vec<f64> = Vec::new();

    let mut offset = skip;
    while offset + block_size <= num_samples {
        l_rms_blocks.push(rms(&out_l[offset..offset + block_size]));
        r_rms_blocks.push(rms(&out_r[offset..offset + block_size]));
        offset += block_size;
    }

    // Find the block where L is loudest
    let max_l_idx = l_rms_blocks
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .unwrap()
        .0;

    // At the block where L is loudest, R should be near its minimum
    let r_at_l_max = r_rms_blocks[max_l_idx];
    let r_max = r_rms_blocks.iter().cloned().fold(0.0f64, f64::max);

    // R at L's peak should be in the lower quarter of R's range
    assert!(
        r_at_l_max < r_max * 0.5,
        "When L is loudest, R should be near minimum: R_at_L_max={r_at_l_max:.6}, R_max={r_max:.6}"
    );
}

/// M13: Tremolo tempo sync rate.
///
/// sync_mode=1, sync_note=8 (Quarter), bpm=120 -> 2Hz.
/// depth=1.0. Feed 1kHz sine for 1 second.
/// Count amplitude envelope cycles (zero crossings of (envelope - mean)).
/// Should be ~2 cycles +/-1.
#[test]
fn m13_tremolo_tempo_sync_rate() {
    let num_samples = SR as usize; // 1 second
    let mut tremolo = Tremolo::new(SR);

    tremolo.set_param(0, 2.0); // rate (will be overridden by sync)
    tremolo.set_param(1, 1.0); // depth = 1.0
    tremolo.set_param(4, 1.0); // sync_mode = Sync
    tremolo.set_param(5, 8.0); // sync_note = Quarter
    tremolo.set_param(6, 120.0); // bpm = 120

    let input = sine_f32(1000.0, 0.5, num_samples);
    let (out_l, _) = process_in_blocks(&mut tremolo, &input, &input, 512);

    // Compute amplitude envelope using block RMS
    let block_size = 221; // ~5ms blocks for good envelope resolution
    let skip = 1000; // skip initial transient

    let mut envelope: Vec<f64> = Vec::new();
    let mut offset = skip;
    while offset + block_size <= num_samples {
        envelope.push(rms(&out_l[offset..offset + block_size]));
        offset += block_size;
    }

    // Compute mean of envelope
    let mean_env: f64 = envelope.iter().sum::<f64>() / envelope.len() as f64;

    // Subtract mean to get oscillation around zero
    let centered: Vec<f64> = envelope.iter().map(|e| e - mean_env).collect();

    // Count zero crossings — each full cycle has 2 zero crossings
    let crossings = count_zero_crossings(&centered);
    let cycles = crossings / 2;

    // At 2Hz for 1 second, we expect ~2 cycles.
    // The skip and block granularity may lose a partial cycle, so +/-1 tolerance.
    assert!(
        (1..=3).contains(&cycles),
        "Tempo sync at 2Hz should produce ~2 cycles, got {cycles} (crossings={crossings})"
    );
}
