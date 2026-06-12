//! Plug-in metadata classification tests.
//!
//! DAWs split their plug-in browser into "Instruments" vs "Effects" using
//! the VST3 subCategories string (e.g. "Instrument|Synth", "Fx|Reverb",
//! "Fx|Dynamics"). Without this metadata the host can't know how to route
//! a freshly-instantiated plug-in (does it want MIDI in? audio in? both?).

use moonlitt_vst3::{PluginKind, Vst3Host};

#[test]
fn scanned_plugins_carry_subcategories_vendor_version() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    if plugins.is_empty() {
        eprintln!("No VST3 plug-ins installed — skipping");
        return;
    }

    for info in &plugins {
        eprintln!(
            "  scanned: name={:?} sub={:?} vendor={:?} version={:?}",
            info.name, info.subcategories, info.vendor, info.version
        );
    }

    for info in &plugins {
        // PClassInfo2 fields are optional in the spec but every modern
        // plug-in fills them. We only require that scan() populates them
        // when the factory returns IPluginFactory2 / IPluginFactory3
        // (sane modern plug-ins do).
        assert!(
            info.subcategories.is_some(),
            "{} has no subcategories — getClassInfo2 returned nothing",
            info.name
        );
        assert!(info.vendor.is_some(), "{} has no vendor", info.name);
    }
}

#[test]
fn classify_pianoteq_as_instrument() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let Some(p) = plugins.iter().find(|p| p.name.contains("Pianoteq")) else {
        eprintln!("Pianoteq not installed — skipping");
        return;
    };
    assert_eq!(
        p.kind(),
        PluginKind::Instrument,
        "Pianoteq subcategories=\"{:?}\" but classified as {:?}",
        p.subcategories,
        p.kind()
    );
}

#[test]
fn classify_surge_as_instrument() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let Some(p) = plugins.iter().find(|p| p.name == "Surge") else {
        eprintln!("Surge not installed — skipping");
        return;
    };
    assert_eq!(p.kind(), PluginKind::Instrument);
}

#[test]
fn classify_surge_fx_as_effect() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let Some(p) = plugins.iter().find(|p| p.name == "SurgeEffectsBank") else {
        eprintln!("SurgeEffectsBank not installed — skipping");
        return;
    };
    assert_eq!(p.kind(), PluginKind::Effect);
}

#[test]
fn plugin_kind_derives_from_subcategories() {
    use moonlitt_vst3::PluginInfo;
    use std::path::PathBuf;

    let make = |sub: &str| PluginInfo {
        name: "test".into(),
        path: PathBuf::new(),
        class_id: [0u8; 16],
        category: "Audio Module Class".into(),
        subcategories: Some(sub.into()),
        vendor: None,
        version: None,
    };

    assert_eq!(make("Instrument|Synth").kind(), PluginKind::Instrument);
    assert_eq!(make("Fx|Reverb").kind(), PluginKind::Effect);
    assert_eq!(make("Instrument").kind(), PluginKind::Instrument);
    assert_eq!(make("Fx").kind(), PluginKind::Effect);
    assert_eq!(make("Fx|Filter|Modulation").kind(), PluginKind::Effect);
    // Unknown / empty fall back to Unknown.
    assert_eq!(make("").kind(), PluginKind::Unknown);
    assert_eq!(make("Analyzer").kind(), PluginKind::Unknown);
}
