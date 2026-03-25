//! Integration test: scan, load Pianoteq, play a note, verify audio output.
//!
//! This test is designed to be skipped gracefully if Pianoteq is not installed.

use moonlitt_vst3::Vst3Host;

#[test]
fn test_scan_and_load_pianoteq() {
    let host = Vst3Host::new(44100, 256).unwrap();

    // Scan for plugins
    let plugins = host.scan().unwrap();
    eprintln!("Found {} VST3 plugin(s):", plugins.len());
    for p in &plugins {
        eprintln!("  - {} ({})", p.name, p.path.display());
    }

    // Find Pianoteq
    let pianoteq = plugins.iter().find(|p| p.name.contains("Pianoteq"));
    if pianoteq.is_none() {
        eprintln!("Pianoteq not installed — skipping test");
        return;
    }
    let pianoteq_info = pianoteq.unwrap();
    eprintln!("Loading: {}", pianoteq_info.name);

    // Load the plugin
    let mut plugin = host.load(pianoteq_info).unwrap();
    eprintln!("Loaded: {}", plugin.name());

    // Send note on
    plugin.note_on(0, 60, 100);

    // Render several blocks to give the synth time to produce audio
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    let mut max_sample = 0.0f32;

    for block in 0..16 {
        plugin.render(&mut left, &mut right).unwrap();
        let block_max = left
            .iter()
            .chain(right.iter())
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        if block_max > max_sample {
            max_sample = block_max;
        }
        if block < 4 {
            eprintln!("  block {block}: max={block_max:.6}");
        }
    }

    eprintln!("Peak amplitude after 16 blocks: {max_sample:.6}");
    assert!(
        max_sample > 0.001,
        "Pianoteq should produce audio, got max={max_sample}"
    );

    // Send note off and render a few more blocks
    plugin.note_off(0, 60);
    for _ in 0..4 {
        plugin.render(&mut left, &mut right).unwrap();
    }

    eprintln!("Test passed: Pianoteq produced audio via pure Rust VST3 hosting");
}

#[test]
fn test_scan_returns_plugins() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    // This test just verifies scanning doesn't crash.
    // On a system with no VST3 plugins, it returns an empty list.
    eprintln!("Scan found {} plugin(s)", plugins.len());
}
