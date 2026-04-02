use moonlitt_core::AudioBackend;

#[test]
fn test_create_unsupported_format() {
    let result = moonlitt_engine::create("file.mp3", 44100, 256);
    assert!(result.is_err());
}

#[test]
fn test_create_unknown_extension() {
    // .xyz should fail with UnsupportedFormat
    assert!(moonlitt_engine::create("nonexistent.xyz", 44100, 256).is_err());
}

#[test]
fn test_supported_formats() {
    let formats = moonlitt_engine::supported_formats();
    // At least SF2 should be supported (default feature)
    assert!(formats.contains(&"sf2"), "sf2 should be in supported formats");
}

#[test]
fn test_scan_plugins() {
    let plugins = moonlitt_engine::scan_plugins(44100, 256);
    // Should return a list (possibly empty) without panicking
    println!("Found {} plugins", plugins.len());
    for p in &plugins {
        println!("  {} ({:?}) at {}", p.name, p.format, p.path);
    }
}

#[test]
fn test_null_backend_renders_silence() {
    let mut backend = moonlitt_core::NullBackend::new(44100);
    let mut left = vec![1.0f32; 256];
    let mut right = vec![1.0f32; 256];
    backend.render(&mut left, &mut right);
    assert!(left.iter().all(|&s| s == 0.0));
    assert!(right.iter().all(|&s| s == 0.0));
}

#[test]
fn test_null_backend_midi_noop() {
    let mut backend = moonlitt_core::NullBackend::new(44100);
    // Should not panic — all MIDI methods are no-ops
    backend.note_on(0, 60, 100);
    backend.note_off(0, 60);
    backend.cc(0, 64, 127);
    backend.pitch_bend(0, 0);
    backend.program_change(0, 0);
    backend.all_notes_off();
}

#[cfg(feature = "sf2")]
#[test]
fn test_create_sf2() {
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        eprintln!("SF2 file not found, skipping test");
        return;
    }
    let mut backend = moonlitt_engine::create(sf2, 44100, 256).unwrap();

    // Check backend info
    let info = backend.info();
    assert_eq!(info.name, "OxiSynth");

    // Play a note and render
    backend.note_on(0, 60, 100);
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    backend.render(&mut left, &mut right);
    let max = left
        .iter()
        .chain(right.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    assert!(max > 0.0, "SF2 should produce audio, got max={max}");
}

#[cfg(feature = "sf2")]
#[test]
fn test_create_sf2_presets() {
    let sf2 = "/Users/wangyan/Desktop/stardew valley mods/mods/piano-block/assets/soundfonts/GeneralUser_GS.sf2";
    if !std::path::Path::new(sf2).exists() {
        return;
    }
    let backend = moonlitt_engine::create(sf2, 44100, 256).unwrap();
    let presets = backend.presets();
    // GeneralUser_GS has many presets
    assert!(!presets.is_empty(), "SF2 should have presets");
    println!("Found {} SF2 presets", presets.len());
    for p in presets.iter().take(5) {
        println!("  [{}] {}", p.id, p.name);
    }
}

#[cfg(feature = "vst3")]
#[test]
fn test_create_vst3() {
    let vst3 = "/Library/Audio/Plug-Ins/VST3/Pianoteq 9.vst3";
    if !std::path::Path::new(vst3).exists() {
        eprintln!("Pianoteq VST3 not found, skipping test");
        return;
    }
    let mut backend = moonlitt_engine::create(vst3, 44100, 256).unwrap();

    backend.note_on(0, 60, 100);
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    for _ in 0..16 {
        backend.render(&mut left, &mut right);
    }
    let max = left
        .iter()
        .chain(right.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    assert!(max > 0.001, "Pianoteq should produce audio, got max={max}");
}
