//! End-to-end tests: Engine -> Runtime -> real audio output.
//!
//! These tests verify the full pipeline with real plugins.
//! They skip gracefully if:
//! - The required plugin is not installed
//! - No audio output device is available

use moonlitt_engine::engine::Engine;
use moonlitt_runtime::Runtime;
use std::thread;
use std::time::Duration;

const SF2_PATH: &str = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
const VST3_PATH: &str = "/Library/Audio/Plug-Ins/VST3/Pianoteq 9.vst3";

fn has_file(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// Returns true if the error message indicates a missing audio device
/// (as opposed to a configuration or code bug).
fn is_no_device_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("no audio")
        || lower.contains("not available")
        || lower.contains("no device")
        || lower.contains("no output device")
}

/// Try to create and start a Runtime, returning None only if no audio device
/// is present. Panics on any other error to surface real regressions.
fn try_create_runtime(engine: Engine) -> Option<Runtime> {
    match Runtime::new(engine) {
        Ok(rt) => match rt.start() {
            Ok(()) => Some(rt),
            Err(e) => {
                if is_no_device_error(&e) {
                    eprintln!("No audio device, skipping: {e}");
                    None
                } else {
                    panic!("Runtime start failed (not a device issue): {e}");
                }
            }
        },
        Err((e, _engine)) => {
            if is_no_device_error(&e) {
                eprintln!("No audio device, skipping: {e}");
                None
            } else {
                panic!("Runtime creation failed (not a device issue): {e}");
            }
        }
    }
}

/// Full pipeline: Pianoteq VST3 -> Engine -> Runtime -> cpal audio output.
#[test]
fn e2e_pianoteq_runtime() {
    if !has_file(VST3_PATH) {
        eprintln!("Pianoteq not installed -- skipping");
        return;
    }

    let mut engine = Engine::new(44100, 256);
    engine.load(VST3_PATH).unwrap();
    assert!(engine.is_loaded());

    let info = engine.backend_info().unwrap();
    eprintln!("Backend: {} ({:?})", info.name, info.backend_type);

    let mut rt = match try_create_runtime(engine) {
        Some(rt) => rt,
        None => return,
    };

    // Play a C major chord
    rt.note_on(0, 60, 100); // C4
    rt.note_on(0, 64, 90);  // E4
    rt.note_on(0, 67, 85);  // G4
    eprintln!("Playing C major chord via Pianoteq...");

    thread::sleep(Duration::from_millis(800));

    // Release chord
    rt.note_off(0, 60);
    rt.note_off(0, 64);
    rt.note_off(0, 67);

    // Let the tail ring
    thread::sleep(Duration::from_millis(300));

    rt.shutdown();
    eprintln!("E2E Pianoteq: passed (no crash, clean shutdown)");
}

/// Polyphony stress: rapid notes through the lock-free queue.
#[test]
fn e2e_sf2_polyphony_stress() {
    if !has_file(SF2_PATH) {
        eprintln!("SF2 not found -- skipping");
        return;
    }

    let mut engine = Engine::new(44100, 256);
    engine.load(SF2_PATH).unwrap();

    let mut rt = match try_create_runtime(engine) {
        Some(rt) => rt,
        None => return,
    };

    // Fire 20 rapid notes (tests queue doesn't block or lose events)
    for note in 40..60 {
        rt.note_on(0, note, 100);
    }
    eprintln!("Sent 20 simultaneous notes");

    thread::sleep(Duration::from_millis(500));

    // Release all
    rt.all_notes_off();
    thread::sleep(Duration::from_millis(200));

    rt.shutdown();
    eprintln!("E2E polyphony stress: passed");
}

/// Volume control through the event queue.
#[test]
fn e2e_volume_control() {
    if !has_file(SF2_PATH) {
        eprintln!("SF2 not found -- skipping");
        return;
    }

    let mut engine = Engine::new(44100, 256);
    engine.load(SF2_PATH).unwrap();

    let mut rt = match try_create_runtime(engine) {
        Some(rt) => rt,
        None => return,
    };

    rt.note_on(0, 60, 100);

    // Fade volume
    for i in (0..=10).rev() {
        rt.set_volume(i as f32 / 10.0);
        thread::sleep(Duration::from_millis(50));
    }

    rt.note_off(0, 60);
    rt.shutdown();
    eprintln!("E2E volume control: passed");
}

/// Transport play/pause/stop cycle.
#[test]
fn e2e_transport_controls() {
    if !has_file(SF2_PATH) {
        eprintln!("SF2 not found -- skipping");
        return;
    }

    let mut engine = Engine::new(44100, 256);
    engine.load(SF2_PATH).unwrap();

    let mut rt = match try_create_runtime(engine) {
        Some(rt) => rt,
        None => return,
    };

    // Test transport state transitions
    assert!(!rt.is_playing());

    rt.play();
    assert!(rt.is_playing());

    rt.pause_playback();
    assert!(!rt.is_playing());

    rt.play();
    assert!(rt.is_playing());

    rt.stop_playback();
    assert!(!rt.is_playing());

    // Test tempo override
    rt.set_tempo(140.0);
    rt.set_loop(true);

    // Play a note while transport is active
    rt.play();
    rt.note_on(0, 72, 100);
    thread::sleep(Duration::from_millis(200));
    rt.note_off(0, 72);

    rt.shutdown();
    eprintln!("E2E transport controls: passed");
}
