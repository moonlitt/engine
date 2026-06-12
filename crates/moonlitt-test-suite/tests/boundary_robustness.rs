//! Boundary and Robustness Tests
//!
//! References:
//! - IEEE 754-2019: https://standards.ieee.org/ieee/754/6210/
//! - IEC 61672-1 (denormal handling)
//!
//! Verifies the engine handles edge cases without crash, overflow, or state corruption.

use moonlitt_audio_io::mixer::Mixer;
use moonlitt_core::AudioBackend;
use std::path::Path;
use std::time::Instant;

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

/// Create a backend loaded with the real SF2 at a custom sample rate.
fn load_sf2_engine_at(sample_rate: u32) -> Option<Box<dyn AudioBackend>> {
    if !Path::new(SF2_PATH).exists() {
        eprintln!("SF2 not found at {SF2_PATH}, skipping test");
        return None;
    }
    moonlitt_engine::create(SF2_PATH, sample_rate, BUFFER_SIZE as u32).ok()
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
    if rms < 1e-20 {
        f64::NEG_INFINITY
    } else {
        20.0 * rms.log10()
    }
}

/// Verify no NaN or Inf in buffer.
fn assert_no_nan_inf(buf: &[f32], name: &str) {
    for (i, &s) in buf.iter().enumerate() {
        assert!(!s.is_nan(), "{name}[{i}] is NaN");
        assert!(!s.is_infinite(), "{name}[{i}] is Inf");
    }
}

// =============================================================================
// B1: Denormal Protection
// =============================================================================

#[test]
fn b01_denormal_protection() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);

    // Play a note and let it decay into near-silence
    mixer.note_on(0, 60, 127);

    // Render 16 blocks with active signal — measure time as baseline
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];

    // Warm up
    for _ in 0..4 {
        mixer.render(&mut left, &mut right);
    }

    let start_normal = Instant::now();
    for _ in 0..100 {
        mixer.render(&mut left, &mut right);
    }
    let dur_normal = start_normal.elapsed();

    // Release note and render until tail is very quiet
    mixer.note_off(0, 60);
    for _ in 0..500 {
        mixer.render(&mut left, &mut right);
    }

    // Now render 100 blocks in near-silence (potential denormal territory)
    let start_quiet = Instant::now();
    for _ in 0..100 {
        mixer.render(&mut left, &mut right);
    }
    let dur_quiet = start_quiet.elapsed();

    let ratio = dur_quiet.as_secs_f64() / dur_normal.as_secs_f64();
    eprintln!("b01: normal={dur_normal:?}, quiet={dur_quiet:?}, ratio={ratio:.2}x");
    assert!(
        ratio < 3.0,
        "Near-silence render is {ratio:.2}x slower than normal — possible denormal CPU spike"
    );
}

// =============================================================================
// B2: Overflow Protection (Limiter)
// =============================================================================

#[test]
fn b02_overflow_protection() {
    // Create mixer with default limiter_threshold (0.95)
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);

    // Add multiple tracks all playing loud notes to force summation beyond 1.0
    for i in 0..8 {
        let engine = match load_sf2_engine() {
            Some(e) => e,
            None => return,
        };
        let track_id = mixer.add_track(engine, 0xFFFF);
        // Boost volume per track to force hot signal
        mixer.track_mut(track_id).unwrap().volume = 2.0;
        // Each track plays a different note at max velocity
        mixer.note_on(0, 48 + i, 127);
    }

    // Also boost master volume
    mixer.set_master_volume(2.0);

    let (left, right) = render_blocks(&mut mixer, 32);

    // Verify no sample exceeds [-1.0, 1.0]
    for (i, (&l, &r)) in left.iter().zip(right.iter()).enumerate() {
        assert!(l.abs() <= 1.0, "left[{i}] = {l} exceeds [-1, 1]");
        assert!(r.abs() <= 1.0, "right[{i}] = {r} exceeds [-1, 1]");
    }

    // Verify we actually had loud input (signal exists)
    assert_no_nan_inf(&left, "left");
    assert_no_nan_inf(&right, "right");

    let peak_l: f32 = left.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    let peak_r: f32 = right.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    eprintln!("b02: peak_l={peak_l:.6}, peak_r={peak_r:.6}");
    assert!(
        peak_l > 0.01,
        "Expected audible signal from 8 tracks, got peak_l={peak_l}"
    );
}

// =============================================================================
// B3: Empty Mixer Renders Silence
// =============================================================================

#[test]
fn b03_empty_mixer_renders_silence() {
    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);

    // No tracks added — render should produce bit-exact silence
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    mixer.render(&mut left, &mut right);

    for (i, (&l, &r)) in left.iter().zip(right.iter()).enumerate() {
        assert_eq!(l, 0.0, "left[{i}] is not bit-exact 0.0: {l}");
        assert_eq!(r, 0.0, "right[{i}] is not bit-exact 0.0: {r}");
    }

    eprintln!("b03: Empty mixer produces bit-exact silence ({BUFFER_SIZE} samples)");
}

