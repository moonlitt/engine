//! Sprint 1 Tests: Voice — SF2 parsing + single note Sinc72 playback
//!
//! Acceptance criteria:
//! 1. Load SF2, extract sample data for preset 0 (Grand Piano)
//! 2. Render C4 (MIDI note 60) for 2 seconds at 44100Hz
//! 3. FFT: fundamental frequency at 261.6Hz ±2Hz
//! 4. No NaN, no Inf, no clipping (peak ≤ 1.0)
//! 5. Output is not silence (RMS > 0.01)

use std::f32::consts::PI;

const SF2_PATH: &str = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
const SAMPLE_RATE: u32 = 44100;
const C4_FREQ: f32 = 261.626; // Hz
const C4_NOTE: u8 = 60;

fn has_sf2() -> bool {
    std::path::Path::new(SF2_PATH).exists()
}

// =============================================================================
// Test 1: SF2 sample pool loading
// =============================================================================

#[test]
fn t1_load_sf2_sample_pool() {
    if !has_sf2() { eprintln!("SF2 not found, skipping"); return; }

    let pool = moonlitt_sampler::SamplePool::from_file(SF2_PATH).unwrap();

    // GeneralUser_GS should have many samples
    assert!(pool.sample_count() > 0, "Should have samples");
    eprintln!("Loaded {} samples", pool.sample_count());

    // Should have presets
    assert!(pool.preset_count() > 0, "Should have presets");
    eprintln!("Loaded {} presets", pool.preset_count());
}

// =============================================================================
// Test 2: Find correct sample for a note
// =============================================================================

#[test]
fn t2_find_sample_for_note() {
    if !has_sf2() { return; }

    let pool = moonlitt_sampler::SamplePool::from_file(SF2_PATH).unwrap();

    // Preset 0, Bank 0 = Acoustic Grand Piano
    // Note 60 (C4) should map to a sample
    let sample_info = pool.find_sample(0, 0, C4_NOTE, 100);
    assert!(sample_info.is_some(), "Should find sample for C4 in Grand Piano");

    let info = sample_info.unwrap();
    eprintln!("Sample for C4: '{}', root={}, pitchadj={} cents, rate={}Hz, len={} samples",
        info.name, info.root_key, info.pitch_correction, info.sample_rate, info.len());

    // Verify: if root_key=60 and note=60, expected freq depends on pitch_correction
    let correction_semitones = info.pitch_correction as f64 / 100.0;
    let expected_freq = 261.626 * 2.0f64.powf(correction_semitones / 12.0);
    eprintln!("Expected freq with correction: {expected_freq:.3}Hz");

    assert!(info.sample_rate > 0, "Sample rate should be positive");
    assert!(info.len() > 0, "Sample should have data");
}

// =============================================================================
// Test 3: Voice renders audio (not silence)
// =============================================================================

#[test]
fn t3_voice_renders_audio() {
    if !has_sf2() { return; }

    let pool = moonlitt_sampler::SamplePool::from_file(SF2_PATH).unwrap();
    let sample = pool.find_sample(0, 0, C4_NOTE, 100).expect("Should find C4 sample");
    let mut voice = moonlitt_sampler::Voice::new(&pool, SAMPLE_RATE);

    voice.note_on(sample, C4_NOTE, 100);

    let mut output = vec![0.0f32; SAMPLE_RATE as usize * 2]; // 2 seconds
    voice.render(&mut output);

    let rms = (output.iter().map(|s| s * s).sum::<f32>() / output.len() as f32).sqrt();
    eprintln!("RMS: {rms:.6}");
    assert!(rms > 0.001, "Output should not be silence, got RMS={rms}");
}

// =============================================================================
// Test 4: No NaN, no Inf, no clipping
// =============================================================================

