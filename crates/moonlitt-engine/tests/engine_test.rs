use moonlitt_engine::engine::Engine;

#[test]
fn test_engine_create() {
    let engine = Engine::new(44100, 256);
    assert!(!engine.is_loaded());
}

#[test]
fn test_engine_render_silence_when_no_backend() {
    let mut engine = Engine::new(44100, 256);
    let mut left = vec![1.0f32; 256];
    let mut right = vec![1.0f32; 256];
    engine.render(&mut left, &mut right);
    assert!(left.iter().all(|&s| s == 0.0));
    assert!(right.iter().all(|&s| s == 0.0));
}

#[test]
fn test_engine_unsupported_format() {
    let mut engine = Engine::new(44100, 256);
    let result = engine.load("file.mp3");
    assert!(result.is_err());
}

#[test]
fn test_engine_auto_detect_format() {
    let mut engine = Engine::new(44100, 256);
    // .xyz should fail with UnsupportedFormat
    assert!(engine.load("nonexistent.xyz").is_err());
}

#[test]
fn test_engine_unload_when_nothing_loaded() {
    let mut engine = Engine::new(44100, 256);
    // Should not panic
    engine.unload();
    assert!(!engine.is_loaded());
}

#[test]
fn test_engine_backend_info_none_when_empty() {
    let engine = Engine::new(44100, 256);
    assert!(engine.backend_info().is_none());
}

#[test]
fn test_engine_midi_noop_when_no_backend() {
    let mut engine = Engine::new(44100, 256);
    // Should not panic — all MIDI methods are no-ops when no backend
    engine.note_on(0, 60, 100);
    engine.note_off(0, 60);
    engine.cc(0, 64, 127);
    engine.pitch_bend(0, 0);
    engine.program_change(0, 0);
    engine.all_notes_off();
}

#[test]
fn test_engine_set_volume() {
    let mut engine = Engine::new(44100, 256);
    // Should not panic when no backend
    engine.set_volume(0.5);
}

#[test]
fn test_engine_scan_plugins() {
    let engine = Engine::new(44100, 256);
    let plugins = engine.scan_plugins();
    // Should return a list (possibly empty) without panicking
    println!("Found {} plugins", plugins.len());
    for p in &plugins {
        println!("  {} ({:?}) at {}", p.name, p.format, p.path);
    }
}

#[cfg(feature = "sf2")]
#[test]
fn test_engine_load_sf2() {
    let mut engine = Engine::new(44100, 256);
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        eprintln!("SF2 file not found, skipping test");
        return;
    }
    engine.load(sf2).unwrap();
    assert!(engine.is_loaded());

    // Check backend info
    let info = engine.backend_info().unwrap();
    assert_eq!(info.name, "FluidLite");

    // Play a note and render
    engine.note_on(0, 60, 100);
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    engine.render(&mut left, &mut right);
    let max = left
        .iter()
        .chain(right.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    assert!(max > 0.0, "SF2 should produce audio, got max={max}");
}

#[cfg(feature = "sf2")]
#[test]
fn test_engine_sf2_unload() {
    let mut engine = Engine::new(44100, 256);
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        return;
    }
    engine.load(sf2).unwrap();
    assert!(engine.is_loaded());
    engine.unload();
    assert!(!engine.is_loaded());
}

#[cfg(feature = "vst3")]
#[test]
fn test_engine_load_vst3() {
    let mut engine = Engine::new(44100, 256);
    let vst3 = "/Library/Audio/Plug-Ins/VST3/Pianoteq 9.vst3";
    if !std::path::Path::new(vst3).exists() {
        eprintln!("Pianoteq VST3 not found, skipping test");
        return;
    }
    engine.load(vst3).unwrap();
    assert!(engine.is_loaded());

    engine.note_on(0, 60, 100);
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    for _ in 0..16 {
        engine.render(&mut left, &mut right);
    }
    let max = left
        .iter()
        .chain(right.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    assert!(max > 0.001, "Pianoteq should produce audio via engine, got max={max}");
}

#[cfg(feature = "sf2")]
#[test]
fn test_engine_presets_sf2() {
    let mut engine = Engine::new(44100, 256);
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        return;
    }
    engine.load(sf2).unwrap();
    let presets = engine.presets();
    // GeneralUser_GS has many presets
    assert!(!presets.is_empty(), "SF2 should have presets");
    println!("Found {} SF2 presets", presets.len());
    for p in presets.iter().take(5) {
        println!("  [{}] {}", p.id, p.name);
    }
}
