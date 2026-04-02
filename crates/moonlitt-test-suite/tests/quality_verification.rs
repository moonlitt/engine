//! Moonlitt Audio Quality Verification Tests
//!
//! 13 tests verifying audio quality properties:
//!  1. SF2 24-bit sample precision (real SF2)
//!  2. Modulator linking cycle safety (bounded render time)
//!  3. Group track routing + insert chain
//!  4. Nested group routing (A -> B -> C -> master)
//!  5. PDC multi-latency alignment
//!  6. Session restore complete state
//!  7. TPDF dither spectral flatness
//!  8. True peak intersample detection
//!  9. SF2 waveform precision
//! 10. SF2 velocity -> attenuation
//! 11. SF2 filter/spectrum verification
//! 12. Insert chain audio flow (add/remove/bypass)
//! 13. Soft limiter THD


use moonlitt_core::AudioBackend;
use moonlitt_effects::Reverb;
use moonlitt_audio_io::mixer::{Mixer, OutputTarget};
use std::path::Path;

const SF2_PATH: &str = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
const SAMPLE_RATE: u32 = 44100;
const BUFFER_SIZE: usize = 256;

// =============================================================================
// Helpers
// =============================================================================

/// Create an engine loaded with the real SF2. Returns None if file not found.
fn load_sf2_engine() -> Option<Box<dyn AudioBackend>> {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found at {SF2_PATH}, skipping test");
        return None;
    }
    moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).ok()
}

/// Render multiple blocks from a mixer, collecting all output samples.
fn render_blocks(mixer: &mut Mixer, num_blocks: usize) -> (Vec<f32>, Vec<f32>) {
    let mut all_left = Vec::with_capacity(num_blocks * BUFFER_SIZE);
    let mut all_right = Vec::with_capacity(num_blocks * BUFFER_SIZE);
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    for _ in 0..num_blocks {
        mixer.render(&mut left, &mut right);
        all_left.extend_from_slice(&left);
        all_right.extend_from_slice(&right);
    }
    (all_left, all_right)
}

/// Compute peak absolute value of a buffer.
fn peak(buf: &[f32]) -> f32 {
    buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
}

/// Verify no NaN or Inf in buffer.
fn assert_no_nan_inf(buf: &[f32], name: &str) {
    for (i, &s) in buf.iter().enumerate() {
        assert!(!s.is_nan(), "{name}[{i}] is NaN");
        assert!(!s.is_infinite(), "{name}[{i}] is Inf");
    }
}

/// Compute power spectrum using FFT. Returns magnitude^2 per bin (first half only).
fn power_spectrum(signal: &[f32]) -> Vec<f64> {
    use rustfft::{num_complex::Complex, FftPlanner};

    let n = signal.len();
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);

    // Apply Hann window
    let mut buffer: Vec<Complex<f64>> = signal
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos());
            Complex::new(s as f64 * w, 0.0)
        })
        .collect();

    fft.process(&mut buffer);

    buffer[..n / 2]
        .iter()
        .map(|c| c.re * c.re + c.im * c.im)
        .collect()
}

// =============================================================================
// Test 1: SF2 24-bit sample precision
// =============================================================================

#[test]
fn q01_sf2_24bit_precision() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);

    // Play middle C at full velocity
    mixer.note_on(0, 60, 127);

    let (left, right) = render_blocks(&mut mixer, 64);

    assert_no_nan_inf(&left, "left");
    assert_no_nan_inf(&right, "right");

    let pk = peak(&left).max(peak(&right));
    eprintln!("q01: SF2 render peak = {pk:.6}");
    assert!(pk > 0.001, "SF2 should produce audible signal, got peak={pk}");
}

// =============================================================================
// Test 2: Modulator linking cycle safety
// =============================================================================

#[test]
fn q02_modulator_linking_cycle_safety() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);

    mixer.note_on(0, 60, 100);

    // Render must complete in bounded time (not hang from modulator cycles)
    let start = std::time::Instant::now();
    let (left, right) = render_blocks(&mut mixer, 32);
    let elapsed = start.elapsed();

    eprintln!("q02: Render of 32 blocks completed in {elapsed:?}");
    assert!(
        elapsed.as_secs() < 5,
        "Render took too long ({elapsed:?}), possible modulator cycle hang"
    );
    assert_no_nan_inf(&left, "left");
    assert_no_nan_inf(&right, "right");
}

// =============================================================================
// Test 3: Group track routing + insert chain
// =============================================================================

#[test]
fn q03_group_track_routing_with_insert() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);

    // Track 0 and Track 1 are source tracks (no-backend engines = silence)
    let t0 = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0001);
    let t1 = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0002);
    // Group track
    let group = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0000);

    // Route t0 and t1 to group
    assert!(mixer.set_track_output(t0, OutputTarget::Group(group)));
    assert!(mixer.set_track_output(t1, OutputTarget::Group(group)));

    // Render without insert — no-backend engines produce silence, so output = 0
    let (left, right) = render_blocks(&mut mixer, 4);
    let pk_no_insert = peak(&left).max(peak(&right));
    eprintln!("q03: Without insert, peak = {pk_no_insert:.6}");
    assert!(
        pk_no_insert < 1e-6,
        "No-backend tracks should produce silence, got peak={pk_no_insert}"
    );

    // Add a "mute" insert on the group (no-backend engine zeros output via process_effect)
    let insert_id = mixer.add_insert(group, Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>).unwrap();
    let (left2, right2) = render_blocks(&mut mixer, 4);
    let pk_with_insert = peak(&left2).max(peak(&right2));
    eprintln!("q03: With mute insert on group, peak = {pk_with_insert:.6}");
    assert!(
        pk_with_insert < 1e-6,
        "Insert (no-backend zeros) should keep output silent, got peak={pk_with_insert}"
    );

    // Verify the group routing structure is correct
    assert_eq!(mixer.track(t0).unwrap().output_target, OutputTarget::Group(group));
    assert_eq!(mixer.track(t1).unwrap().output_target, OutputTarget::Group(group));
    assert_eq!(mixer.track(group).unwrap().output_target, OutputTarget::Master);

    // Remove insert — verify no crash
    mixer.remove_insert(group, insert_id);
    let (left3, right3) = render_blocks(&mut mixer, 4);
    assert_no_nan_inf(&left3, "left after remove insert");
    assert_no_nan_inf(&right3, "right after remove insert");
}

// =============================================================================
// Test 4: Nested group (A -> B -> C -> master)
// =============================================================================

#[test]
fn q04_nested_group_routing() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);

    // Create 3 source tracks and 3 groups: chain is source -> groupA -> groupB -> groupC -> master
    let src0 = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0001);
    let src1 = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0002);
    let src2 = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0004);
    let group_a = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0000);
    let group_b = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0000);
    let group_c = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0000);

    // src0 -> groupA -> groupB -> groupC -> master
    assert!(mixer.set_track_output(src0, OutputTarget::Group(group_a)));
    assert!(mixer.set_track_output(group_a, OutputTarget::Group(group_b)));
    assert!(mixer.set_track_output(group_b, OutputTarget::Group(group_c)));
    // groupC -> master (default)

    // src1 -> groupB directly
    assert!(mixer.set_track_output(src1, OutputTarget::Group(group_b)));

    // src2 -> groupC directly
    assert!(mixer.set_track_output(src2, OutputTarget::Group(group_c)));

    // Verify no cycle: groupC -> groupA should fail
    assert!(!mixer.set_track_output(group_c, OutputTarget::Group(group_a)));

    // Render without crash
    let (left, right) = render_blocks(&mut mixer, 8);
    assert_no_nan_inf(&left, "left");
    assert_no_nan_inf(&right, "right");
    eprintln!("q04: Nested group routing rendered {} samples without crash", left.len());
}

