//! Latency and tail-time reporting tests.
//!
//! VST3 plug-ins report processing latency via IAudioProcessor::getLatencySamples
//! (host should delay other signal paths to keep tracks aligned) and tail
//! ringing via getTailSamples (host should keep calling process() after the
//! last note_off so reverb/delay tails ring out). Without these, reverbs
//! get cut off, delays glitch, and FX plug-ins appear out of sync.
//!
//! These tests verify our Vst3Plugin surfaces both. We don't pin the
//! reported values — they're plug-in-internal — but we do pin the API
//! contract: every plug-in answers the query and the enum encodes the
//! VST3 constants correctly.

use moonlitt_vst3::{TailSamples, Vst3Host};

#[test]
fn pianoteq_reports_latency_and_tail() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let Some(info) = plugins.iter().find(|p| p.name.contains("Pianoteq")) else {
        eprintln!("Pianoteq not installed — skipping");
        return;
    };
    let plugin = host.load(info).unwrap();

    // Pianoteq is a physical-model synth. Latency must be non-negative.
    // Tail is plug-in-internal — Pianoteq lets per-note release handle
    // ringing rather than reporting a global tail, so it returns kNoTail.
    // We don't pin a specific tail value; just verify the API works.
    let latency = plugin.latency_samples();
    assert!(
        latency >= 0,
        "latency must be non-negative, got {latency}"
    );
    let _tail = plugin.tail_samples();
}

#[test]
fn sfizz_reports_latency_and_tail() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let Some(info) = plugins.iter().find(|p| p.name == "sfizz") else {
        eprintln!("sfizz not installed — skipping");
        return;
    };
    let plugin = host.load(info).unwrap();

    let latency = plugin.latency_samples();
    assert!(latency >= 0, "latency must be non-negative, got {latency}");

    // sfizz is a sample player; tail depends on loaded SFZ but the
    // function must still answer.
    let _ = plugin.tail_samples();
}

#[test]
fn tail_samples_enum_encodes_vst3_constants() {
    // kNoTail == 0
    assert_eq!(TailSamples::from_raw(0), TailSamples::None);
    // kInfiniteTail == u32::MAX
    assert_eq!(TailSamples::from_raw(u32::MAX), TailSamples::Infinite);
    // Any other value is Samples(n)
    assert_eq!(TailSamples::from_raw(44100), TailSamples::Samples(44100));
    assert_eq!(TailSamples::from_raw(1), TailSamples::Samples(1));
}
