use moonlitt_engine::engine::Engine;
use moonlitt_runtime::Runtime;
use std::thread;
use std::time::Duration;

/// Try to create and start a Runtime, returning None if no audio device.
fn try_create_runtime(engine: Engine) -> Option<Runtime> {
    match Runtime::new(engine) {
        Ok(rt) => {
            if rt.start().is_err() {
                eprintln!("No audio device available, skipping");
                return None;
            }
            Some(rt)
        }
        Err(e) => {
            eprintln!("Runtime creation failed (no audio device?): {e}, skipping");
            None
        }
    }
}

#[test]
fn runtime_start_stop() {
    let mut engine = Engine::new(44100, 256);
    // Load SF2 for a simple test
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        return;
    }
    engine.load(sf2).unwrap();

    let mut rt = match try_create_runtime(engine) {
        Some(rt) => rt,
        None => return,
    };

    // Send a note
    rt.note_on(0, 60, 100);

    // Let it play for 1 second
    thread::sleep(Duration::from_secs(1));

    rt.note_off(0, 60);
    thread::sleep(Duration::from_millis(200));

    rt.shutdown();
}