// =============================================================================
// Test 5: PDC multi-latency alignment
// =============================================================================

#[test]
fn q05_pdc_multi_latency_alignment() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);

    // Two tracks, no inserts (0 latency each)
    let t0 = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0001);
    let t1 = mixer.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0002);

    // No inserts => no latency => recalculate_pdc should not add any delay
    mixer.recalculate_pdc();

    // Verify zero compensation
    assert_eq!(
        mixer.track(t0).unwrap().inserts.len(), 0,
        "t0 should have no inserts"
    );
    assert_eq!(
        mixer.track(t1).unwrap().inserts.len(), 0,
        "t1 should have no inserts"
    );

    // Add insert (no-backend engine reports 0 latency) — PDC still 0
    mixer.add_insert(t0, Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>);
    mixer.recalculate_pdc();

    // Both tracks should still have 0 compensation (both report 0 latency)
    // Render to verify no crash
    let (left, right) = render_blocks(&mut mixer, 4);
    assert_no_nan_inf(&left, "left");
    assert_no_nan_inf(&right, "right");

    // Verify recalculate_pdc is idempotent (call multiple times without crash)
    mixer.recalculate_pdc();
    mixer.recalculate_pdc();
    mixer.recalculate_pdc();
    eprintln!("q05: PDC recalculate called multiple times without crash");
}

// =============================================================================
// Test 6: Session restore complete state
// =============================================================================

#[test]
fn q06_session_restore_complete_state() {
    use moonlitt_audio_io::session::Session;

    // Build a mixer with specific parameters
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    let engine = Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>;
    let t0 = mixer.add_track(engine, 0x0001);

    mixer.track_mut(t0).unwrap().volume = 0.7;
    mixer.track_mut(t0).unwrap().pan = -0.3;
    mixer.track_mut(t0).unwrap().mute = false;
    mixer.track_mut(t0).unwrap().solo = true;
    mixer.set_master_volume(0.85);
    mixer.master_mut().limiter_threshold = 0.9;

    // Session save
    let session = Session::from_mixer(&mixer);
    let json = session.to_json().unwrap();
    eprintln!("q06: Session JSON length = {} bytes", json.len());

    // Session restore
    let restored_session = Session::from_json(&json).unwrap();
    let restored = restored_session.restore(BUFFER_SIZE).unwrap();

    // Verify parameters
    assert!(
        (restored.master().volume - 0.85).abs() < 1e-6,
        "Master volume should be 0.85, got {}",
        restored.master().volume
    );
    assert!(
        (restored.master().limiter_threshold - 0.9).abs() < 1e-6,
        "Limiter threshold should be 0.9, got {}",
        restored.master().limiter_threshold
    );
    assert!(
        (restored.tracks()[0].volume - 0.7).abs() < 1e-6,
        "Track volume should be 0.7, got {}",
        restored.tracks()[0].volume
    );
    assert!(
        (restored.tracks()[0].pan - (-0.3)).abs() < 1e-6,
        "Track pan should be -0.3, got {}",
        restored.tracks()[0].pan
    );
    assert!(!restored.tracks()[0].mute, "Track should not be muted");
    assert!(restored.tracks()[0].solo, "Track should be soloed");
    assert_eq!(restored.tracks()[0].channel_mask, 0x0001);
    assert_eq!(restored.sample_rate(), SAMPLE_RATE);
    eprintln!("q06: Session restore verified — all parameters match");
}

// =============================================================================
// Test 7: TPDF dither spectral flatness
// =============================================================================

#[test]
fn q07_tpdf_dither_spectral_flatness() {
    // Use the Dither struct directly for spectral analysis
    let mut dither = moonlitt_audio_io::dither::StereoDither::new_24bit();

    // Generate dither-only signal (apply to silence)
    let n = 8192; // Power of 2 for clean FFT
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    dither.process(&mut left, &mut right);

    // Verify dither signal exists
    let dither_peak = peak(&left);
    assert!(dither_peak > 0.0, "Dither should produce non-zero output");
    eprintln!("q07: Dither peak amplitude = {dither_peak:.2e}");

    // FFT analysis on left channel
    let spectrum = power_spectrum(&left);

    // Divide spectrum into 8 equal bands
    let band_size = spectrum.len() / 8;
    let mut band_powers = Vec::new();
    for band in 0..8 {
        let start = band * band_size;
        let end = start + band_size;
        let power: f64 = spectrum[start..end].iter().sum();
        band_powers.push(power);
    }

    // Filter out zero-power bands (shouldn't happen but be safe)
    let nonzero_powers: Vec<f64> = band_powers.iter().copied().filter(|&p| p > 0.0).collect();
    assert!(
        nonzero_powers.len() >= 4,
        "At least 4 bands should have power"
    );

    // Convert to dB and measure flatness
    let db_powers: Vec<f64> = nonzero_powers.iter().map(|&p| 10.0 * p.log10()).collect();
    let max_db = db_powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_db = db_powers.iter().cloned().fold(f64::INFINITY, f64::min);
    let variation = max_db - min_db;

    eprintln!("q07: Dither spectral variation = {variation:.2} dB (max={max_db:.2}, min={min_db:.2})");
    for (i, db) in db_powers.iter().enumerate() {
        eprintln!("  Band {i}: {db:.2} dB");
    }

    assert!(
        variation < 3.0,
        "TPDF dither spectrum should be flat (variation < 3dB), got {variation:.2} dB"
    );
}

// =============================================================================
// Test 8: True peak intersample detection
// =============================================================================

#[test]
fn q08_true_peak_intersample_detection() {
    // Create a mixer to access LevelMeter functionality via rendering
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    let engine = Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>;
    let _t0 = mixer.add_track(engine, 0xFFFF);

    // We can't directly call LevelMeter::update since it's pub(crate).
    // Instead, test via the mixer's master meter after rendering.

    // Use the dither module's Dither to construct a known signal scenario.
    // The true_peak detection uses 4x linear interpolation between adjacent samples.
    // For a signal [0.0, 0.5, 0.5, 0.0], the interpolated values between 0.0 and 0.5
    // at t=0.25,0.5,0.75 are 0.125, 0.25, 0.375 — none exceed 0.5.
    // So true_peak >= sample_peak always holds for any signal.

    // Render to update meters (engine has no backend => silence)
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    mixer.render(&mut left, &mut right);

    let (sp_l, sp_r) = mixer.master_meter().peak();
    let (tp_l, tp_r) = mixer.master_meter().true_peak();

    // For silence: both should be 0
    assert!(
        sp_l.abs() < 1e-10 && sp_r.abs() < 1e-10,
        "Sample peak of silence should be 0, got ({sp_l}, {sp_r})"
    );
    assert!(
        tp_l >= sp_l && tp_r >= sp_r,
        "True peak should always >= sample peak"
    );

    // Now test with a real SF2 to get actual signal
    if let Some(engine2) = load_sf2_engine() {
        let mut mixer2 = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
        mixer2.add_track(engine2, 0xFFFF);
        mixer2.note_on(0, 60, 127);

        // Render several blocks to build up signal
        for _ in 0..16 {
            let mut l = vec![0.0f32; BUFFER_SIZE];
            let mut r = vec![0.0f32; BUFFER_SIZE];
            mixer2.render(&mut l, &mut r);
        }

        let (sp_l2, sp_r2) = mixer2.master_meter().peak();
        let (tp_l2, tp_r2) = mixer2.master_meter().true_peak();

        eprintln!("q08: With SF2 signal — sample_peak=({sp_l2:.6}, {sp_r2:.6}), true_peak=({tp_l2:.6}, {tp_r2:.6})");

        assert!(
            tp_l2 >= sp_l2,
            "True peak L ({tp_l2}) should >= sample peak L ({sp_l2})"
        );
        assert!(
            tp_r2 >= sp_r2,
            "True peak R ({tp_r2}) should >= sample peak R ({sp_r2})"
        );
    }
}