// =============================================================================
// B4: Max Polyphony
// =============================================================================

#[test]
fn b04_max_polyphony() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);

    // Send 256 NoteOn events across the full note range (36-91, repeated)
    for i in 0..256u16 {
        let note = 36 + (i % 56) as u8; // notes 36-91
        let channel = (i % 16) as u8;
        mixer.note_on(channel, note, 127);
    }

    // Render 4 blocks — must not panic, overflow, or produce NaN/Inf
    let (left, right) = render_blocks(&mut mixer, 4);

    assert_no_nan_inf(&left, "left");
    assert_no_nan_inf(&right, "right");

    let peak_l: f32 = left.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    eprintln!(
        "b04: 256 simultaneous notes, rendered {} samples, peak={peak_l:.6}",
        left.len()
    );
}

// =============================================================================
// B5: Zero-Length Buffer
// =============================================================================

#[test]
fn b05_zero_length_buffer() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);
    mixer.note_on(0, 60, 100);

    // Render with zero-length slices — should not panic
    mixer.render(&mut [], &mut []);

    // Now render a normal buffer — engine state should not be corrupted
    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];
    mixer.render(&mut left, &mut right);

    assert_no_nan_inf(&left, "left after zero-length render");
    assert_no_nan_inf(&right, "right after zero-length render");

    let peak_l: f32 = left.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    eprintln!("b05: After zero-length render, normal render peak={peak_l:.6}");
    assert!(
        peak_l > 0.001,
        "Engine should still produce signal after zero-length render, got peak={peak_l}"
    );
}

// =============================================================================
// B6: Extreme Sample Rates
// =============================================================================

#[test]
fn b06_extreme_sample_rates() {
    // Test at 8 kHz (telephony)
    {
        let engine = match load_sf2_engine_at(8000) {
            Some(e) => e,
            None => return,
        };

        let mut mixer = Mixer::new(8000, BUFFER_SIZE);
        mixer.add_track(engine, 0xFFFF);
        mixer.note_on(0, 60, 100);

        let (left, right) = render_blocks(&mut mixer, 16);
        assert_no_nan_inf(&left, "left@8kHz");
        assert_no_nan_inf(&right, "right@8kHz");

        let rms = rms_dbfs(&left);
        eprintln!("b06: 8 kHz render RMS = {rms:.1} dBFS");
        assert!(
            rms > -60.0,
            "8 kHz render should produce signal > -60 dBFS, got {rms:.1} dBFS"
        );
    }

    // Test at 192 kHz (studio high-res)
    {
        let engine = match load_sf2_engine_at(192000) {
            Some(e) => e,
            None => return,
        };

        let mut mixer = Mixer::new(192000, BUFFER_SIZE);
        mixer.add_track(engine, 0xFFFF);
        mixer.note_on(0, 60, 100);

        let (left, right) = render_blocks(&mut mixer, 16);
        assert_no_nan_inf(&left, "left@192kHz");
        assert_no_nan_inf(&right, "right@192kHz");

        let rms = rms_dbfs(&left);
        eprintln!("b06: 192 kHz render RMS = {rms:.1} dBFS");
        assert!(
            rms > -60.0,
            "192 kHz render should produce signal > -60 dBFS, got {rms:.1} dBFS"
        );
    }
}

// =============================================================================
// B7: NaN/Inf Propagation (Stress Test)
// =============================================================================

#[test]
fn b07_nan_inf_propagation() {
    let engine = match load_sf2_engine() {
        Some(e) => e,
        None => return,
    };

    let mut mixer = Mixer::new(SAMPLE_RATE, BUFFER_SIZE);
    mixer.add_track(engine, 0xFFFF);

    let mut left = vec![0.0f32; BUFFER_SIZE];
    let mut right = vec![0.0f32; BUFFER_SIZE];

    // Rapid NoteOn/NoteOff stress test: 100 pairs with render between each
    for i in 0..100u8 {
        let note = 36 + (i % 56); // notes 36-91
        let velocity = 64 + (i % 64); // velocities 64-127

        mixer.note_on(0, note, velocity);
        mixer.note_off(0, note);

        mixer.render(&mut left, &mut right);

        // Check every sample after every render
        for (j, (&l, &r)) in left.iter().zip(right.iter()).enumerate() {
            assert!(
                !l.is_nan() && !l.is_infinite(),
                "left[{j}] is NaN/Inf after rapid note pair {i}"
            );
            assert!(
                !r.is_nan() && !r.is_infinite(),
                "right[{j}] is NaN/Inf after rapid note pair {i}"
            );
        }
    }

    eprintln!("b07: 100 rapid NoteOn/NoteOff pairs — all output samples finite");
}
