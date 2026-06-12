//! Sprint 6 — MIDI expressiveness parity with the OxiSynth backend:
//! per-channel voice tracking, CC7 volume, CC11 expression, CC64
//! sustain, and pitch bend.
//!
//! Skips gracefully when the reference SF2 isn't on this machine.

use moonlitt_core::AudioBackend;
use moonlitt_sampler::backend::SamplerBackend;
use moonlitt_sampler::voicepool::VoicePool;
use moonlitt_sampler::SamplePool;

const SF2_PATH: &str =
    "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";

fn has_sf2() -> bool {
    std::path::Path::new(SF2_PATH).exists()
}

fn pool() -> SamplePool {
    SamplePool::from_file(SF2_PATH).expect("load SF2")
}

fn loaded_backend() -> SamplerBackend {
    let mut b = SamplerBackend::new(44100).expect("backend");
    b.load(SF2_PATH).expect("load SF2");
    b
}

fn rms(l: &[f32], r: &[f32]) -> f64 {
    let sum: f64 = l
        .iter()
        .chain(r.iter())
        .map(|&s| (s as f64) * (s as f64))
        .sum();
    (sum / (l.len() + r.len()) as f64).sqrt()
}

/// Render eight 1024-frame blocks and report the RMS of the last four
/// (past the attack), so envelope shape doesn't skew level comparisons.
fn settled_rms(b: &mut SamplerBackend) -> f64 {
    let mut l = vec![0.0f32; 1024];
    let mut r = vec![0.0f32; 1024];
    let mut tail = 0.0;
    for i in 0..8 {
        l.fill(0.0);
        r.fill(0.0);
        b.render(&mut l, &mut r);
        if i >= 4 {
            tail += rms(&l, &r);
        }
    }
    tail / 4.0
}

// ---------------------------------------------------------------------------
// Channel tracking (VoicePool level)
// ---------------------------------------------------------------------------

#[test]
fn t1_note_off_releases_only_its_channel() {
    if !has_sf2() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let pool = pool();
    let mut vp = VoicePool::new(8, 44100);

    vp.note_on(&pool, 0, 0, 0, 60, 100);
    vp.note_on(&pool, 1, 0, 0, 60, 100);
    assert_eq!(vp.voices_on_channel(0), 1);
    assert_eq!(vp.voices_on_channel(1), 1);

    vp.note_off(1, 60);
    assert_eq!(
        vp.releasing_on_channel(1),
        1,
        "channel 1's voice must be releasing"
    );
    assert_eq!(
        vp.releasing_on_channel(0),
        0,
        "channel 0's voice must be untouched"
    );
}

// ---------------------------------------------------------------------------
// Sustain pedal (CC64)
// ---------------------------------------------------------------------------

#[test]
fn t2_sustain_pedal_holds_and_releases() {
    if !has_sf2() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let pool = pool();
    let mut vp = VoicePool::new(8, 44100);

    vp.cc(0, 64, 127); // pedal down
    vp.note_on(&pool, 0, 0, 0, 60, 100);
    vp.note_off(0, 60);
    assert_eq!(
        vp.releasing_on_channel(0),
        0,
        "note must be held by the sustain pedal"
    );

    vp.cc(0, 64, 0); // pedal up
    assert_eq!(
        vp.releasing_on_channel(0),
        1,
        "pedal release must release the held note"
    );
}

// ---------------------------------------------------------------------------
// CC7 / CC11 gain (backend level, end to end)
// ---------------------------------------------------------------------------

#[test]
fn t3_cc7_volume_attenuates_output() {
    if !has_sf2() {
        eprintln!("SF2 not found, skipping");
        return;
    }

    let mut full = loaded_backend();
    full.note_on(0, 60, 100);
    let full_rms = settled_rms(&mut full);
    assert!(full_rms > 1e-5, "reference render must be audible");

    let mut half = loaded_backend();
    half.cc(0, 7, 64);
    half.note_on(0, 60, 100);
    let half_rms = settled_rms(&mut half);

    let ratio = half_rms / full_rms;
    // Square-law gain: (64/127)^2 ≈ 0.254
    assert!(
        (0.15..0.40).contains(&ratio),
        "CC7=64 should attenuate to ~0.25× (got {ratio:.3})"
    );

    // Defaults are full-scale: explicitly setting CC7=127 changes nothing.
    let mut explicit = loaded_backend();
    explicit.cc(0, 7, 127);
    explicit.note_on(0, 60, 100);
    let explicit_rms = settled_rms(&mut explicit);
    assert!(
        (explicit_rms / full_rms - 1.0).abs() < 0.01,
        "CC7=127 must equal the untouched default"
    );
}

#[test]
fn t4_cc11_expression_attenuates_output() {
    if !has_sf2() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let mut full = loaded_backend();
    full.note_on(0, 60, 100);
    let full_rms = settled_rms(&mut full);

    let mut quiet = loaded_backend();
    quiet.cc(0, 11, 32);
    quiet.note_on(0, 60, 100);
    let quiet_rms = settled_rms(&mut quiet);

    assert!(
        quiet_rms < full_rms * 0.2,
        "CC11=32 should strongly attenuate (full={full_rms:.5}, quiet={quiet_rms:.5})"
    );
}

#[test]
fn t5_cc_on_one_channel_does_not_affect_another() {
    if !has_sf2() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let mut b = loaded_backend();
    b.cc(3, 7, 0); // silence channel 3 only
    b.note_on(0, 60, 100);
    let out = settled_rms(&mut b);
    assert!(
        out > 1e-5,
        "channel 0 must be unaffected by channel 3's CC7"
    );
}

// ---------------------------------------------------------------------------
// Pitch bend
// ---------------------------------------------------------------------------

#[test]
fn t6_pitch_bend_changes_output_and_stays_finite() {
    if !has_sf2() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let render = |bend: i16| -> Vec<f32> {
        let mut b = loaded_backend();
        if bend != 0 {
            b.pitch_bend(0, bend);
        }
        b.note_on(0, 60, 100);
        let mut l = vec![0.0f32; 4096];
        let mut r = vec![0.0f32; 4096];
        b.render(&mut l, &mut r);
        assert!(
            l.iter().chain(r.iter()).all(|s| s.is_finite()),
            "bend={bend}: output must stay finite"
        );
        l
    };

    let unbent = render(0);
    let bent_up = render(8191);
    let bent_down = render(-8192);

    assert!(unbent != bent_up, "full bend up must alter the waveform");
    assert!(
        unbent != bent_down,
        "full bend down must alter the waveform"
    );
    assert!(bent_up != bent_down, "up and down bends must differ");
}

#[test]
fn t7_bend_applies_to_already_sounding_notes() {
    if !has_sf2() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let mut b = loaded_backend();
    b.note_on(0, 60, 100);
    let mut l = vec![0.0f32; 1024];
    let mut r = vec![0.0f32; 1024];
    b.render(&mut l, &mut r);
    let before = l.clone();

    b.pitch_bend(0, 8191); // bend while the note is sounding
    b.render(&mut l, &mut r);

    // The post-bend block must not continue the unbent trajectory: render
    // the same span on an unbent twin and compare.
    let mut twin = loaded_backend();
    twin.note_on(0, 60, 100);
    let mut tl = vec![0.0f32; 1024];
    let mut tr = vec![0.0f32; 1024];
    twin.render(&mut tl, &mut tr);
    assert_eq!(before, tl, "twin setup must match before the bend");
    twin.render(&mut tl, &mut tr);

    assert!(l != tl, "live bend must alter already-sounding voices");
}