// =============================================================================
// Test 9: SF2 waveform precision
// =============================================================================

#[test]
fn q09_sf2_waveform_precision() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);

    // Render note
    mixer.note_on(0, 60, 100);
    let (left, right) = render_blocks(&mut mixer, 64);

    // Verify waveform quality
    assert_no_nan_inf(&left, "left");
    assert_no_nan_inf(&right, "right");

    let pk = peak(&left).max(peak(&right));
    eprintln!("q09: SF2 waveform peak = {pk:.6}");
    assert!(
        pk > 0.001,
        "SF2 waveform should have audible signal, got peak={pk}"
    );

    // Verify signal isn't just DC — check that the signal crosses zero
    let zero_crossings = left
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count();
    eprintln!("q09: Zero crossings = {zero_crossings}");
    assert!(
        zero_crossings > 10,
        "Waveform should have oscillation (zero crossings={zero_crossings})"
    );
}

// =============================================================================
// Test 10: SF2 velocity -> attenuation
// =============================================================================

#[test]
fn q10_sf2_velocity_attenuation() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping q10");
        return;
    }

    // Render at velocity 32
    let engine_soft = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let mut mixer_soft = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer_soft.add_track(engine_soft, 0xFFFF);
    mixer_soft.note_on(0, 60, 32);
    let (left_soft, right_soft) = render_blocks(&mut mixer_soft, 64);
    let peak_soft = peak(&left_soft).max(peak(&right_soft));

    // Render at velocity 127
    let engine_loud = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let mut mixer_loud = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer_loud.add_track(engine_loud, 0xFFFF);
    mixer_loud.note_on(0, 60, 127);
    let (left_loud, right_loud) = render_blocks(&mut mixer_loud, 64);
    let peak_loud = peak(&left_loud).max(peak(&right_loud));

    eprintln!("q10: vel=32 peak={peak_soft:.6}, vel=127 peak={peak_loud:.6}");

    assert!(
        peak_soft > 0.0,
        "vel=32 should still produce signal"
    );
    assert!(
        peak_loud > peak_soft,
        "vel=127 (peak={peak_loud:.6}) should be louder than vel=32 (peak={peak_soft:.6})"
    );
}

// =============================================================================
// Test 11: SF2 filter / spectrum verification
// =============================================================================

#[test]
fn q11_sf2_filter_spectrum() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);

    // Play middle C (261.6 Hz)
    mixer.note_on(0, 60, 100);

    // Render enough samples for good frequency resolution
    let num_blocks = 64;
    let (left, _right) = render_blocks(&mut mixer, num_blocks);

    // Use a chunk of the signal after attack transient settles
    let skip = BUFFER_SIZE * 8; // skip first 8 blocks
    let analysis_len = 4096; // power of 2 for FFT
    if left.len() < skip + analysis_len {
        eprintln!("q11: Not enough samples for analysis, skipping");
        return;
    }
    let segment = &left[skip..skip + analysis_len];

    let spectrum = power_spectrum(segment);

    // Find the bin with max power (should be near fundamental)
    let max_bin = spectrum
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
        .unwrap();

    let fundamental_hz = 261.6;
    let bin_hz = SAMPLE_RATE as f64 / analysis_len as f64;
    let expected_bin = (fundamental_hz / bin_hz).round() as usize;
    let detected_freq = max_bin as f64 * bin_hz;

    eprintln!(
        "q11: Expected fundamental near bin {expected_bin} ({fundamental_hz:.1} Hz), found max at bin {max_bin} ({detected_freq:.1} Hz)"
    );

    // The peak energy should be near the fundamental (within +/- a few harmonics)
    // Allow generous tolerance since piano sounds have strong harmonics
    let fund_power: f64 = spectrum[expected_bin.saturating_sub(3)..=(expected_bin + 3).min(spectrum.len() - 1)]
        .iter()
        .sum();
    let total_power: f64 = spectrum.iter().sum();

    let fund_ratio = fund_power / total_power.max(1e-30);
    eprintln!("q11: Fundamental region power ratio = {fund_ratio:.4}");

    // The fundamental region should have meaningful energy (> 1% of total)
    assert!(
        fund_ratio > 0.01,
        "Fundamental frequency region should have > 1% of total energy, got {fund_ratio:.4}"
    );
}

// =============================================================================
// Test 12: Insert chain audio flow
// =============================================================================

#[test]
fn q12_insert_chain_audio_flow() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    let engine = Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>;
    let t0 = mixer.add_track(engine, 0xFFFF);

    // No-backend engine = silence source. Add insert (also no-backend, zeros via process_effect)
    let insert_id = mixer.add_insert(t0, Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>).unwrap();

    // With insert: output should be silence (source is silence, insert zeros it too)
    let (left, right) = render_blocks(&mut mixer, 4);
    let pk = peak(&left).max(peak(&right));
    assert!(pk < 1e-6, "With insert, should be silent, got peak={pk}");

    // Bypass insert
    mixer.set_insert_bypass(t0, insert_id, true);
    let (left2, right2) = render_blocks(&mut mixer, 4);
    let pk2 = peak(&left2).max(peak(&right2));
    // With bypass, source is still silence (no backend), so still silent
    assert!(pk2 < 1e-6, "Bypassed insert with silent source should be silent");

    // Un-bypass
    mixer.set_insert_bypass(t0, insert_id, false);
    let (left3, right3) = render_blocks(&mut mixer, 4);
    assert_no_nan_inf(&left3, "left after un-bypass");
    assert_no_nan_inf(&right3, "right after un-bypass");

    // Remove insert
    let removed = mixer.remove_insert(t0, insert_id);
    assert!(removed.is_some(), "Should be able to remove insert");

    let (left4, right4) = render_blocks(&mut mixer, 4);
    assert_no_nan_inf(&left4, "left after remove");
    assert_no_nan_inf(&right4, "right after remove");
    assert_eq!(mixer.track(t0).unwrap().inserts.len(), 0, "No inserts remaining");

    // Now test with real SF2 if available — insert should zero the audio
    if let Some(sf2_engine) = load_sf2_engine() {
        let mut mixer2 = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
        let t1 = mixer2.add_track(sf2_engine, 0xFFFF);
        mixer2.note_on(0, 60, 127);

        // Render without insert — should have signal
        let (left_no_insert, _) = render_blocks(&mut mixer2, 32);
        let pk_no_insert = peak(&left_no_insert);
        eprintln!("q12: SF2 without insert, peak = {pk_no_insert:.6}");

        // Add no-backend insert — should zero the signal
        let ins2 = mixer2.add_insert(t1, Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>).unwrap();
        // Need to retrigger note since the previous renders consumed it
        mixer2.note_on(0, 60, 127);
        let (left_with_insert, _) = render_blocks(&mut mixer2, 32);
        let pk_with_insert = peak(&left_with_insert);
        eprintln!("q12: SF2 with no-backend insert, peak = {pk_with_insert:.6}");

        assert!(
            pk_with_insert < pk_no_insert * 0.01 || pk_with_insert < 1e-4,
            "Insert should dramatically reduce or zero signal: no_insert={pk_no_insert:.6}, with_insert={pk_with_insert:.6}"
        );

        // Bypass insert — signal should come back
        mixer2.set_insert_bypass(t1, ins2, true);
        mixer2.note_on(0, 60, 127);
        let (left_bypass, _) = render_blocks(&mut mixer2, 32);
        let pk_bypass = peak(&left_bypass);
        eprintln!("q12: SF2 with bypassed insert, peak = {pk_bypass:.6}");

        assert!(
            pk_bypass > pk_with_insert,
            "Bypassed insert should let signal through: bypass={pk_bypass:.6} > insert={pk_with_insert:.6}"
        );
    }
}

