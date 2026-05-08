//! Integration test: multi-bus output API.
//!
//! Verifies that `render_all` + `bus_output` work end-to-end on a real
//! plugin: bus topology is reported, audio actually arrives in bus 0's
//! scratch buffer, and the contract holds across both single-out and
//! multi-out plugins. Falls back gracefully when no plugins are
//! installed.

use std::sync::{Mutex, MutexGuard, OnceLock};

use moonlitt_vst3::Vst3Host;

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |a, &x| a.max(x.abs()))
}

/// Serialize plugin instantiation across tests in this file. Several real
/// plugins (Pianoteq, Spectrasonics, etc.) keep process-wide globals that
/// crash when two instances spin up on parallel test threads. cargo's
/// default thread-per-test scheduler hits this; this guard keeps each
/// `Vst3Host::load` exclusive.
fn plugin_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn audio_output_topology_is_exposed() {
    let _g = plugin_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();

    // Pick the first instrument we can load.
    let info = plugins
        .iter()
        .find(|p| p.name.contains("Pianoteq") || p.name.contains("Surge"));
    let Some(info) = info else {
        eprintln!("No suitable instrument plugin installed — skipping");
        return;
    };

    let plugin = host.load(info).unwrap();
    let buses = plugin.audio_output_buses();
    assert!(!buses.is_empty(), "plugin must declare at least one audio output bus");
    assert_eq!(plugin.audio_output_bus_count(), buses.len());
    assert!(
        plugin.audio_output_bus_info(0).is_some(),
        "bus 0 info should be retrievable"
    );
    assert!(
        plugin.audio_output_bus_info(buses.len()).is_none(),
        "out-of-range bus info should be None"
    );
    eprintln!(
        "Plugin '{}' exposes {} audio output bus(es):",
        plugin.name(),
        buses.len()
    );
    for (i, b) in buses.iter().enumerate() {
        eprintln!(
            "  bus {i}: name=\"{}\" channels={} main={} default_active={}",
            b.name, b.channel_count, b.is_main, b.default_active
        );
    }
}

#[test]
fn render_all_populates_bus_output_scratch() {
    let _g = plugin_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();

    let info = plugins
        .iter()
        .find(|p| p.name.contains("Pianoteq") || p.name.contains("Surge"));
    let Some(info) = info else {
        eprintln!("No suitable instrument plugin installed — skipping");
        return;
    };

    let mut plugin = host.load(info).unwrap();
    plugin.note_on(0, 60, 100);

    let mut peaked = false;
    for _ in 0..32 {
        plugin.render_all().unwrap();
        if let Some((l, r)) = plugin.bus_output(0) {
            if peak(l).max(peak(r)) > 1e-3 {
                peaked = true;
                break;
            }
        }
    }
    assert!(peaked, "bus 0 produced no audio under render_all");
}

#[test]
fn render_one_and_render_all_agree_on_bus_zero() {
    let _g = plugin_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();

    let info = plugins
        .iter()
        .find(|p| p.name.contains("Pianoteq") || p.name.contains("Surge"));
    let Some(info) = info else {
        eprintln!("No suitable instrument plugin installed — skipping");
        return;
    };

    // Two parallel plugin instances so we can compare their outputs
    // without the second render seeing the first's note state.
    let mut a = host.load(info).unwrap();
    let mut b = host.load(info).unwrap();
    a.note_on(0, 60, 100);
    b.note_on(0, 60, 100);

    // Burn a few blocks of warmup so attack envelopes settle similarly.
    let mut l = vec![0.0f32; 256];
    let mut r = vec![0.0f32; 256];
    for _ in 0..4 {
        a.render(&mut l, &mut r).unwrap();
        b.render_all().unwrap();
    }

    let (bl, br) = b.bus_output(0).unwrap();
    let pa = peak(&l).max(peak(&r));
    let pb = peak(bl).max(peak(br));

    // Same plugin + same MIDI + same warmup → both paths should produce
    // signal in the same order of magnitude. Allow generous slack since
    // sample-level reproducibility is not guaranteed across instances.
    assert!(pa > 1e-3 && pb > 1e-3, "both paths must produce audio (a={pa}, b={pb})");
    assert!(
        (pa - pb).abs() / pa.max(pb) < 0.5,
        "render and render_all bus 0 peaks diverge: a={pa} b={pb}"
    );
}
