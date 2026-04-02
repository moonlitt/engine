use moonlitt_core::AudioBackend;
use moonlitt_audio_io::Runtime;
use std::thread;
use std::time::Duration;

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
fn try_create_runtime(backend: Box<dyn AudioBackend>, sample_rate: u32, buffer_size: u32) -> Option<Runtime> {
    match Runtime::new(backend, sample_rate, buffer_size) {
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
        Err((e, _backend)) => {
            if is_no_device_error(&e) {
                eprintln!("No audio device, skipping: {e}");
                None
            } else {
                panic!("Runtime creation failed (not a device issue): {e}");
            }
        }
    }
}

#[test]
fn runtime_start_stop() {
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        return;
    }

    let backend = moonlitt_engine::create(sf2, 44100, 256).unwrap();

    let mut rt = match try_create_runtime(backend, 44100, 256) {
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
