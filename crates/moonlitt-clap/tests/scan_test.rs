//! Integration test — scan for CLAP plugins on the system.

use moonlitt_clap::ClapHost;

#[test]
fn scan_system_clap_plugins() {
    let host = ClapHost::new(44100, 256).expect("failed to create host");
    let plugins = host.scan().expect("scan failed");

    // Print what we found (visible with `cargo test -- --nocapture`)
    if plugins.is_empty() {
        println!("No CLAP plugins found on this system.");
    } else {
        println!("Found {} CLAP plugin(s):", plugins.len());
        for p in &plugins {
            println!("  {} ({})", p.name, p.plugin_id);
            println!("    path: {}", p.path.display());
            if !p.vendor.is_empty() {
                println!("    vendor: {}", p.vendor);
            }
        }
    }

    // This test always passes — we're just checking that scan doesn't crash.
    // If CLAP plugins are installed, we also verify the data looks sane.
    for p in &plugins {
        assert!(!p.name.is_empty(), "plugin name should not be empty");
        assert!(!p.plugin_id.is_empty(), "plugin id should not be empty");
        assert!(p.path.exists(), "plugin path should exist");
    }
}

#[test]
fn host_can_be_created() {
    let host = ClapHost::new(48000, 512);
    assert!(host.is_ok());
}

/// If a CLAP plugin is available, try loading it, sending a note, and rendering.
#[test]
fn load_and_render_if_available() {
    let host = ClapHost::new(44100, 256).expect("failed to create host");
    let plugins = host.scan().expect("scan failed");

    if plugins.is_empty() {
        println!("No CLAP plugins installed — skipping load/render test.");
        return;
    }

    let info = &plugins[0];
    println!("Loading CLAP plugin: {} ({})", info.name, info.plugin_id);

    let mut plugin = match host.load(info) {
        Ok(p) => p,
        Err(e) => {
            println!("Failed to load (may need license): {e}");
            return;
        }
    };

    println!("Plugin loaded: {}", plugin.name());

    // Send a note and render
    plugin.note_on(0, 60, 100);

    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];

    // Render several buffers to let the plugin produce audio
    for _ in 0..10 {
        plugin.render(&mut left, &mut right).expect("render failed");
    }

    plugin.note_off(0, 60);

    // Render a few more for tail
    for _ in 0..5 {
        plugin.render(&mut left, &mut right).expect("render failed");
    }

    let peak = left
        .iter()
        .chain(right.iter())
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);

    println!("Peak amplitude after render: {peak:.6}");
    // We don't assert on the peak — some plugins may not produce audio
    // without a license or specific configuration.
}
