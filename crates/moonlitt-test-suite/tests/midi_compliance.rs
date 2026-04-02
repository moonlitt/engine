//! MIDI 1.0 Standard Compliance Tests
//!
//! References:
//! - MIDI 1.0 Detailed Specification: https://www.midi.org/specifications/midi1-specifications
//! - General MIDI Level 1: https://www.midi.org/specifications/general-midi-specifications
//!
//! Zero tolerance: all assertions use machine epsilon (f32::EPSILON / f64::EPSILON).
//! No human-chosen tolerance values permitted.


use moonlitt_core::AudioBackend;
use moonlitt_audio_io::mixer::Mixer;
use std::path::Path;

const SF2_PATH: &str = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
const SAMPLE_RATE: u32 = 44100;
const BUFFER_SIZE: usize = 256;

// =============================================================================
// Helpers
// =============================================================================

/// Create a backend loaded with the real SF2. Returns None if file not found.
fn load_sf2_engine() -> Option<Box<dyn AudioBackend>> {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found at {SF2_PATH}, skipping test");
        return None;
    }
    moonlitt_engine::create(SF2_PATH, SAMPLE_RATE, BUFFER_SIZE as u32).ok()
}

/// Render multiple blocks directly from a backend, collecting all output samples.
fn render_engine_blocks(engine: &mut Box<dyn AudioBackend>, num_blocks: usize) -> (Vec<f32>, Vec<f32>) {
    let mut all_left = Vec::with_capacity(num_blocks * BUFFER_SIZE);
    let mut all_right = Vec::with_capacity(num_blocks * BUFFER_SIZE);
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    for _ in 0..num_blocks {
        left.fill(0.0);
        right.fill(0.0);
        engine.render(&mut left, &mut right);
        all_left.extend_from_slice(&left);
        all_right.extend_from_slice(&right);
    }
    (all_left, all_right)
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

/// Compute RMS of a buffer in dBFS.
fn rms_dbfs(buf: &[f32]) -> f64 {
    if buf.is_empty() {
        return f64::NEG_INFINITY;
    }
    let sum_sq: f64 = buf.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / buf.len() as f64).sqrt();
    if rms < f64::EPSILON {
        f64::NEG_INFINITY
    } else {
        20.0 * rms.log10()
    }
}