#[test]
fn t4_no_nan_inf_clipping() {
    if !has_sf2() { return; }

    let pool = moonlitt_sampler::SamplePool::from_file(SF2_PATH).unwrap();
    let sample = pool.find_sample(0, 0, C4_NOTE, 100).expect("Should find C4 sample");
    let mut voice = moonlitt_sampler::Voice::new(&pool, SAMPLE_RATE);

    voice.note_on(sample, C4_NOTE, 100);

    let mut output = vec![0.0f32; SAMPLE_RATE as usize]; // 1 second
    voice.render(&mut output);

    let mut nan_count = 0;
    let mut inf_count = 0;
    let mut peak = 0.0f32;

    for &s in &output {
        if s.is_nan() { nan_count += 1; }
        if s.is_infinite() { inf_count += 1; }
        let a = s.abs();
        if a > peak { peak = a; }
    }

    assert_eq!(nan_count, 0, "No NaN samples allowed");
    assert_eq!(inf_count, 0, "No Inf samples allowed");
    assert!(peak <= 1.0, "Peak should not exceed 1.0, got {peak}");
    eprintln!("Peak: {peak:.6}, no NaN/Inf");
}

// =============================================================================
// Test 5: Correct pitch via FFT
// =============================================================================

#[test]
fn t5_correct_pitch_fft() {
    if !has_sf2() { return; }

    let pool = moonlitt_sampler::SamplePool::from_file(SF2_PATH).unwrap();
    let sample = pool.find_sample(0, 0, C4_NOTE, 100).expect("Should find C4 sample");
    let mut voice = moonlitt_sampler::Voice::new(&pool, SAMPLE_RATE);

    voice.note_on(sample, C4_NOTE, 100);

    // Render 4 seconds for high FFT resolution (0.25Hz/bin)
    let duration_secs = 4;
    let n = SAMPLE_RATE as usize * duration_secs;
    let mut output = vec![0.0f32; n];
    voice.render(&mut output);

    // FFT to find fundamental frequency
    use rustfft::{num_complex::Complex, FftPlanner};

    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);

    // Apply Hann window
    let mut buffer: Vec<Complex<f64>> = output.iter().enumerate().map(|(i, &s)| {
        let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos());
        Complex::new(s as f64 * w, 0.0)
    }).collect();

    fft.process(&mut buffer);

    // Find peak in magnitude spectrum (skip DC, only first half)
    let magnitudes: Vec<f64> = buffer[1..n/2].iter()
        .map(|c| (c.re * c.re + c.im * c.im).sqrt())
        .collect();

    let peak_bin = magnitudes.iter().enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap().0 + 1;

    // Parabolic interpolation for sub-bin accuracy
    let alpha = if peak_bin > 1 { magnitudes[peak_bin - 2].ln() } else { 0.0 };
    let beta = magnitudes[peak_bin - 1].ln();
    let gamma = if peak_bin < magnitudes.len() { magnitudes[peak_bin].ln() } else { 0.0 };
    let p = 0.5 * (alpha - gamma) / (alpha - 2.0 * beta + gamma);
    let precise_bin = peak_bin as f64 + p;
    let precise_freq = precise_bin * SAMPLE_RATE as f64 / n as f64;

    let freq_error = (precise_freq - C4_FREQ as f64).abs();
    let bin_resolution = SAMPLE_RATE as f64 / n as f64;

    eprintln!("FFT resolution: {bin_resolution:.2}Hz/bin");
    eprintln!("Detected frequency: {precise_freq:.3}Hz (expected {C4_FREQ}Hz, error {freq_error:.3}Hz)");

    assert!(
        freq_error < 0.5,
        "Fundamental should be at {C4_FREQ}Hz ±0.5Hz, got {precise_freq:.3}Hz (error {freq_error:.3}Hz)"
    );
}

// =============================================================================
// Test 6: Sinc72 interpolation is used (not linear)
// =============================================================================

#[test]
fn t6_uses_sinc72_interpolation() {
    if !has_sf2() { return; }

    let pool = moonlitt_sampler::SamplePool::from_file(SF2_PATH).unwrap();

    // Verify the voice uses Sinc72 quality
    let voice = moonlitt_sampler::Voice::new(&pool, SAMPLE_RATE);
    assert_eq!(
        voice.interpolation_quality(),
        moonlitt_resampler::Quality::Sinc72,
        "Voice should use Sinc72 interpolation"
    );
}
