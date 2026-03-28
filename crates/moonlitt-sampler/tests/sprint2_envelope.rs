//! Sprint 2 Tests: DAHDSR Envelope
//!
//! SF2 spec defines 6-stage volume envelope:
//!   Delay → Attack → Hold → Decay → Sustain → Release
//!
//! Timing is in "timecents": tc = 1200 × log2(seconds)
//!   0 tc = 1s, 1200 tc = 2s, -1200 tc = 0.5s, -12000 tc ≈ instant
//!
//! Acceptance criteria:
//! 1. Attack from 0 to 1.0 in specified time
//! 2. Decay from 1.0 to sustain level
//! 3. Sustain holds at constant level until note-off
//! 4. Release from sustain to 0 after note-off
//! 5. Timecents conversion matches SF2 spec exactly
//! 6. Envelope integrated with Voice produces shaped audio

use moonlitt_sampler::envelope::{Envelope, EnvelopeParams};

const SAMPLE_RATE: u32 = 44100;

// =============================================================================
// Test 1: Timecents to seconds conversion
// =============================================================================

#[test]
fn t1_timecents_to_seconds() {
    use moonlitt_sampler::envelope::timecents_to_secs;

    // SF2 spec: timecents = 1200 × log2(seconds)
    // So seconds = 2^(timecents / 1200)
    let eps = 1e-6;

    assert!((timecents_to_secs(0) - 1.0).abs() < eps, "0 tc = 1s");
    assert!((timecents_to_secs(1200) - 2.0).abs() < eps, "1200 tc = 2s");
    assert!((timecents_to_secs(-1200) - 0.5).abs() < eps, "-1200 tc = 0.5s");
    assert!((timecents_to_secs(2400) - 4.0).abs() < eps, "2400 tc = 4s");

    // Very small (effectively instant)
    let instant = timecents_to_secs(-12000);
    assert!(instant < 0.001, "-12000 tc should be < 1ms, got {instant}");
}

// =============================================================================
// Test 2: Attack ramps from 0 to 1
// =============================================================================

#[test]
fn t2_attack_ramp() {
    let params = EnvelopeParams {
        delay: -12000,   // instant
        attack: 0,       // 1 second
        hold: -12000,    // instant
        decay: -12000,   // instant
        sustain: 0.0,    // 0dB = full sustain (no decay)
        release: -12000, // instant
    };
    let mut env = Envelope::new(params, SAMPLE_RATE);
    env.note_on();

    // Sample at start: should be near 0
    let start = env.process();
    assert!(start < 0.01, "Start of attack should be near 0, got {start}");

    // Advance ~halfway through attack (0.5s = 22050 samples)
    for _ in 0..22050 {
        env.process();
    }
    let mid = env.process();
    assert!(mid > 0.3 && mid < 0.7, "Midpoint of 1s attack should be ~0.5, got {mid}");

    // Advance to end of attack (~1s total)
    for _ in 0..22049 {
        env.process();
    }
    let end = env.process();
    assert!(end > 0.95, "End of 1s attack should be near 1.0, got {end}");
}

// =============================================================================
// Test 3: Sustain holds constant level
// =============================================================================

#[test]
fn t3_sustain_holds() {
    let params = EnvelopeParams {
        delay: -12000,
        attack: -12000,  // instant attack
        hold: -12000,
        decay: -12000,   // instant decay
        sustain: 0.5,    // sustain at 50%
        release: -12000,
    };
    let mut env = Envelope::new(params, SAMPLE_RATE);
    env.note_on();

    // Skip past attack+decay
    for _ in 0..1000 {
        env.process();
    }

    // Sustain should be constant at 0.5
    let mut samples = Vec::new();
    for _ in 0..100 {
        samples.push(env.process());
    }

    let avg = samples.iter().sum::<f32>() / samples.len() as f32;
    let max_dev = samples.iter().map(|s| (s - avg).abs()).fold(0.0f32, f32::max);

    assert!((avg - 0.5).abs() < 0.05, "Sustain average should be ~0.5, got {avg}");
    assert!(max_dev < 0.01, "Sustain should be constant, max deviation {max_dev}");
}

// =============================================================================
// Test 4: Release decays to 0 after note-off
// =============================================================================

#[test]
fn t4_release_to_zero() {
    let params = EnvelopeParams {
        delay: -12000,
        attack: -12000,
        hold: -12000,
        decay: -12000,
        sustain: 1.0,    // full sustain
        release: 0,      // 1 second release
    };
    let mut env = Envelope::new(params, SAMPLE_RATE);
    env.note_on();

    // Skip to sustain
    for _ in 0..1000 {
        env.process();
    }

    // Should be at ~1.0 (sustain)
    let before_off = env.process();
    assert!(before_off > 0.9, "Before note-off should be ~1.0, got {before_off}");

    // Note off
    env.note_off();

    // After ~1 second, should be near 0
    for _ in 0..44100 {
        env.process();
    }
    let after_release = env.process();
    assert!(after_release < 0.01, "After 1s release should be ~0, got {after_release}");
}

// =============================================================================
// Test 5: Envelope is silent when not triggered
// =============================================================================

#[test]
fn t5_idle_is_silent() {
    let params = EnvelopeParams::default();
    let mut env = Envelope::new(params, SAMPLE_RATE);

    // Without note_on, should be 0
    for _ in 0..100 {
        let v = env.process();
        assert_eq!(v, 0.0, "Idle envelope should output 0");
    }
}

// =============================================================================
// Test 6: Full DAHDSR cycle shape
// =============================================================================

#[test]
fn t6_full_cycle() {
    let params = EnvelopeParams {
        delay: -12000,
        attack: -7200,   // ~0.25s attack (2^(-7200/1200) = 2^-6 = 0.015625... no)
        hold: -12000,
        decay: -4800,    // ~0.0625s
        sustain: 0.6,
        release: -4800,  // ~0.0625s
    };
    let mut env = Envelope::new(params, SAMPLE_RATE);
    env.note_on();

    // Collect 1 second of envelope
    let mut values = Vec::new();
    for _ in 0..SAMPLE_RATE {
        values.push(env.process());
    }

    // Should start at 0, rise to peak, decay to sustain level
    assert!(values[0] < 0.1, "Should start near 0");

    let peak = values.iter().cloned().fold(0.0f32, f32::max);
    assert!(peak > 0.9, "Should reach near 1.0, got {peak}");

    // End of 1s should be at sustain level (0.6)
    let end = values.last().unwrap();
    assert!((*end - 0.6).abs() < 0.1, "Should settle at sustain 0.6, got {end}");

    // Now release
    env.note_off();
    let mut release_end = 0.0;
    for _ in 0..SAMPLE_RATE {
        release_end = env.process();
    }
    assert!(release_end < 0.01, "After release should be ~0, got {release_end}");
}
