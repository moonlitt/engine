//! Proof that `Vst3Backend::plugin_handle()` exposes the same plug-in
//! instance the audio side renders against.
//!
//! This is the architectural invariant the desktop app relies on: the
//! plug-in GUI window clones the Arc, drives `set_state` through it, and
//! the audio thread (which goes through `Vst3Backend::render`) hears the
//! new patch without any state-copy or backend rebuild.
//!
//! Skips gracefully when Keyscape is unavailable. Uses the same
//! `keyscape-default.mlstate` fixture as `keyscape_session_roundtrip` so
//! the two tests stay in lock-step on patch coverage.

#![cfg(feature = "vst3")]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use moonlitt_core::AudioBackend;
use moonlitt_engine::backends::vst3::Vst3Backend;

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
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()? // crates/
        .join("moonlitt-vst3")
        .join("tests")
        .join("fixtures")
        .join("keyscape-default.mlstate");
    p.exists().then_some(p)
}

#[test]
fn gui_side_set_state_audible_through_audio_backend() {
    let _g = keyscape_lock();

    let Some(path) = find_keyscape_path() else {
        eprintln!("Keyscape not installed — skipping shared-handle test");
        return;
    };
    let Some(fixture) = find_fixture() else {
        eprintln!(
            "Keyscape fixture not present — skipping. Capture one via the \
             desktop app's '💾 导出状态' button."
        );
        return;
    };
    let state_bytes = std::fs::read(&fixture).expect("fixture readable");

    let sample_rate = 44100u32;
    let buffer_size = 256u32;

    // The audio side: a Vst3Backend that holds the plug-in via an Arc.
    let mut backend = Vst3Backend::new(sample_rate, buffer_size).expect("backend new");
    backend
        .load(&path.to_string_lossy())
        .expect("Keyscape load");

    // The GUI side: a clone of the Arc, as if plugin_window had grabbed
    // it from `Engine::vst3_plugin_handle`.
    let gui_handle = backend
        .plugin_handle()
        .expect("VST3 backend must expose plug-in handle after load");

    // Drive state through the GUI-side handle. This is the moment that
    // proves single-instance: under the old design, this `set_state` on
    // a separate plug-in would have left the audio side silent until we
    // performed an explicit state-copy + rebuild.
    {
        let mut p = gui_handle.lock();
        p.set_state(&state_bytes)
            .expect("Keyscape must accept fixture state via shared handle");
        let warmup = p.recommended_warm_up_blocks();
        assert_eq!(
            warmup, 8192,
            "Keyscape must auto-report 8192 warm-up blocks"
        );
        p.warm_up(warmup)
            .expect("warm-up must succeed via shared handle");
    }

    // Round-trip check: state read back through the backend matches what
    // we'd read back through the GUI-side handle. Identical bytes prove
    // "same instance" without depending on audio rendering working.
    let backend_state = backend.save_state().expect("backend save_state");
    let gui_state = gui_handle
        .lock()
        .get_state()
        .expect("gui handle get_state");
    assert_eq!(
        backend_state, gui_state,
        "shared handle and audio backend must serialise identical state — \
         proves they are the same Vst3Plugin instance"
    );

    // End-to-end: drive notes through the AudioBackend trait (this is
    // exactly what the audio thread would do) and verify the patch the
    // GUI side just installed is audible.
    backend.note_on(0, 60, 100);
    backend.note_on(0, 64, 100);
    backend.note_on(0, 67, 100);

    let mut left = vec![0.0f32; buffer_size as usize];
    let mut right = vec![0.0f32; buffer_size as usize];
    let mut peak = 0.0f32;
    // ~750 ms — plenty of time for the patch to develop past its attack.
    for _ in 0..128 {
        left.fill(0.0);
        right.fill(0.0);
        backend.render(&mut left, &mut right);
        for s in left.iter().chain(right.iter()) {
            let m = s.abs();
            if m > peak {
                peak = m;
            }
        }
    }

    assert!(
        peak > 1e-3,
        "GUI-side set_state must be audible through the audio backend \
         (peak={peak})"
    );
    println!("✅ shared handle → audible patch (peak={peak:.4})");
}
