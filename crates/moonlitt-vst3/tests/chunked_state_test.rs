//! Integration tests for the new chunked state format.
//!
//! These tests pin two contracts:
//!
//!   1. Every `Vst3Plugin::get_state()` output is a parseable `ChunkedState`
//!      (i.e., the new MLST-prefixed format — NOT legacy single-blob).
//!   2. Legacy single-blob fixtures still load — `set_state` falls back to
//!      treating them as component-only state. Existing roundtrip tests in
//!      the project depended on this implicit invariant.

use moonlitt_vst3::{ChunkedState, Vst3Host};

/// Both tests in this binary load real plug-ins; cargo runs them in
/// parallel threads. Spectrasonics' STEAM library SIGSEGVs when two
/// instances load simultaneously in one process (same hazard
/// keyscape_probe serialises against), so serialise here too.
fn plugin_load_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[test]
fn get_state_emits_chunked_format_for_every_installed_plugin() {
    let _serial = plugin_load_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    if plugins.is_empty() {
        eprintln!("No VST3 plug-ins installed — skipping");
        return;
    }

    let mut audited = 0usize;
    for info in &plugins {
        // Keyscape needs serialization (per keyscape_lock) but here we just
        // load + get_state once per plug-in, which is single-instance.
        let plugin = match host.load(info) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("load {}: {e} — skipping", info.name);
                continue;
            }
        };
        let bytes = plugin.get_state().expect("get_state must succeed");
        let parsed = ChunkedState::parse(&bytes).unwrap_or_else(|| {
            panic!(
                "get_state output for {} did not parse as chunked container ({} bytes)",
                info.name,
                bytes.len()
            )
        });
        // Component state may be empty for plug-ins that have no internal
        // DSP state to persist (rare but legal). Controller state is also
        // permitted to be empty. The only hard requirement is that the
        // container is well-formed.
        eprintln!(
            "  {:24} component={:>7}B controller={:>7}B",
            info.name,
            parsed.component.len(),
            parsed.controller.len()
        );
        audited += 1;
    }
    assert!(audited > 0, "no plug-ins were audited");
    eprintln!("chunked-state format verified across {audited} plug-in(s)");
}

#[test]
fn set_state_accepts_legacy_single_blob_for_back_compat() {
    let _serial = plugin_load_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let Some(info) = plugins.iter().find(|p| p.name == "Surge") else {
        eprintln!("Surge not installed — skipping back-compat test");
        return;
    };
    let mut plugin = host.load(info).unwrap();

    // Synthesize a "legacy" payload by extracting just the component chunk
    // and feeding it back unwrapped. set_state must treat it as old-style
    // component-only state, not error out.
    let chunked_bytes = plugin.get_state().unwrap();
    let chunked = ChunkedState::parse(&chunked_bytes).unwrap();
    let legacy_blob = chunked.component;

    // Sanity: legacy_blob does NOT begin with MLST.
    assert_ne!(
        &legacy_blob.get(..4).unwrap_or(&[]),
        b"MLST",
        "extracted component chunk should not itself be MLST-tagged"
    );

    plugin
        .set_state(&legacy_blob)
        .expect("legacy single-blob fixtures must remain loadable");
}