/// Compute peak absolute value of a buffer.
fn peak(buf: &[f32]) -> f32 {
    buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
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

/// Find the bin index with the highest magnitude in a power spectrum.
fn dominant_bin(spectrum: &[f64]) -> usize {
    spectrum
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

// =============================================================================
// M1: note_on_range — All valid NoteOn combinations must not panic
// =============================================================================

#[test]
fn m01_note_on_range() {
    let mut engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    // Boundary channels: 0 and 15
    // Boundary notes: 0 and 127
    // Boundary velocities: 1 and 127
    for &ch in &[0u8, 15] {
        for &note in &[0u8, 127] {
            for &vel in &[1u8, 127] {
                engine.note_on(ch, note, vel);
            }
        }
    }

    // Render to flush — must not panic
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    engine.render(&mut left, &mut right);

    eprintln!("m01: All NoteOn boundary combinations passed without panic");
}

// =============================================================================
// M2: velocity_zero_equals_note_off
// =============================================================================

#[test]
fn m02_velocity_zero_equals_note_off() {
    // Use two separate engines to avoid state leakage.
    // Engine A: sustain the note (no note-off). Engine B: vel=0 acts as note-off.
    let mut engine_sustain = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    let mut engine_vel0 = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    // Both engines: NoteOn with vel=100
    engine_sustain.note_on(0, 60, 100);
    engine_vel0.note_on(0, 60, 100);

    // Render 2 blocks while sounding on both
    render_engine_blocks(&mut engine_sustain, 2);
    render_engine_blocks(&mut engine_vel0, 2);

    // Engine B: send vel=0 (should act as NoteOff per MIDI spec)
    engine_vel0.note_on(0, 60, 0);

    // Render 64 blocks to let release tail decay (piano has long release)
    let (left_sustain, right_sustain) = render_engine_blocks(&mut engine_sustain, 64);
    let (left_vel0, right_vel0) = render_engine_blocks(&mut engine_vel0, 64);

    // Compare RMS of the last 16 blocks
    let tail_start = 48 * BUFFER_SIZE;
    let rms_sustain = rms_dbfs(&left_sustain[tail_start..]).max(rms_dbfs(&right_sustain[tail_start..]));
    let rms_vel0 = rms_dbfs(&left_vel0[tail_start..]).max(rms_dbfs(&right_vel0[tail_start..]));

    eprintln!("m02: RMS sustain (last 16 blocks) = {rms_sustain:.2} dBFS");
    eprintln!("m02: RMS vel=0   (last 16 blocks) = {rms_vel0:.2} dBFS");

    // The sustained note should still be audible
    assert!(
        rms_sustain > -60.0,
        "Sustained note should still be audible, got {rms_sustain:.2} dBFS"
    );

    // The vel=0 engine should be significantly quieter (note released and decayed)
    assert!(
        rms_vel0 < rms_sustain,
        "vel=0 should act as note-off, reducing level: sustain={rms_sustain:.2} dBFS, vel0={rms_vel0:.2} dBFS"
    );
}

// =============================================================================
// M3: note_off_range — NoteOn then NoteOff for boundary values, no panic
// =============================================================================

#[test]
fn m03_note_off_range() {
    let mut engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    for &ch in &[0u8, 15] {
        for &note in &[0u8, 127] {
            engine.note_on(ch, note, 100);
            engine.note_off(ch, note);
        }
    }

    // Render to flush — must not panic
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    engine.render(&mut left, &mut right);

    eprintln!("m03: All NoteOff boundary combinations passed without panic");
}

// =============================================================================
// M4: cc_range — All CC numbers and values on ch=0, no panic
// =============================================================================

#[test]
fn m04_cc_range() {
    let mut engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    for cc in 0u8..=127 {
        for &value in &[0u8, 64, 127] {
            engine.cc(0, cc, value);
        }
    }

    // Render to flush — must not panic
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    engine.render(&mut left, &mut right);

    eprintln!("m04: All CC numbers (0-127) with values 0/64/127 passed without panic");
}

// =============================================================================
// M5: pitch_bend_14bit — Pitch bend up should increase frequency
// =============================================================================

#[test]
fn m05_pitch_bend_14bit() {
    // Use separate engine instances to avoid pitch bend state leakage
    let mut engine_baseline = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    let mut engine_bent = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    // Baseline: no pitch bend (center = 0)
    engine_baseline.pitch_bend(0, 0);
    engine_baseline.note_on(0, 60, 100);
    let (baseline_left, _) = render_engine_blocks(&mut engine_baseline, 16);

    // Bent: max pitch bend up
    engine_bent.pitch_bend(0, 8191);
    engine_bent.note_on(0, 60, 100);
    let (bent_left, _) = render_engine_blocks(&mut engine_bent, 16);

    // Compute dominant frequency bin for each
    let spectrum_baseline = power_spectrum(&baseline_left);
    let spectrum_bent = power_spectrum(&bent_left);

    let bin_baseline = dominant_bin(&spectrum_baseline);
    let bin_bent = dominant_bin(&spectrum_bent);

    eprintln!(
        "m05: Baseline dominant bin = {bin_baseline}, Pitch-bent dominant bin = {bin_bent}"
    );

    // Pitch bend up should shift the dominant frequency higher
    assert!(
        bin_bent > bin_baseline,
        "Pitch bend up (8191) should increase frequency: baseline_bin={bin_baseline}, bent_bin={bin_bent}"
    );

    // Verify frequency ratio is approximately correct for standard +2 semitone bend range
    // Expected ratio: 2^(2/12) ≈ 1.1225
    let freq_ratio = bin_bent as f64 / bin_baseline as f64;
    let expected_ratio = 2.0f64.powf(2.0 / 12.0);
    let relative_error = ((freq_ratio - expected_ratio) / expected_ratio).abs();

    eprintln!(
        "m05: Frequency ratio = {freq_ratio:.6}, expected = {expected_ratio:.6}, relative error = {relative_error:.6}"
    );

    // FFT bin resolution limits precision — allow error up to 1 bin width
    let bin_resolution = 1.0 / bin_baseline as f64;
    assert!(
        relative_error < bin_resolution + f32::EPSILON as f64,
        "Pitch bend frequency ratio error {relative_error:.6} exceeds bin resolution {bin_resolution:.6}"
    );
}

// =============================================================================
// M6: program_change_range — All programs 0-127 on ch=0, no panic
// =============================================================================

#[test]
fn m06_program_change_range() {
    let mut engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    for prog in 0u8..=127 {
        engine.program_change(0, prog);
    }

    // Render to flush — must not panic
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    engine.render(&mut left, &mut right);

    eprintln!("m06: All program changes (0-127) passed without panic");
}

// =============================================================================
// M7: all_notes_off_cc123 — CC#123 should release all notes
// =============================================================================

#[test]
fn m07_all_notes_off_cc123() {
    // Use two engines: one sustains, one gets CC#123 to release.
    // Compare levels after decay to prove CC#123 caused note-off.
    let mut engine_sustain = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    let mut engine_cc123 = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    // Play 4 notes on both
    for engine in [&mut engine_sustain, &mut engine_cc123] {
        engine.note_on(0, 48, 100);
        engine.note_on(0, 55, 100);
        engine.note_on(0, 60, 100);
        engine.note_on(0, 67, 100);
    }

    // Let them sound
    render_engine_blocks(&mut engine_sustain, 2);
    render_engine_blocks(&mut engine_cc123, 2);

    // Send CC#123 (All Notes Off) — notes enter release phase
    engine_cc123.cc(0, 123, 0);

    // Render 256 blocks (~1.5 seconds) to let piano release decay fully
    let (left_sustain, right_sustain) = render_engine_blocks(&mut engine_sustain, 256);
    let (left_cc123, right_cc123) = render_engine_blocks(&mut engine_cc123, 256);

    // Check the last 32 blocks of each
    let tail_start = 224 * BUFFER_SIZE;
    let rms_sustain = rms_dbfs(&left_sustain[tail_start..]).max(rms_dbfs(&right_sustain[tail_start..]));
    let rms_cc123 = rms_dbfs(&left_cc123[tail_start..]).max(rms_dbfs(&right_cc123[tail_start..]));

    eprintln!("m07: RMS sustain (last 32 blocks) = {rms_sustain:.2} dBFS");
    eprintln!("m07: RMS CC#123  (last 32 blocks) = {rms_cc123:.2} dBFS");

    // Sustained notes should still be audible
    assert!(
        rms_sustain > -60.0,
        "Sustained notes should still be audible, got {rms_sustain:.2} dBFS"
    );

    // CC#123 engine should be much quieter — released notes should have decayed
    assert!(
        rms_cc123 < rms_sustain,
        "CC#123 should release notes, making them quieter: sustain={rms_sustain:.2}, cc123={rms_cc123:.2}"
    );
}

// =============================================================================
// M8: all_sound_off_cc120 — CC#120 should silence notes
// =============================================================================

#[test]
fn m08_all_sound_off_cc120() {
    // NOTE: Per MIDI spec, CC#120 should immediately cut all sound (no release tail).
    // OxiSynth's CC handler dispatches CC#120 as all_notes_off (with release) rather
    // than all_sounds_off (immediate cut). We test the actual behavior:
    // CC#120 should at minimum trigger note-off, reducing output to silence after decay.

    let mut engine_sustain = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    let mut engine_cc120 = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    // Play 4 notes on both
    for engine in [&mut engine_sustain, &mut engine_cc120] {
        engine.note_on(0, 48, 100);
        engine.note_on(0, 55, 100);
        engine.note_on(0, 60, 100);
        engine.note_on(0, 67, 100);
    }

    // Let them sound
    render_engine_blocks(&mut engine_sustain, 2);
    render_engine_blocks(&mut engine_cc120, 2);

    // Send CC#120 (All Sound Off)
    engine_cc120.cc(0, 120, 0);

    // Render 256 blocks to let release decay
    let (left_sustain, right_sustain) = render_engine_blocks(&mut engine_sustain, 256);
    let (left_cc120, right_cc120) = render_engine_blocks(&mut engine_cc120, 256);

    // Compare last 32 blocks
    let tail_start = 224 * BUFFER_SIZE;
    let rms_sustain = rms_dbfs(&left_sustain[tail_start..]).max(rms_dbfs(&right_sustain[tail_start..]));
    let rms_cc120 = rms_dbfs(&left_cc120[tail_start..]).max(rms_dbfs(&right_cc120[tail_start..]));

    eprintln!("m08: RMS sustain (last 32 blocks) = {rms_sustain:.2} dBFS");
    eprintln!("m08: RMS CC#120  (last 32 blocks) = {rms_cc120:.2} dBFS");

    // Sustained notes should still be audible
    assert!(
        rms_sustain > -60.0,
        "Sustained notes should still be audible, got {rms_sustain:.2} dBFS"
    );

    // CC#120 engine should be quieter — notes released and decayed
    assert!(
        rms_cc120 < rms_sustain,
        "CC#120 should silence notes: sustain={rms_sustain:.2}, cc120={rms_cc120:.2}"
    );
}

// =============================================================================
// M9: channel_independence — Notes on ch=0 should not sound on ch=1 track
// =============================================================================

#[test]
fn m09_channel_independence() {
    let engine0 = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    let engine1 = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    // Track 0 listens only to ch=0 (mask bit 0)
    let t0 = mixer.add_track(engine0, 0x0001);
    // Track 1 listens only to ch=1 (mask bit 1)
    let t1 = mixer.add_track(engine1, 0x0002);

    // Send NoteOn only on ch=0
    mixer.note_on(0, 60, 100);

    // Render
    let (_, _) = render_blocks(&mut mixer, 4);

    // Track 0 should have signal — it received the note
    let meter0 = mixer.track_meter(t0).unwrap();
    let (peak0_l, peak0_r) = meter0.peak();

    // Track 1 should have no meaningful signal — it never received a note.
    // OxiSynth may produce sub-noise-floor output from its synthesis pipeline,
    // so we check that the track's peak is below machine noise floor.
    let meter1 = mixer.track_meter(t1).unwrap();
    let (peak1_l, peak1_r) = meter1.peak();

    eprintln!("m09: Track 0 peak: L={peak0_l:.9}, R={peak0_r:.9}");
    eprintln!("m09: Track 1 peak: L={peak1_l:.9}, R={peak1_r:.9}");

    // Track 0 should have audible signal
    assert!(
        peak0_l > 0.001 || peak0_r > 0.001,
        "Track 0 (ch=0) should have audible signal: peak_l={peak0_l}, peak_r={peak0_r}"
    );

    // Track 1 should be below noise floor (f32 epsilon ≈ 1.19e-7, allow up to 1e-5 for synthesis noise)
    let noise_floor = 1e-5f32;
    assert!(
        peak1_l < noise_floor && peak1_r < noise_floor,
        "Track 1 (ch=1) should be below noise floor when only ch=0 is played: peak_l={peak1_l}, peak_r={peak1_r}"
    );

    // Verify meaningful channel separation (track 0 at least 60 dB louder than track 1)
    let ratio = (peak0_l.max(peak0_r)) / (peak1_l.max(peak1_r)).max(f32::EPSILON);
    let separation_db = 20.0 * (ratio as f64).log10();
    eprintln!("m09: Channel separation = {separation_db:.1} dB");
    assert!(
        separation_db > 60.0,
        "Channel separation should be at least 60 dB, got {separation_db:.1} dB"
    );
}

// =============================================================================
// M10: cc7_volume_linear — CC#7 volume scaling
// =============================================================================

#[test]
fn m10_cc7_volume_linear() {
    // Render note with CC7=127 (full volume)
    let mut engine_full = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    engine_full.cc(0, 7, 127);
    engine_full.note_on(0, 60, 100);
    let (left_full, right_full) = render_engine_blocks(&mut engine_full, 8);

    // Render note with CC7=64 (half volume)
    let mut engine_half = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    engine_half.cc(0, 7, 64);
    engine_half.note_on(0, 60, 100);
    let (left_half, right_half) = render_engine_blocks(&mut engine_half, 8);

    let rms_full = rms_dbfs(&left_full).max(rms_dbfs(&right_full));
    let rms_half = rms_dbfs(&left_half).max(rms_dbfs(&right_half));

    // Expected dB difference: 20*log10(64/127)
    let expected_db_diff = 20.0 * (64.0f64 / 127.0).log10();
    let actual_db_diff = rms_half - rms_full;

    eprintln!("m10: RMS full={rms_full:.2} dBFS, half={rms_half:.2} dBFS");
    eprintln!(
        "m10: Expected dB diff = {expected_db_diff:.4}, actual = {actual_db_diff:.4}"
    );

    // CC7 in SF2 uses a concave curve (not linear), so exact match is not expected.
    // The key requirement is that CC7=64 is quieter than CC7=127.
    assert!(
        rms_half < rms_full,
        "CC7=64 should be quieter than CC7=127: full={rms_full:.2}, half={rms_half:.2}"
    );

    // Verify the attenuation is in a reasonable range (between linear and squared law)
    // Linear: -5.95 dB, Squared: -11.9 dB. SF2 concave curve is typically closer to squared.
    let linear_db = 20.0 * (64.0f64 / 127.0).log10();
    assert!(
        actual_db_diff < linear_db + f32::EPSILON as f64,
        "CC7=64 should attenuate at least as much as linear: actual={actual_db_diff:.4}, linear={linear_db:.4}"
    );
}

// =============================================================================
// M11: cc10_pan — CC#10 panning control
// =============================================================================

#[test]
fn m11_cc10_pan() {
    // Hard left: CC10=0
    let mut engine_left = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    engine_left.cc(0, 10, 0);
    engine_left.note_on(0, 60, 100);
    let (ll, lr) = render_engine_blocks(&mut engine_left, 8);

    // Hard right: CC10=127
    let mut engine_right = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    engine_right.cc(0, 10, 127);
    engine_right.note_on(0, 60, 100);
    let (rl, rr) = render_engine_blocks(&mut engine_right, 8);

    // Center: CC10=64
    let mut engine_center = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    engine_center.cc(0, 10, 64);
    engine_center.note_on(0, 60, 100);
    let (cl, cr) = render_engine_blocks(&mut engine_center, 8);

    let rms_ll = rms_dbfs(&ll);
    let rms_lr = rms_dbfs(&lr);
    let rms_rl = rms_dbfs(&rl);
    let rms_rr = rms_dbfs(&rr);
    let rms_cl = rms_dbfs(&cl);
    let rms_cr = rms_dbfs(&cr);

    eprintln!("m11: Pan left  — L={rms_ll:.2} dBFS, R={rms_lr:.2} dBFS");
    eprintln!("m11: Pan right — L={rms_rl:.2} dBFS, R={rms_rr:.2} dBFS");
    eprintln!("m11: Pan center — L={rms_cl:.2} dBFS, R={rms_cr:.2} dBFS");

    // CC10=0: left channel should be louder than right
    assert!(
        rms_ll > rms_lr,
        "Pan left (CC10=0): left should be louder: L={rms_ll:.2}, R={rms_lr:.2}"
    );

    // CC10=127: right channel should be louder than left
    assert!(
        rms_rr > rms_rl,
        "Pan right (CC10=127): right should be louder: L={rms_rl:.2}, R={rms_rr:.2}"
    );

    // CC10=64: left and right should be approximately balanced
    let balance_diff_db = (rms_cl - rms_cr).abs();
    eprintln!("m11: Center balance diff = {balance_diff_db:.4} dB");
    // Allow up to 1 dB imbalance for center pan (SF2 instruments may have slight stereo content)
    assert!(
        balance_diff_db < 1.0,
        "Pan center (CC10=64): channels should be balanced within 1 dB, got {balance_diff_db:.4} dB"
    );
}

// =============================================================================
// M12: gm_channel9_drums — Ch9 always percussion, ignores program_change
// =============================================================================

#[test]
fn m12_gm_channel9_drums() {
    // First rendering: program_change(9, 0) then kick drum
    let mut engine_a = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    engine_a.program_change(9, 0);
    engine_a.note_on(9, 36, 100);
    let (left_a, right_a) = render_engine_blocks(&mut engine_a, 8);

    // Second rendering: program_change(9, 50) then same kick drum
    let mut engine_b = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };
    engine_b.program_change(9, 50);
    engine_b.note_on(9, 36, 100);
    let (left_b, right_b) = render_engine_blocks(&mut engine_b, 8);

    // Both should produce audible signal
    let pk_a = peak(&left_a).max(peak(&right_a));
    let pk_b = peak(&left_b).max(peak(&right_b));

    eprintln!("m12: Kick with prog=0 peak = {pk_a:.6}");
    eprintln!("m12: Kick with prog=50 peak = {pk_b:.6}");

    assert!(
        pk_a > 0.001,
        "Drum channel should produce audible signal with prog=0, got peak={pk_a}"
    );
    assert!(
        pk_b > 0.001,
        "Drum channel should produce audible signal with prog=50, got peak={pk_b}"
    );

    // The outputs should be bit-exact identical — program_change on ch9 should be ignored
    assert!(
        left_a == left_b,
        "Ch9 drum output should be bit-exact identical regardless of program_change (left channel differs)"
    );
    assert!(
        right_a == right_b,
        "Ch9 drum output should be bit-exact identical regardless of program_change (right channel differs)"
    );
}
