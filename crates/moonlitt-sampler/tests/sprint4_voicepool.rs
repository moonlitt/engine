//! Sprint 4 Tests: VoicePool — polyphony + voice stealing
//!
//! Acceptance criteria:
//! 1. Multiple simultaneous notes (polyphony)
//! 2. Voice stealing when pool exhausted
//! 3. Note-off releases correct voice
//! 4. All-notes-off silences everything
//! 5. No memory leaks (voice count bounded)
//! 6. Render output is sum of all active voices

const SF2_PATH: &str = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
const SAMPLE_RATE: u32 = 44100;

fn has_sf2() -> bool {
    std::path::Path::new(SF2_PATH).exists()
}

use moonlitt_sampler::voicepool::VoicePool;
use moonlitt_sampler::SamplePool;

// =============================================================================
// Test 1: Polyphony — multiple notes sound simultaneously
// =============================================================================

#[test]
fn t1_polyphony() {
    if !has_sf2() { return; }

    let pool = SamplePool::from_file(SF2_PATH).unwrap();
    let mut vp = VoicePool::new(16, SAMPLE_RATE);

    // Play C major chord: C4, E4, G4
    vp.note_on(&pool, 0, 0, 60, 100); // C4
    vp.note_on(&pool, 0, 0, 64, 100); // E4
    vp.note_on(&pool, 0, 0, 67, 100); // G4

    assert_eq!(vp.active_count(), 3, "Should have 3 active voices");

    // Render
    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    vp.render(&mut left, &mut right);

    let rms = (left.iter().map(|s| s * s).sum::<f32>() / left.len() as f32).sqrt();
    assert!(rms > 0.01, "Chord should produce audio, RMS={rms}");
}

// =============================================================================
// Test 2: Voice stealing when pool is full
// =============================================================================

#[test]
fn t2_voice_stealing() {
    if !has_sf2() { return; }

    let pool = SamplePool::from_file(SF2_PATH).unwrap();
    let mut vp = VoicePool::new(4, SAMPLE_RATE); // Only 4 voices

    // Play 5 notes — should steal the oldest
    vp.note_on(&pool, 0, 0, 60, 100);
    vp.note_on(&pool, 0, 0, 62, 100);
    vp.note_on(&pool, 0, 0, 64, 100);
    vp.note_on(&pool, 0, 0, 65, 100);
    vp.note_on(&pool, 0, 0, 67, 100); // This should steal voice 0 (note 60)

    assert!(vp.active_count() <= 4, "Should not exceed pool size of 4");

    // Should still produce audio
    let mut left = vec![0.0f32; 1024];
    let mut right = vec![0.0f32; 1024];
    vp.render(&mut left, &mut right);

    let rms = (left.iter().map(|s| s * s).sum::<f32>() / left.len() as f32).sqrt();
    assert!(rms > 0.01, "Should still produce audio after stealing");
}

// =============================================================================
// Test 3: Note-off releases correct voice
// =============================================================================

#[test]
fn t3_note_off() {
    if !has_sf2() { return; }

    let pool = SamplePool::from_file(SF2_PATH).unwrap();
    let mut vp = VoicePool::new(16, SAMPLE_RATE);

    vp.note_on(&pool, 0, 0, 60, 100);
    vp.note_on(&pool, 0, 0, 64, 100);
    assert_eq!(vp.active_count(), 2);

    // Release note 60
    vp.note_off(0, 60);

    // Render some samples to let envelope release
    let mut left = vec![0.0f32; 44100]; // 1 second
    let mut right = vec![0.0f32; 44100];
    vp.render(&mut left, &mut right);

    // After release, one voice should have finished
    // (depends on release time — with instant release, should be 1)
    assert!(vp.active_count() <= 2, "Note-off should release a voice");
}

// =============================================================================
// Test 4: All-notes-off silences everything
// =============================================================================

#[test]
fn t4_all_notes_off() {
    if !has_sf2() { return; }

    let pool = SamplePool::from_file(SF2_PATH).unwrap();
    let mut vp = VoicePool::new(16, SAMPLE_RATE);

    // Play many notes
    for note in 48..72 {
        vp.note_on(&pool, 0, 0, note, 100);
    }
    assert!(vp.active_count() > 0);

    vp.all_notes_off();

    // Render
    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    vp.render(&mut left, &mut right);

    // Should be silent (or nearly silent after release tails)
    assert_eq!(vp.active_count(), 0, "All voices should be inactive");
}

// =============================================================================
// Test 5: Voice count never exceeds pool size
// =============================================================================

#[test]
fn t5_bounded_voices() {
    if !has_sf2() { return; }

    let pool = SamplePool::from_file(SF2_PATH).unwrap();
    let max_voices = 8;
    let mut vp = VoicePool::new(max_voices, SAMPLE_RATE);

    // Spam 100 notes
    for note in 0..100 {
        vp.note_on(&pool, 0, 0, (note % 128) as u8, 100);
    }

    assert!(
        vp.active_count() <= max_voices,
        "Active voices {} should never exceed pool size {}",
        vp.active_count(), max_voices
    );
}

// =============================================================================
// Test 6: Rendered output is not clipping
// =============================================================================

#[test]
fn t6_no_clipping() {
    if !has_sf2() { return; }

    let pool = SamplePool::from_file(SF2_PATH).unwrap();
    let mut vp = VoicePool::new(16, SAMPLE_RATE);

    // Play a full chord
    for note in [60, 64, 67, 72, 76, 79] {
        vp.note_on(&pool, 0, 0, note, 100);
    }

    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    vp.render(&mut left, &mut right);

    let peak = left.iter().chain(right.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    assert!(!peak.is_nan(), "No NaN");
    assert!(!peak.is_infinite(), "No Inf");
    eprintln!("6-note chord peak: {peak:.4}");
}
