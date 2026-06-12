//! RT-safety: the audio-thread paths of `Vst3Backend` must never wait
//! on the shared plugin mutex. When the GUI side holds it (a streamer's
//! `set_state` can take ~1 s), render outputs silence, effects pass
//! input through, events are dropped — and every miss is counted.
//!
//! Skips when Pianoteq isn't installed (test name carries "pianoteq"
//! so CI filters it).

#![cfg(feature = "vst3")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use moonlitt_core::AudioBackend;
use moonlitt_engine::backends::vst3::Vst3Backend;

fn find_pianoteq_path() -> Option<PathBuf> {
    let host = moonlitt_vst3::Vst3Host::new(44100, 256).ok()?;
    let plugins = host.scan().ok()?;
    plugins
        .into_iter()
        .find(|p| p.name.to_lowercase().contains("pianoteq"))
        .map(|info| info.path)
}

#[test]
fn pianoteq_render_under_gui_lock_is_silent_not_stalled() {
    let Some(path) = find_pianoteq_path() else {
        eprintln!("Pianoteq not installed — skipping");
        return;
    };
    let mut backend = Vst3Backend::new(44100, 256).expect("backend");
    backend.load(&path.to_string_lossy()).expect("load");

    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];

    // Baseline: uncontended render works.
    backend.note_on(0, 60, 100);
    for _ in 0..16 {
        backend.render(&mut left, &mut right);
    }

    // The GUI side grabs the plugin for a slow operation on another
    // thread (simulates set_state / create_view).
    let gui = backend.plugin_handle().expect("plugin handle");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _g = gui.lock();
        started_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(400));
    });
    started_rx.recv().unwrap();

    // Audio path while the lock is held elsewhere: must return fast and
    // silent — never stall the device callback.
    left.fill(0.7);
    right.fill(0.7);
    let t0 = Instant::now();
    backend.render(&mut left, &mut right);
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "render must not wait for the GUI lock (took {elapsed:?})"
    );
    assert!(
        left.iter().chain(right.iter()).all(|&s| s == 0.0),
        "contended render must output silence"
    );

    // Events during the window are dropped, not blocked on.
    let t0 = Instant::now();
    backend.note_on(0, 64, 100);
    backend.note_off(0, 64);
    assert!(
        t0.elapsed() < Duration::from_millis(100),
        "event dispatch must not wait for the GUI lock"
    );

    // Effects pass input through instead of going silent.
    let in_l = vec![0.5f32; 256];
    let in_r = vec![0.5f32; 256];
    let mut out_l = vec![0.0f32; 256];
    let mut out_r = vec![0.0f32; 256];
    backend.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);
    assert_eq!(out_l, in_l, "contended effect must pass input through");
    assert_eq!(out_r, in_r, "contended effect must pass input through");

    // Every miss was counted (render + 2 events + 1 effect).
    assert!(
        backend.lock_contentions() >= 4,
        "contention must be counted (got {})",
        backend.lock_contentions()
    );

    holder.join().unwrap();

    // And the path recovers immediately once the GUI lets go.
    let before = backend.lock_contentions();
    backend.render(&mut left, &mut right);
    assert_eq!(
        backend.lock_contentions(),
        before,
        "uncontended render must not count a miss"
    );
}
