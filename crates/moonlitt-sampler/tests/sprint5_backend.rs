//! Sprint 5 Tests: AudioBackend integration
//!
//! Acceptance criteria:
//! 1. SamplerBackend implements AudioBackend trait
//! 2. Load SF2, note_on, render produces audio
//! 3. Multiple channels work (program change + note)
//! 4. All MIDI methods (CC, pitch bend, all_notes_off) don't panic
//! 5. Render C4 at correct pitch (FFT verified)
//! 6. Engine::load auto-detects and uses SamplerBackend for .sf2

const SF2_PATH: &str = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
const SAMPLE_RATE: u32 = 44100;

fn has_sf2() -> bool {
    std::path::Path::new(SF2_PATH).exists()
}

// =============================================================================
// Test 1: SamplerBackend loads SF2 and reports info
// =============================================================================

#[test]
fn t1_backend_load() {
    if !has_sf2() { return; }

    use moonlitt_sampler::backend::SamplerBackend;
    use moonlitt_engine::backend::AudioBackend;

    let mut backend = SamplerBackend::new(SAMPLE_RATE).unwrap();
    backend.load(SF2_PATH).unwrap();

    let info = backend.info();
    assert_eq!(info.name, "moonlitt-sampler");
    eprintln!("Backend: {}, type: {:?}", info.name, info.backend_type);
}

// =============================================================================
// Test 2: Note-on + render produces audio
// =============================================================================

#[test]
fn t2_note_on_render() {
    if !has_sf2() { return; }

    use moonlitt_sampler::backend::SamplerBackend;
    use moonlitt_engine::backend::AudioBackend;

    let mut backend = SamplerBackend::new(SAMPLE_RATE).unwrap();
    backend.load(SF2_PATH).unwrap();

    backend.note_on(0, 60, 100);

    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    backend.render(&mut left, &mut right);

    let rms = (left.iter().map(|s| s * s).sum::<f32>() / left.len() as f32).sqrt();
    eprintln!("RMS after note_on: {rms:.6}");
    assert!(rms > 0.001, "Should produce audio, got RMS={rms}");
}

// =============================================================================
// Test 3: Program change + multi-channel
// =============================================================================

#[test]
fn t3_program_change() {
    if !has_sf2() { return; }

    use moonlitt_sampler::backend::SamplerBackend;
    use moonlitt_engine::backend::AudioBackend;

    let mut backend = SamplerBackend::new(SAMPLE_RATE).unwrap();
    backend.load(SF2_PATH).unwrap();

    // Channel 0: Piano (program 0)
    backend.program_change(0, 0);
    backend.note_on(0, 60, 100);

    // Channel 1: Strings (program 48)
    backend.program_change(1, 48);
    backend.note_on(1, 60, 100);

    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    backend.render(&mut left, &mut right);

    let rms = (left.iter().map(|s| s * s).sum::<f32>() / left.len() as f32).sqrt();
    eprintln!("Multi-channel RMS: {rms:.6}");
    assert!(rms > 0.001, "Multi-channel should produce audio");
}

// =============================================================================
// Test 4: MIDI methods don't panic
// =============================================================================

#[test]
fn t4_midi_methods_safe() {
    if !has_sf2() { return; }

    use moonlitt_sampler::backend::SamplerBackend;
    use moonlitt_engine::backend::AudioBackend;

    let mut backend = SamplerBackend::new(SAMPLE_RATE).unwrap();
    backend.load(SF2_PATH).unwrap();

    // All methods should be safe even in edge cases
    backend.note_on(0, 60, 100);
    backend.note_off(0, 60);
    backend.cc(0, 64, 127); // sustain
    backend.cc(0, 7, 100);  // volume
    backend.pitch_bend(0, 0);
    backend.pitch_bend(0, 8191);
    backend.pitch_bend(0, -8192);
    backend.program_change(0, 0);
    backend.program_change(15, 127);
    backend.all_notes_off();

    // Render after all that
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    backend.render(&mut left, &mut right);

    // No panic = pass
    assert!(left.iter().all(|s| !s.is_nan()), "No NaN");
}

// =============================================================================
// Test 5: Correct pitch via FFT
// =============================================================================

#[test]
fn t5_correct_pitch() {
    if !has_sf2() { return; }

    use moonlitt_sampler::backend::SamplerBackend;
    use moonlitt_engine::backend::AudioBackend;
    use rustfft::{num_complex::Complex, FftPlanner};

    let mut backend = SamplerBackend::new(SAMPLE_RATE).unwrap();
    backend.load(SF2_PATH).unwrap();
    backend.note_on(0, 60, 100);

    let n = SAMPLE_RATE as usize * 2; // 2 seconds
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    backend.render(&mut left, &mut right);

    // FFT with Hann window
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    let mut buffer: Vec<Complex<f64>> = left.iter().enumerate().map(|(i, &s)| {
        let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos());
        Complex::new(s as f64 * w, 0.0)
    }).collect();
    fft.process(&mut buffer);

    let magnitudes: Vec<f64> = buffer[1..n/2].iter()
        .map(|c| (c.re * c.re + c.im * c.im).sqrt())
        .collect();

    let peak_bin = magnitudes.iter().enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap().0 + 1;

    // Parabolic interpolation
    let alpha = if peak_bin > 1 { magnitudes[peak_bin - 2].ln() } else { 0.0 };
    let beta = magnitudes[peak_bin - 1].ln();
    let gamma = if peak_bin < magnitudes.len() { magnitudes[peak_bin].ln() } else { 0.0 };
    let denom = alpha - 2.0 * beta + gamma;
    let p = if denom.abs() > 1e-10 { 0.5 * (alpha - gamma) / denom } else { 0.0 };
    let precise_freq = (peak_bin as f64 + p) * SAMPLE_RATE as f64 / n as f64;

    let expected = 261.626;
    let error = (precise_freq - expected).abs();
    eprintln!("Backend pitch: {precise_freq:.3}Hz (expected {expected}Hz, error {error:.3}Hz)");

    assert!(error < 1.0, "Pitch error should be < 1Hz, got {error:.3}Hz");
}

// =============================================================================
// Test 6: Volume control
// =============================================================================

#[test]
fn t6_volume_control() {
    if !has_sf2() { return; }

    use moonlitt_sampler::backend::SamplerBackend;
    use moonlitt_engine::backend::AudioBackend;

    let mut backend = SamplerBackend::new(SAMPLE_RATE).unwrap();
    backend.load(SF2_PATH).unwrap();

    // Full volume
    backend.set_volume(1.0);
    backend.note_on(0, 60, 100);
    let mut left1 = vec![0.0f32; 4096];
    let mut right1 = vec![0.0f32; 4096];
    backend.render(&mut left1, &mut right1);
    let rms_full = (left1.iter().map(|s| s * s).sum::<f32>() / left1.len() as f32).sqrt();

    // Half volume
    backend.all_notes_off();
    backend.set_volume(0.5);
    backend.note_on(0, 60, 100);
    let mut left2 = vec![0.0f32; 4096];
    let mut right2 = vec![0.0f32; 4096];
    backend.render(&mut left2, &mut right2);
    let rms_half = (left2.iter().map(|s| s * s).sum::<f32>() / left2.len() as f32).sqrt();

    eprintln!("Full volume RMS: {rms_full:.4}, Half volume RMS: {rms_half:.4}");
    assert!(rms_half < rms_full, "Half volume should be quieter");
}
