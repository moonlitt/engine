//! Integration test for the state-capture pipeline.
//!
//! Validates the workflow that backs the desktop "Save State" button:
//! get_state() on one plug-in instance, set_state() into a fresh
//! instance, then verify the rehydrated instance produces audio. This
//! is the same code path the Keyscape capture flow exercises -- only
//! the patch picker on the source side differs.
//!
//! Skipped gracefully when no compatible plug-in is installed.

use std::sync::{Mutex, MutexGuard, OnceLock};

use moonlitt_vst3::Vst3Host;

fn plugin_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
}

fn peak_after_blocks(plugin: &mut moonlitt_vst3::Vst3Plugin, blocks: usize) -> f32 {
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    let mut peak = 0.0f32;
    for _ in 0..blocks {
        plugin.render(&mut left, &mut right).unwrap();
        for &s in left.iter().chain(right.iter()) {
            peak = peak.max(s.abs());
        }
    }
    peak
}

#[test]
fn state_capture_and_replay_produces_audio() {
    let _g = plugin_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();

    // Pianoteq is the only test plug-in we have whose default state is a
    // playable sound. Surge powers up to a default patch too, but keeps
    // its programs in a controller-side database we don't fully replay
    // through state alone yet. Stick to Pianoteq for this assertion.
    let info = plugins.iter().find(|p| p.name.contains("Pianoteq"));
    let Some(info) = info else {
        eprintln!("Pianoteq not installed — skipping state-roundtrip test");
        return;
    };

    // 1. Source instance: load, warm up, capture state.
    let mut source = host.load(info).unwrap();
    source.note_on(0, 60, 100);
    let _ = peak_after_blocks(&mut source, 4);
    let state = source.get_state().expect("get_state should succeed");
    assert!(!state.is_empty(), "captured state must not be empty");
    drop(source);

    // 2. Target instance: load fresh, rehydrate from captured bytes,
    //    verify it makes sound.
    let mut target = host.load(info).unwrap();
    target.set_state(&state).expect("set_state should succeed");
    target.note_on(0, 60, 100);
    let p = peak_after_blocks(&mut target, 16);
    assert!(
        p > 1e-3,
        "rehydrated plug-in produced silence (peak={p}); state pipeline broken"
    );
}