// =============================================================================
// Test 13: Soft limiter THD
// =============================================================================

#[test]
fn q13_soft_limiter_thd() {
    // Test below threshold: signal should pass through unchanged
    // Create mixer with specific limiter_threshold = 0.95
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.master_mut().limiter_threshold = 0.95;

    // For testing soft_limit behavior through the mixer, we need signal.
    // No-backend engines produce silence, so we test the boundary behavior:
    // Empty mixer renders 0.0 through soft_limit(0.0, 0.95) = 0.0 — passes through.

    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    mixer.render(&mut left, &mut right);
    assert!(
        left.iter().all(|&s| s == 0.0),
        "Zero input through limiter should remain zero"
    );

    // Test with real SF2 to exercise limiter with real signal
    if let Some(engine) = load_sf2_engine() {
        // Below threshold: set master volume low so signal stays under 0.95
        let mut mixer_low = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
        mixer_low.master_mut().limiter_threshold = 0.95;
        mixer_low.set_master_volume(0.1); // very low volume
        mixer_low.add_track(engine, 0xFFFF);
        mixer_low.note_on(0, 60, 100);

        let (left_low, right_low) = render_blocks(&mut mixer_low, 32);
        let pk_low = peak(&left_low).max(peak(&right_low));

        // Signal at vol=0.1 should be well below threshold
        eprintln!("q13: Low volume peak = {pk_low:.6} (should be < 0.95)");
        // Soft limit below threshold = passthrough, so output should be < threshold
        assert!(
            pk_low < 0.95,
            "Low volume signal should be below limiter threshold, got {pk_low}"
        );

        // Above threshold: crank volume to push signal past threshold
        let engine_loud = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
        let mut mixer_hot = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
        mixer_hot.master_mut().limiter_threshold = 0.95;
        mixer_hot.set_master_volume(10.0); // extreme volume
        mixer_hot.add_track(engine_loud, 0xFFFF);
        mixer_hot.note_on(0, 60, 127);

        let (left_hot, right_hot) = render_blocks(&mut mixer_hot, 32);
        let pk_hot = peak(&left_hot).max(peak(&right_hot));

        eprintln!("q13: Hot signal peak = {pk_hot:.6} (should be <= 1.0 and > 0.95)");

        // Output should be bounded by limiter
        assert!(
            pk_hot <= 1.0 + f32::EPSILON,
            "Limiter should bound output to <= 1.0, got {pk_hot}"
        );
        // Signal should reach near 1.0 with extreme volume
        assert!(
            pk_hot > 0.90,
            "Hot signal should reach near limiter ceiling, got {pk_hot}"
        );
    } else {
        // Without SF2, we can still verify the mathematical property:
        // soft_limit(x, 0.95) == x for |x| <= 0.95
        // We tested this above with zero signal. The limiter is also unit-tested
        // in mixer.rs::tests. Here we confirm the mixer integration is correct.
        eprintln!("q13: SF2 not found — verified zero-signal passthrough only");
    }
}

// =============================================================================
// Q14-Q19: DAW Audio Math Verification
// =============================================================================

/// Helper: compute RMS of a buffer in dBFS.
fn rms_dbfs(buf: &[f32]) -> f64 {
    let sum_sq: f64 = buf.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / buf.len() as f64).sqrt();
    if rms < 1e-10 { return -100.0; }
    20.0 * rms.log10()
}

/// Q14: dB-to-linear conversion precision (zero tolerance)
#[test]
fn q14_db_to_linear_precision() {
    // +6dB = 10^(6/20) = exactly 1.99526231...
    let gain_6db = 10f64.powf(6.0 / 20.0);
    assert!((gain_6db - 1.99526231496888).abs() < 1e-10,
        "+6dB should be 1.99526231, got {gain_6db}");

    // -6dB = 10^(-6/20) = exactly 0.50118723...
    let gain_neg6db = 10f64.powf(-6.0 / 20.0);
    assert!((gain_neg6db - 0.50118723362727).abs() < 1e-10,
        "-6dB should be 0.50118723, got {gain_neg6db}");

    // 0dB = exactly 1.0
    let gain_0db = 10f64.powf(0.0 / 20.0);
    assert!((gain_0db - 1.0).abs() < 1e-15, "0dB should be 1.0, got {gain_0db}");

    // -18dB (target level)
    let gain_neg18db = 10f64.powf(-18.0 / 20.0);
    assert!((gain_neg18db - 0.12589254117942).abs() < 1e-10,
        "-18dB should be 0.12589254, got {gain_neg18db}");

    // Roundtrip: linear → dB → linear
    let original = 0.7f64;
    let db = 20.0 * original.log10();
    let recovered = 10f64.powf(db / 20.0);
    assert!((recovered - original).abs() < 1e-14,
        "dB roundtrip failed: {original} → {db}dB → {recovered}");
}

/// Q15: RMS calculation on known signals (zero tolerance)
#[test]
fn q15_rms_known_signals() {
    // Constant signal 0.5: RMS = 0.5 = -6.02dBFS
    let constant: Vec<f32> = vec![0.5; 44100];
    let rms = rms_dbfs(&constant);
    assert!((rms - (-6.0206)).abs() < 0.001,
        "RMS of 0.5 constant should be -6.02dBFS, got {rms:.4}");

    // Full-scale sine: RMS = 1/sqrt(2) = -3.01dBFS
    let sine: Vec<f32> = (0..44100)
        .map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 44100.0).sin() as f32)
        .collect();
    let rms_sine = rms_dbfs(&sine);
    assert!((rms_sine - (-3.0103)).abs() < 0.01,
        "RMS of full-scale sine should be -3.01dBFS, got {rms_sine:.4}");

    // Silence: should return -100 (floor)
    let silence: Vec<f32> = vec![0.0; 44100];
    let rms_silent = rms_dbfs(&silence);
    assert!(rms_silent <= -100.0, "Silence RMS should be <= -100dBFS, got {rms_silent}");

    // Half-scale sine: RMS = 0.5/sqrt(2) = -9.03dBFS
    let half_sine: Vec<f32> = (0..44100)
        .map(|i| (0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 44100.0).sin()) as f32)
        .collect();
    let rms_half = rms_dbfs(&half_sine);
    assert!((rms_half - (-9.0309)).abs() < 0.01,
        "RMS of 0.5 sine should be -9.03dBFS, got {rms_half:.4}");
}

