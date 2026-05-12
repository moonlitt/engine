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

    // Pianoteq is the canonical case — physical-model synth whose default
    // state is immediately playable. See `every_default_audible_plugin_*`
    // below for the matrix-driven version that runs this against every
    // installed plug-in.
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

/// For every installed plug-in whose default state already produces
/// audio (verified by a quick warm-up render), prove that capturing its
/// state, loading a fresh instance, and applying the state restores the
/// audio. This is the real DAW-parity test — saving a project must
/// reproduce the same sound on reload, for every plug-in we host.
///
/// Plug-ins that ship silent by default (Keyscape, Omnisphere, sfizz
/// with no .sfz loaded) are intentionally skipped here. Their state
/// roundtrip is exercised by [keyscape_probe.rs] once a GUI-captured
/// fixture is in place.
#[test]
fn every_default_audible_plugin_round_trips_audio() {
    let _g = plugin_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    if plugins.is_empty() {
        eprintln!("No VST3 plug-ins installed — skipping matrix");
        return;
    }

    let mut covered = Vec::new();
    let mut skipped_silent = Vec::new();

    for info in &plugins {
        // First pass: does it produce audio in default state?
        let mut warm = host.load(info).unwrap();
        warm.note_on(0, 60, 100);
        warm.note_on(0, 64, 100);
        let warm_peak = peak_after_blocks(&mut warm, 16);
        if warm_peak < 1e-3 {
            skipped_silent.push(info.name.clone());
            continue;
        }
        let state = warm.get_state().expect("warm get_state");
        drop(warm);
        assert!(
            !state.is_empty(),
            "{}: default-audible plug-in must export non-empty state",
            info.name
        );

        // Second pass: fresh instance, apply state, verify audio.
        let mut fresh = host.load(info).unwrap();
        fresh.set_state(&state).unwrap_or_else(|e| {
            panic!("{}: set_state failed for self-captured state: {e}", info.name)
        });
        fresh.note_on(0, 60, 100);
        fresh.note_on(0, 64, 100);
        let restored_peak = peak_after_blocks(&mut fresh, 16);
        assert!(
            restored_peak > 1e-3,
            "{}: produced audio in default state (peak {warm_peak}) but silent after \
             state roundtrip (peak {restored_peak}) — state pipeline corrupting playable patch",
            info.name
        );
        covered.push((info.name.clone(), warm_peak, restored_peak));
    }

    println!(
        "\n  state-roundtrip audio verified for {} plug-in(s):",
        covered.len()
    );
    for (name, warm, restored) in &covered {
        println!(
            "    {name:<22} default={warm:.3}  restored={restored:.3}"
        );
    }
    if !skipped_silent.is_empty() {
        println!(
            "  skipped (silent in default state — needs GUI patch + fixture): {}\n",
            skipped_silent.join(", ")
        );
    }
}
