use moonlitt_engine::engine::Engine;
use moonlitt_runtime::Runtime;
use std::thread;
use std::time::Duration;

#[test]
fn runtime_start_stop() {
    let mut engine = Engine::new(44100, 256);
    // Load SF2 for a simple test
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        return;
    }
    engine.load(sf2).unwrap();

    let mut rt = Runtime::new(engine).unwrap();
    rt.start().unwrap();

    // Send a note
    rt.note_on(0, 60, 100);

    // Let it play for 1 second
    thread::sleep(Duration::from_secs(1));

    rt.note_off(0, 60);
    thread::sleep(Duration::from_millis(200));

    rt.shutdown();
}