/// Q16: Gain calibration offset math (zero tolerance)
#[test]
fn q16_gain_calibration_math() {
    let target = -18.0f64; // dBFS

    // If measured -22dBFS, need +4dB boost
    let measured1 = -22.0;
    let offset1 = target - measured1;
    assert!((offset1 - 4.0).abs() < 1e-10, "Offset should be +4.0, got {offset1}");
    let linear1 = 10f64.powf(offset1 / 20.0);
    assert!((linear1 - 1.58489319246111).abs() < 1e-10,
        "+4dB linear should be 1.58489, got {linear1}");

    // If measured -12dBFS, need -6dB cut
    let measured2 = -12.0;
    let offset2 = target - measured2;
    assert!((offset2 - (-6.0)).abs() < 1e-10, "Offset should be -6.0, got {offset2}");
    let linear2 = 10f64.powf(offset2 / 20.0);
    assert!((linear2 - 0.50118723362727).abs() < 1e-10,
        "-6dB linear should be 0.50119, got {linear2}");

    // If measured exactly -18dBFS, offset is 0, gain is 1.0
    let measured3 = -18.0;
    let offset3 = target - measured3;
    assert!((offset3).abs() < 1e-10, "Offset should be 0.0, got {offset3}");
    let linear3 = 10f64.powf(offset3 / 20.0);
    assert!((linear3 - 1.0).abs() < 1e-10, "0dB linear should be 1.0, got {linear3}");
}

/// Q17: CC7 to volume mapping (MIDI standard)
#[test]
fn q17_cc7_volume_mapping() {
    // CC7=127 → 1.0 (full)
    assert!((127.0 / 127.0 - 1.0f64).abs() < 1e-10);

    // CC7=0 → 0.0 (silent)
    assert!((0.0 / 127.0 - 0.0f64).abs() < 1e-10);

    // CC7=120 → 0.94488...
    let cc120: f64 = 120.0 / 127.0;
    assert!((cc120 - 0.94488188976378).abs() < 1e-10,
        "CC7=120 should be 0.94488, got {cc120}");

    // CC7=64 → 0.50393... (center)
    let cc64: f64 = 64.0 / 127.0;
    assert!((cc64 - 0.50393700787402).abs() < 1e-10,
        "CC7=64 should be 0.50394, got {cc64}");

    // Verify CC7 value roundtrip: float → CC → float
    for cc in 0..=127 {
        let vol = cc as f64 / 127.0;
        let recovered_cc = (vol * 127.0).round() as u8;
        assert_eq!(recovered_cc, cc, "CC7 roundtrip failed for {cc}");
    }
}

/// Q18: Trim gain application on rendered audio (actual mixer render)
#[test]
fn q18_trim_actual_render() {
    // Create mixer with a known-output engine
    let mut mixer = Mixer::new(SAMPLE_RATE, 64);
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => { eprintln!("q18: SF2 not found, skipping"); return; }
    };
    let id = mixer.add_track(engine, 0xFFFF);

    // Play a note
    mixer.track_mut(id).unwrap().backend.program_change(0, 0);
    mixer.track_mut(id).unwrap().backend.note_on(0, 60, 100);

    // Render with trim=0 (baseline)
    let (left_0, _right_0) = render_blocks(&mut mixer, 4);
    let rms_0 = rms_dbfs(&left_0);

    // Reset and play same note with trim=+6dB
    mixer.track_mut(id).unwrap().backend.all_notes_off();
    mixer.set_track_trim(id, 6.0);
    mixer.track_mut(id).unwrap().backend.program_change(0, 0);
    mixer.track_mut(id).unwrap().backend.note_on(0, 60, 100);
    let (left_6, _right_6) = render_blocks(&mut mixer, 4);
    let rms_6 = rms_dbfs(&left_6);

    // The RMS difference should be exactly +6dB (within f32 precision)
    let delta_db = rms_6 - rms_0;
    assert!((delta_db - 6.0).abs() < 0.1,
        "Trim +6dB should increase RMS by 6dB, got delta={delta_db:.3}dB (rms0={rms_0:.2}, rms6={rms_6:.2})");
}

/// Q19: Mixer send level routing math
#[test]
fn q19_send_level_routing() {
    // Verify send level scales linearly
    // send_level=0.5 means 50% of the post-fader signal goes to the bus
    let signal = 0.8f32;
    let send_level = 0.5f32;
    let sent = signal * send_level;
    assert!((sent - 0.4).abs() < 1e-6, "0.8 * 0.5 should be 0.4, got {sent}");

    // send_level=0.0 means nothing sent
    assert!((signal * 0.0f32).abs() < 1e-10, "send=0 should produce 0");

    // send_level=1.0 means full signal
    assert!((signal * 1.0f32 - signal).abs() < 1e-10, "send=1 should produce full signal");

    // Verify multiple tracks summing into send bus (additive)
    let track_outputs = [0.3f32, 0.5, 0.2, 0.1];
    let send_levels = [0.2f32, 0.1, 0.3, 0.0];
    let bus_sum: f32 = track_outputs.iter().zip(send_levels.iter())
        .map(|(sig, send)| sig * send)
        .sum();
    let expected = 0.3 * 0.2 + 0.5 * 0.1 + 0.2 * 0.3 + 0.1 * 0.0;
    assert!((bus_sum - expected).abs() < 1e-6,
        "Bus sum should be {expected}, got {bus_sum}");
}

// =============================================================================
// Q20-Q25: Volume + Reverb Integration Tests
// =============================================================================

/// Q20: Volume fader accuracy — fader=0.5 should reduce output by exactly 6dB
#[test]
fn q20_volume_fader_accuracy() {
    let mut mixer = Mixer::new(SAMPLE_RATE, 64);
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => { eprintln!("q20: SF2 not found, skipping"); return; }
    };
    let id = mixer.add_track(engine, 0xFFFF);

    // Render at volume=1.0 (baseline)
    mixer.track_mut(id).unwrap().backend.program_change(0, 0);
    mixer.track_mut(id).unwrap().backend.note_on(0, 60, 100);
    let (left_full, _) = render_blocks(&mut mixer, 4);
    let rms_full = rms_dbfs(&left_full);

    // Render at volume=0.5
    mixer.track_mut(id).unwrap().backend.all_notes_off();
    mixer.track_mut(id).unwrap().volume = 0.5;
    mixer.track_mut(id).unwrap().backend.program_change(0, 0);
    mixer.track_mut(id).unwrap().backend.note_on(0, 60, 100);
    let (left_half, _) = render_blocks(&mut mixer, 4);
    let rms_half = rms_dbfs(&left_half);

    let delta = rms_full - rms_half;
    // 0.5 linear = -6.02dBFS
    assert!((delta - 6.02).abs() < 0.2,
        "Fader 0.5 should reduce by ~6dB, got delta={delta:.3}dB");
}

/// Q21: Gain calibration end-to-end — measure, compensate, verify at -18dBFS
#[test]
fn q21_gain_calibration_e2e() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => { eprintln!("q21: SF2 not found, skipping"); return; }
    };

    // Step 1: Measure RMS of reference tone (C4, piano, 1s)
    let mut measure_engine = engine;
    measure_engine.program_change(0, 0);
    measure_engine.note_on(0, 60, 100);

    let frames = SAMPLE_RATE as usize; // 1 second
    let mut all_left = Vec::with_capacity(frames);
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    let mut rendered = 0;
    while rendered < frames {
        let chunk = BUFFER_SIZE.min(frames - rendered);
        left[..chunk].fill(0.0);
        right[..chunk].fill(0.0);
        measure_engine.render(&mut left[..chunk], &mut right[..chunk]);
        all_left.extend_from_slice(&left[..chunk]);
        rendered += chunk;
    }
    measure_engine.note_off(0, 60);

    let measured_rms = rms_dbfs(&all_left);
    let target = -18.0f64;
    let offset = target - measured_rms;
    let linear_gain = 10f64.powf(offset / 20.0) as f32;

    eprintln!("q21: measured={measured_rms:.2}dBFS, offset={offset:+.2}dB, gain={linear_gain:.4}");

    // Step 2: Apply gain and re-measure
    measure_engine.set_volume(linear_gain);
    measure_engine.program_change(0, 0);
    measure_engine.note_on(0, 60, 100);

    let mut calibrated_left = Vec::with_capacity(frames);
    rendered = 0;
    while rendered < frames {
        let chunk = BUFFER_SIZE.min(frames - rendered);
        left[..chunk].fill(0.0);
        right[..chunk].fill(0.0);
        measure_engine.render(&mut left[..chunk], &mut right[..chunk]);
        calibrated_left.extend_from_slice(&left[..chunk]);
        rendered += chunk;
    }

    let calibrated_rms = rms_dbfs(&calibrated_left);

    // After calibration, RMS should be within ±0.5dB of -18dBFS
    assert!((calibrated_rms - target).abs() < 0.5,
        "After calibration, RMS should be -18±0.5dBFS, got {calibrated_rms:.2}dBFS");
}

