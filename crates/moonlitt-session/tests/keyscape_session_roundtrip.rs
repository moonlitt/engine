//! End-to-end session validation with a sample-streaming plug-in.
//!
//! This is the load-bearing test for the "DAW-grade session" promise: an
//! engine that can `save_to_file` a project containing Keyscape, then on a
//! cold start `load_from_file` and produce audio without any extra fiddling
//! from the caller — no manual warm_up calls, no patch reselection.
//!
//! The test wires the same path Logic Pro / Reaper would take:
//!
//!   1. Build a live Mixer with a Keyscape track, replaying the captured
//!      state fixture (the same `keyscape-default.mlstate` that the user
//!      saved through moonlitt's GUI).
//!   2. `Session::from_state(...)` snapshots mixer + transport into JSON.
//!      `Vst3Backend` self-reports `recommended_warm_up_blocks = 8192`
//!      because the plug-in's PClassInfo2 vendor is "Spectrasonics".
//!   3. Serialize → deserialize, simulating a cold-start reload.
//!   4. `Session::restore` rebuilds the mixer from scratch: re-creates
//!      Vst3Backend, replays state, runs the 8192-block warm-up.
//!   5. Send notes through the restored mixer and verify audio comes out.
//!
//! Gracefully skips when Keyscape or its fixture is absent. Serializes via
//! a process-wide lock (Keyscape's STEAM library crashes on concurrent
//! instances within the same process).

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use moonlitt_mixer::Mixer;
use moonlitt_session::persistence::Session;
use moonlitt_session::Transport;

fn keyscape_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn find_keyscape_path() -> Option<PathBuf> {
    let host = moonlitt_vst3::Vst3Host::new(44100, 256).ok()?;
    let plugins = host.scan().ok()?;
    plugins
        .into_iter()
        .find(|p| p.name == "Keyscape")
        .map(|info| info.path)
}

fn find_fixture() -> Option<PathBuf> {
    // The fixture lives in the moonlitt-vst3 crate's tests dir — same
    // file the keyscape_probe test exercises. Resolved relative to the
    // session crate at compile time.
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()? // crates/
        .join("moonlitt-vst3")
        .join("tests")
        .join("fixtures")
        .join("keyscape-default.mlstate");
    p.exists().then_some(p)
}

/// Render `num_blocks` of audio through the mixer and return the peak
/// magnitude seen on either channel. Drains MIDI events naturally.
fn render_peak(mixer: &mut Mixer, num_blocks: usize) -> f32 {
    let buffer_size = 256;
    let mut left = vec![0.0f32; buffer_size];
    let mut right = vec![0.0f32; buffer_size];
    let mut peak = 0.0f32;
    for _ in 0..num_blocks {
        left.fill(0.0);
        right.fill(0.0);
        mixer.render(&mut left, &mut right);
        for s in left.iter().chain(right.iter()) {
            let m = s.abs();
            if m > peak {
                peak = m;
            }
        }
    }
    peak
}

#[test]
fn keyscape_track_survives_full_session_roundtrip() {
    let _g = keyscape_lock();

    let Some(keyscape_path) = find_keyscape_path() else {
        eprintln!("Keyscape not installed — skipping session roundtrip test");
        return;
    };
    let Some(fixture_path) = find_fixture() else {
        eprintln!(
            "Keyscape fixture not present — skipping. Capture one via desktop \
             shell's '💾 保存状态' to crates/moonlitt-vst3/tests/fixtures/keyscape-default.mlstate"
        );
        return;
    };
    let state_bytes = std::fs::read(&fixture_path).expect("fixture readable");

    // -----------------------------------------------------------------------
    // Stage 1: build a live mixer with Keyscape pre-loaded + patch restored.
    // -----------------------------------------------------------------------
    let sample_rate = 44100u32;
    let buffer_size = 256usize;
    let mut mixer = Mixer::new(sample_rate, buffer_size);

    let mut backend = moonlitt_engine::create(
        &keyscape_path.to_string_lossy(),
        sample_rate,
        buffer_size as u32,
    )
    .expect("Vst3Backend creation must succeed");
    backend
        .load_state(&state_bytes)
        .expect("Keyscape must accept fixture state");

    // Vst3Backend should self-identify as needing 8192-block warm-up.
    // Keyscape lands here via the hard-coded known-streamer fallback in
    // Vst3Plugin::recommended_warm_up_blocks (its PClassInfo2 vendor is
    // empty and subcategory is mis-tagged "Synth").
    assert_eq!(
        backend.recommended_warm_up_blocks(),
        8192,
        "Keyscape must auto-report a sample-streamer warm-up requirement"
    );

    mixer.add_track_with_source(
        backend,
        Some(keyscape_path.to_string_lossy().into_owned()),
        0xFFFF,
    );

    // -----------------------------------------------------------------------
    // Stage 2: snapshot → JSON → load.
    // -----------------------------------------------------------------------
    let transport = Transport::new();
    let session = Session::from_state(&mixer, &transport, None);
    let json = session.to_json().expect("session JSON must serialize");
    assert_eq!(session.tracks.len(), 1);
    assert_eq!(
        session.tracks[0].source.warm_up_blocks, 8192,
        "session JSON must persist the warm-up recommendation"
    );

    // Drop the live mixer to ensure we're really doing a cold restore, not
    // hanging onto the in-process Vst3Plugin instance.
    drop(mixer);

    // -----------------------------------------------------------------------
    // Stage 3: cold restore.
    // -----------------------------------------------------------------------
    let restored = Session::from_json(&json)
        .expect("session JSON must deserialize")
        .restore(buffer_size)
        .expect("restore must succeed including auto warm-up");

    assert_eq!(restored.mixer.tracks().len(), 1);

    // -----------------------------------------------------------------------
    // Stage 4: send notes through the restored mixer; expect audio.
    // -----------------------------------------------------------------------
    let mut restored_mixer = restored.mixer;
    let track_id = restored_mixer.tracks()[0].id;
    {
        let track = restored_mixer.track_mut(track_id).expect("track exists");
        track.backend.note_on(0, 60, 100);
        track.backend.note_on(0, 64, 100);
        track.backend.note_on(0, 67, 100);
    }

    let peak = render_peak(&mut restored_mixer, 512);
    assert!(
        peak > 1e-3,
        "restored Keyscape session must produce audio (peak={peak})"
    );
    println!("✅ Keyscape survives full session roundtrip → peak={peak:.4}");
}