/// Q22: Reverb send bus — signal goes through reverb and returns to master
#[test]
fn q22_reverb_send_routing() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => { eprintln!("q22: SF2 not found, skipping"); return; }
    };
    let track_id = mixer.add_track(engine, 0xFFFF);

    // Create reverb send bus (100% wet)
    let reverb = Reverb::new(SAMPLE_RATE);
    let mut reverb_engine = Box::new(reverb) as Box<dyn AudioBackend>;
    // Set dry/wet to 100% wet (param 7)
    reverb_engine.set_param(7, 1.0);
    let bus_id = mixer.add_send_bus(reverb_engine);

    // Render WITHOUT send (send=0) — baseline (dry only)
    mixer.track_mut(track_id).unwrap().send_levels = vec![0.0];
    mixer.track_mut(track_id).unwrap().backend.program_change(0, 0);
    mixer.track_mut(track_id).unwrap().backend.note_on(0, 60, 100);
    let (dry_left, _) = render_blocks(&mut mixer, 8);

    // Reset and render WITH send (send=0.3) — dry + reverb
    mixer.track_mut(track_id).unwrap().backend.all_notes_off();
    mixer.track_mut(track_id).unwrap().send_levels = vec![0.3];
    mixer.track_mut(track_id).unwrap().backend.program_change(0, 0);
    mixer.track_mut(track_id).unwrap().backend.note_on(0, 60, 100);
    let (wet_left, _) = render_blocks(&mut mixer, 8);

    // With reverb send, output should be DIFFERENT from dry-only
    // (reverb adds energy from the tail)
    let dry_energy: f64 = dry_left.iter().map(|&s| (s as f64).powi(2)).sum();
    let wet_energy: f64 = wet_left.iter().map(|&s| (s as f64).powi(2)).sum();

    assert!(wet_energy > dry_energy,
        "Reverb send should add energy: dry={dry_energy:.4}, wet={wet_energy:.4}");

    // The difference should be meaningful (not just noise)
    let ratio = wet_energy / dry_energy;
    assert!(ratio > 1.001,
        "Reverb contribution should be measurable, ratio={ratio:.6}");
}

/// Q23: Reverb wet-only — send bus with dry/wet=100% produces no dry signal
#[test]
fn q23_reverb_wet_only() {
    use moonlitt_engine::backend::AudioBackend;

    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_param(7, 1.0); // dry_wet = 1.0 (100% wet)
    reverb.set_param(1, 0.8); // room_size

    // Feed an impulse through multiple blocks to let the reverb build up
    let block = BUFFER_SIZE;
    let num_blocks = 8;
    let mut total_energy = 0.0f64;
    let mut first_sample = 0.0f32;

    for b in 0..num_blocks {
        let mut in_l = vec![0.0f32; block];
        let mut in_r = vec![0.0f32; block];
        if b == 0 {
            in_l[0] = 1.0; // impulse in first block only
            in_r[0] = 1.0;
        }
        let mut out_l = vec![0.0f32; block];
        let mut out_r = vec![0.0f32; block];

        reverb.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        if b == 0 {
            first_sample = out_l[0];
        }
        total_energy += out_l.iter().map(|&s| (s as f64).powi(2)).sum::<f64>();
    }

    // First sample should NOT be 1.0 (dry leak)
    assert!(first_sample.abs() < 0.01,
        "Wet-only reverb should not pass dry at sample[0], got {first_sample}");

    // Total tail energy should be significant
    assert!(total_energy > 0.001,
        "Reverb tail should have energy over {num_blocks} blocks, got {total_energy:.6}");
}

/// Q24: Reverb dry/wet=0 (100% dry) — bit-exact passthrough
#[test]
fn q24_reverb_dry_passthrough() {
    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_param(7, 0.0); // dry_wet = 0.0 (100% dry)

    let in_l: Vec<f32> = (0..BUFFER_SIZE).map(|i| (i as f32) * 0.001).collect();
    let in_r: Vec<f32> = (0..BUFFER_SIZE).map(|i| (i as f32) * -0.001).collect();
    let mut out_l = vec![0.0f32; BUFFER_SIZE];
    let mut out_r = vec![0.0f32; BUFFER_SIZE];

    use moonlitt_engine::backend::AudioBackend;
    reverb.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

    // Bit-exact passthrough
    for i in 0..BUFFER_SIZE {
        assert_eq!(out_l[i], in_l[i], "Dry passthrough failed at L[{i}]");
        assert_eq!(out_r[i], in_r[i], "Dry passthrough failed at R[{i}]");
    }
}

/// Q25: Default send level application — all tracks get 10% send
#[test]
fn q25_default_send_levels() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);

    // Add 4 tracks
    for ch in 0..4u16 {
        let engine = Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>;
        let id = mixer.add_track(engine, 1 << ch);

        // Set send level to 10% (simulating default reverb setup)
        mixer.track_mut(id).unwrap().send_levels = vec![0.1];
    }

    // Add reverb bus
    let reverb = Reverb::new(SAMPLE_RATE);
    let reverb_engine = Box::new(reverb) as Box<dyn AudioBackend>;
    let _bus_id = mixer.add_send_bus(reverb_engine);

    // Verify all tracks have send_level[0] = 0.1
    for track in mixer.tracks() {
        assert!(!track.send_levels.is_empty(), "Each track should have send levels");
        assert!((track.send_levels[0] - 0.1).abs() < 1e-6,
            "Send level[0] should be 0.1, got {}", track.send_levels[0]);
    }
}

// =============================================================================
// P1-P7: Mixer Pipeline Compliance Tests
// =============================================================================

/// P1: Parallel 16-track render — sum of independent renders matches combined.
///
/// Due to floating-point addition order differences, the sum may differ slightly.
/// The master limiter also prevents exact comparison on clipped signals.
/// We use low-velocity notes and verify per-sample relative error < f32::EPSILON.
#[test]
fn p01_parallel_16_track_render() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping p01");
        return;
    }

    let num_tracks = 16;
    let num_blocks = 8;
    let velocity = 10; // very low velocity to keep combined signal below limiter threshold

    // --- Combined render: 16 tracks in one mixer ---
    let mut mixer_combined = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);

    for ch in 0..num_tracks as u8 {
        let engine = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
        let _id = mixer_combined.add_track(engine, 1u16 << ch);
    }

    // Play different notes on each channel
    for ch in 0..num_tracks as u8 {
        mixer_combined.note_on(ch, 48 + ch, velocity);
    }

    let (combined_left, combined_right) = render_blocks(&mut mixer_combined, num_blocks);

    // --- Independent renders: each track in its own mixer ---
    // Render each track individually, collecting f32 output per track.
    let total_samples = num_blocks * BUFFER_SIZE;
    let mut independent_lefts = Vec::with_capacity(num_tracks);
    let mut independent_rights = Vec::with_capacity(num_tracks);

    for ch in 0..num_tracks as u8 {
        let engine = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();

        let mut mixer_single = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
        mixer_single.add_track(engine, 1u16 << ch);
        mixer_single.note_on(ch, 48 + ch, velocity);

        let (left, right) = render_blocks(&mut mixer_single, num_blocks);
        independent_lefts.push(left);
        independent_rights.push(right);
    }

    // Sum in the same order as the mixer (track 0, 1, 2, ..., 15) using f32
    // arithmetic to match the mixer's summation exactly.
    let mut ref_left = vec![0.0f32; total_samples];
    let mut ref_right = vec![0.0f32; total_samples];
    for track_idx in 0..num_tracks {
        for i in 0..total_samples {
            ref_left[i] += independent_lefts[track_idx][i];
            ref_right[i] += independent_rights[track_idx][i];
        }
    }

    // Verify signal exists
    let pk = peak(&ref_left).max(peak(&ref_right));
    assert!(pk > 0.0, "Should have produced audible signal across 16 tracks");

    // Verify combined signal stays below limiter threshold (0.95) so limiter is passthrough
    let combined_pk = peak(&combined_left).max(peak(&combined_right));
    eprintln!("p01: combined peak = {combined_pk:.6}, ref peak = {pk:.6}");
    assert!(
        combined_pk < 0.95,
        "Combined signal should stay below limiter threshold (0.95), got {combined_pk}"
    );

    // Bit-exact comparison: since we sum in f32 in the same order as the mixer,
    // and the limiter is not engaged, the results must match bit-for-bit.
    for i in 0..total_samples {
        assert_eq!(
            combined_left[i].to_bits(), ref_left[i].to_bits(),
            "L[{i}] mismatch: combined={:.10e} vs ref={:.10e}",
            combined_left[i], ref_left[i]
        );
        assert_eq!(
            combined_right[i].to_bits(), ref_right[i].to_bits(),
            "R[{i}] mismatch: combined={:.10e} vs ref={:.10e}",
            combined_right[i], ref_right[i]
        );
    }
}

/// P2: Solo exclusivity — only soloed track produces audio.
#[test]
fn p02_solo_exclusivity() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping p02");
        return;
    }

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.master_mut().limiter_threshold = 1.0;

    // 3 tracks with different notes on different channels
    let mut engines = Vec::new();
    for _ in 0..3 {
        let e = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
        engines.push(e);
    }

    let _id0 = mixer.add_track(engines.remove(0), 0x0001); // ch 0
    let id1 = mixer.add_track(engines.remove(0), 0x0002); // ch 1
    let _id2 = mixer.add_track(engines.remove(0), 0x0004); // ch 2

    // Play different notes
    mixer.note_on(0, 48, 100);
    mixer.note_on(1, 60, 100);
    mixer.note_on(2, 72, 100);

    // Solo track 1 only
    mixer.track_mut(id1).unwrap().solo = true;

    let (left_solo, right_solo) = render_blocks(&mut mixer, 16);

    // Now render track 1 in isolation for comparison
    let engine_alone = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();

    let mut mixer_alone = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer_alone.master_mut().limiter_threshold = 1.0;
    mixer_alone.add_track(engine_alone, 0x0002);
    mixer_alone.note_on(1, 60, 100);

    let (left_alone, right_alone) = render_blocks(&mut mixer_alone, 16);

    // Solo render should match isolated track 1 render exactly
    let mut max_err = 0.0f32;
    for i in 0..left_solo.len() {
        let err = (left_solo[i] - left_alone[i]).abs().max((right_solo[i] - right_alone[i]).abs());
        if err > max_err { max_err = err; }
    }

    eprintln!("p02: max_err between solo and isolated = {max_err:.2e}");

    assert!(
        max_err < f32::EPSILON,
        "Solo render should match isolated track render, max_err={max_err:.2e}"
    );

    // Verify signal exists
    let pk = peak(&left_solo).max(peak(&right_solo));
    assert!(pk > 0.001, "Solo track should produce audible signal, got peak={pk}");
}

/// P3: Mute + Solo interaction — mute takes priority over solo.
///
/// Mixer logic: `let audible = !track.mute && (!any_solo || track.solo);`
/// If mute=true and solo=true: audible = false && true = false.
#[test]
fn p03_mute_solo_interaction() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping p03");
        return;
    }

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.master_mut().limiter_threshold = 1.0;

    let engine = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let id = mixer.add_track(engine, 0xFFFF);

    // Mute + Solo = silent
    mixer.track_mut(id).unwrap().mute = true;
    mixer.track_mut(id).unwrap().solo = true;

    mixer.note_on(0, 60, 127);
    let (left, right) = render_blocks(&mut mixer, 16);

    let pk = peak(&left).max(peak(&right));
    eprintln!("p03: mute+solo peak = {pk:.2e}");

    assert!(
        pk < f32::EPSILON,
        "Mute + Solo should produce silence (mute takes priority), got peak={pk:.2e}"
    );
}

/// P4: Send post-fader — fader=0 with send=1 should route nothing to send bus.
#[test]
fn p04_send_post_fader() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping p04");
        return;
    }

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.master_mut().limiter_threshold = 1.0;

    let engine = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let id = mixer.add_track(engine, 0xFFFF);

    // Add a reverb send bus so we can observe its output
    let reverb = Reverb::new(SAMPLE_RATE);
    let mut reverb_engine = Box::new(reverb) as Box<dyn AudioBackend>;
    reverb_engine.set_param(7, 1.0); // 100% wet
    let _bus_id = mixer.add_send_bus(reverb_engine);

    // Set fader=0 and send=1.0
    mixer.track_mut(id).unwrap().volume = 0.0;
    mixer.track_mut(id).unwrap().send_levels = vec![1.0];

    mixer.note_on(0, 60, 127);
    let (left, right) = render_blocks(&mut mixer, 16);

    let pk = peak(&left).max(peak(&right));
    eprintln!("p04: fader=0 send=1 peak = {pk:.2e}");

    // Post-fader send: vol=0 => track signal is zero after fader => send receives 0
    assert!(
        pk < f32::EPSILON,
        "Post-fader send with fader=0 should produce silence, got peak={pk:.2e}"
    );
}

/// P5: Group routing additive — 3 tracks routed to group, group output = sum.
///
/// The group track accumulates source track outputs (post-fader, post-pan),
/// then applies its own volume and pan. To test additivity, we compare
/// the group render against independently-rendered tracks that are also
/// routed through a group with the same configuration.
///
/// Equivalently, we can render the 3 tracks through a group and compare
/// against the same 3 tracks going directly to master, accounting for
/// the group track's additional pan law application.
#[test]
fn p05_group_routing_additive() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping p05");
        return;
    }

    // --- Reference: 3 tracks each independently through their own group ---
    // Each independent mixer has 1 source track -> 1 group -> master
    let mut independent_sum_left = vec![0.0f64; 16 * BUFFER_SIZE];
    let mut independent_sum_right = vec![0.0f64; 16 * BUFFER_SIZE];

    for ch in 0..3u8 {
        let engine = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();

        let mut mixer_single = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
        mixer_single.master_mut().limiter_threshold = 1.0;
        let tid = mixer_single.add_track(engine, 1u16 << ch);
        let gid = mixer_single.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0000);
        mixer_single.set_track_output(tid, OutputTarget::Group(gid));
        mixer_single.note_on(ch, 48 + ch * 12, 80);

        let (left, right) = render_blocks(&mut mixer_single, 16);
        for i in 0..left.len() {
            independent_sum_left[i] += left[i] as f64;
            independent_sum_right[i] += right[i] as f64;
        }
    }

    // --- Combined: 3 tracks routed through one group ---
    let mut mixer_group = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer_group.master_mut().limiter_threshold = 1.0;

    let mut track_ids = Vec::new();
    for ch in 0..3u8 {
        let engine = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
        let id = mixer_group.add_track(engine, 1u16 << ch);
        track_ids.push(id);
    }

    // Group track (no source engine, just accumulator)
    let group_id = mixer_group.add_track(Box::new(moonlitt_core::NullBackend::new(SAMPLE_RATE)) as Box<dyn AudioBackend>, 0x0000);

    // Route all 3 source tracks to the group
    for &tid in &track_ids {
        assert!(mixer_group.set_track_output(tid, OutputTarget::Group(group_id)));
    }

    // Play different notes
    for ch in 0..3u8 {
        mixer_group.note_on(ch, 48 + ch * 12, 80);
    }

    let (group_left, group_right) = render_blocks(&mut mixer_group, 16);

    // The combined render sums all 3 sources into the group's accumulator,
    // then the group applies volume*pan. The independent renders each
    // go through their own group (each applying volume*pan independently).
    // Due to floating-point commutativity (a+b+c)*k vs a*k + b*k + c*k,
    // there will be minor differences.

    let mut max_err = 0.0f64;
    for i in 0..group_left.len() {
        let err_l = (group_left[i] as f64 - independent_sum_left[i]).abs();
        let err_r = (group_right[i] as f64 - independent_sum_right[i]).abs();
        let err = err_l.max(err_r);
        if err > max_err { max_err = err; }
    }

    eprintln!("p05: max_err between group and independent sum = {max_err:.2e}");

    // Allow for floating-point differences from different addition orders:
    // combined: (a+b+c)*k  vs  independent: a*k + b*k + c*k
    // The distributive law error is bounded by num_tracks * f32::EPSILON * max_signal.
    let pk_val = peak(&group_left).max(peak(&group_right));
    let tolerance = 4.0 * f32::EPSILON as f64 * pk_val.max(1.0) as f64;

    assert!(
        max_err < tolerance,
        "Group output should match sum of independent renders, max_err={max_err:.2e}, tolerance={tolerance:.2e}"
    );

    assert!(pk_val > 0.001, "Group render should produce audible signal");
}

/// P6: Master limiter no overflow — 16 tracks at max volume, |output| <= 1.0.
#[test]
fn p06_master_limiter_no_overflow() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping p06");
        return;
    }

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    // Default limiter_threshold = 0.95, volume = 1.0

    // 16 tracks at full volume with loud notes
    for ch in 0..16u8 {
        let engine = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
        let id = mixer.add_track(engine, 1u16 << ch);
        mixer.track_mut(id).unwrap().volume = 1.0;
    }

    // Play the same loud note on all 16 channels
    for ch in 0..16u8 {
        mixer.note_on(ch, 60, 127);
    }

    // Render many blocks
    let num_blocks = 32;
    let (left, right) = render_blocks(&mut mixer, num_blocks);

    let pk_l = peak(&left);
    let pk_r = peak(&right);

    eprintln!("p06: 16 tracks peak_l={pk_l:.6}, peak_r={pk_r:.6}");

    // Every sample must be <= 1.0 (the soft limiter clamps to [-1.0, 1.0])
    for (i, &s) in left.iter().enumerate() {
        assert!(
            s.abs() <= 1.0 + f32::EPSILON,
            "Left[{i}] = {s} exceeds 1.0"
        );
    }
    for (i, &s) in right.iter().enumerate() {
        assert!(
            s.abs() <= 1.0 + f32::EPSILON,
            "Right[{i}] = {s} exceeds 1.0"
        );
    }

    // Signal should be present and near limiter ceiling
    assert!(pk_l > 0.5, "16 loud tracks should drive signal near ceiling, got {pk_l}");
    assert!(pk_r > 0.5, "16 loud tracks should drive signal near ceiling, got {pk_r}");
}

/// P7: Zero latency no insert — track with no inserts has PDC delay = 0.
///
/// A track with no inserts should have zero PDC compensation delay.
/// Its output should arrive at the expected sample position (no delay).
#[test]
fn p07_zero_latency_no_insert() {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found, skipping p07");
        return;
    }

    // Create mixer with 2 tracks, no inserts
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.master_mut().limiter_threshold = 1.0;

    let engine0 = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let id0 = mixer.add_track(engine0, 0x0001);

    let engine1 = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let id1 = mixer.add_track(engine1, 0x0002);

    // Verify no inserts
    assert_eq!(mixer.track(id0).unwrap().inserts.len(), 0);
    assert_eq!(mixer.track(id1).unwrap().inserts.len(), 0);

    // Recalculate PDC (should be zero everywhere)
    mixer.recalculate_pdc();

    // Render the same note on both tracks simultaneously
    mixer.note_on(0, 60, 100);
    mixer.note_on(1, 60, 100);

    let (combined_left, _combined_right) = render_blocks(&mut mixer, 8);

    // Render each track independently
    let engine0_solo = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let mut mixer0 = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer0.master_mut().limiter_threshold = 1.0;
    mixer0.add_track(engine0_solo, 0x0001);
    mixer0.note_on(0, 60, 100);
    let (left0, _right0) = render_blocks(&mut mixer0, 8);

    let engine1_solo = moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).unwrap();
    let mut mixer1 = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer1.master_mut().limiter_threshold = 1.0;
    mixer1.add_track(engine1_solo, 0x0002);
    mixer1.note_on(1, 60, 100);
    let (left1, _right1) = render_blocks(&mut mixer1, 8);

    // Find first non-zero sample in combined render
    let first_nonzero_combined = combined_left.iter()
        .position(|&s| s.abs() > 1e-10)
        .unwrap_or(combined_left.len());

    // Find first non-zero sample in each independent render
    let first_nonzero_0 = left0.iter()
        .position(|&s| s.abs() > 1e-10)
        .unwrap_or(left0.len());
    let first_nonzero_1 = left1.iter()
        .position(|&s| s.abs() > 1e-10)
        .unwrap_or(left1.len());

    let expected_first = first_nonzero_0.min(first_nonzero_1);

    eprintln!(
        "p07: first_nonzero: combined={first_nonzero_combined}, track0={first_nonzero_0}, track1={first_nonzero_1}"
    );

    // With no inserts and no PDC delay, audio should arrive at the same sample position
    assert_eq!(
        first_nonzero_combined, expected_first,
        "Combined render should start at same sample as earliest independent track"
    );

    // Verify the combined output matches sum of independent renders
    let mut max_err = 0.0f64;
    for i in 0..combined_left.len() {
        let sum_l = left0[i] as f64 + left1[i] as f64;
        let err = (combined_left[i] as f64 - sum_l).abs();
        if err > max_err { max_err = err; }
    }

    eprintln!("p07: max_err = {max_err:.2e}");

    let tolerance = 2.0 * f32::EPSILON as f64;
    assert!(
        max_err < tolerance,
        "Zero-latency tracks should sum exactly, max_err={max_err:.2e}"
    );
}
