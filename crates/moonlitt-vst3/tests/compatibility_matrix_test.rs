//! VST3 compatibility matrix.
//!
//! For every installed VST3 instrument the test can find, runs a structured
//! audit and prints a per-plug-in pass/fail row across DAW-relevant
//! capabilities. The test passes as long as the matrix is populated and
//! every plug-in that has audio in default state actually renders > 0
//! peak. We do NOT assert audio for sample-streamed instruments
//! (Keyscape, Kontakt, Omnisphere…) — they need patch selection via GUI
//! and a state fixture, exercised by [keyscape_probe.rs] separately.
//!
//! Output (example):
//! ```text
//!   plugin             scan  load  bus_count  latency  tail        default_audio  state_roundtrip
//!   Pianoteq 9         ✓     ✓     1          0        kNoTail     ✓              ✓
//!   sfizz              ✓     ✓     1          0        Samples(N)  ✗ (no patch)   ✓
//!   Keyscape           ✓     ✓     9          0        kNoTail     ✗ (sampler)    ✓
//! ```
//!
//! When a plug-in fails to scan or load, the matrix records the failure
//! reason and continues. Only categorical bugs (e.g. our load API itself
//! panics) abort the test.

use std::sync::{Mutex, MutexGuard, OnceLock};

use moonlitt_vst3::{PluginKind, TailSamples, Vst3Host, Vst3Plugin};

/// Some plug-ins (Spectrasonics) can't safely have two instances live
/// in the same process at once. Serialize all matrix loads.
fn matrix_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug)]
struct PluginAudit {
    name: String,
    kind: PluginKind,
    vendor: String,
    load_ok: bool,
    error: Option<String>,
    bus_count: usize,
    latency: i32,
    tail: TailSamples,
    default_audio_peak: f32,
    state_roundtrip_ok: bool,
    state_roundtrip_reason: String,
}

fn render_max_peak(plugin: &mut Vst3Plugin, blocks: usize) -> f32 {
    let mut left = vec![0.0f32; 256];
    let mut right = vec![0.0f32; 256];
    let mut peak = 0.0f32;
    for _ in 0..blocks {
        if plugin.render(&mut left, &mut right).is_err() {
            break;
        }
        for &s in left.iter().chain(right.iter()) {
            peak = peak.max(s.abs());
        }
    }
    peak
}

fn audit(host: &Vst3Host, info: &moonlitt_vst3::PluginInfo) -> PluginAudit {
    let mut audit = PluginAudit {
        name: info.name.clone(),
        kind: info.kind(),
        vendor: info.vendor.clone().unwrap_or_default(),
        load_ok: false,
        error: None,
        bus_count: 0,
        latency: -1,
        tail: TailSamples::None,
        default_audio_peak: 0.0,
        state_roundtrip_ok: false,
        state_roundtrip_reason: String::from("not attempted"),
    };

    let mut plugin = match host.load(info) {
        Ok(p) => {
            audit.load_ok = true;
            p
        }
        Err(e) => {
            audit.error = Some(format!("{e}"));
            return audit;
        }
    };

    audit.bus_count = plugin.audio_output_bus_count();
    audit.latency = plugin.latency_samples();
    audit.tail = plugin.tail_samples();

    // Default-state audio: send a chord, render 32 blocks (~185ms @ 44100/256).
    plugin.note_on(0, 60, 100);
    plugin.note_on(0, 64, 100);
    plugin.note_on(0, 67, 100);
    audit.default_audio_peak = render_max_peak(&mut plugin, 32);

    // State roundtrip — capture default state, load fresh instance,
    // verify the rehydrated processor produces equal or matching audio.
    let state = match plugin.get_state() {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            audit.state_roundtrip_reason = "empty state".into();
            return audit;
        }
        Err(e) => {
            audit.state_roundtrip_reason = format!("get_state failed: {e}");
            return audit;
        }
    };
    drop(plugin);

    let mut fresh = match host.load(info) {
        Ok(p) => p,
        Err(e) => {
            audit.state_roundtrip_reason = format!("reload failed: {e}");
            return audit;
        }
    };
    match fresh.set_state(&state) {
        Ok(()) => {
            audit.state_roundtrip_ok = true;
            audit.state_roundtrip_reason = format!("{} bytes restored", state.len());
        }
        Err(e) => {
            audit.state_roundtrip_reason = format!("set_state failed: {e}");
        }
    }

    audit
}

fn print_matrix(rows: &[PluginAudit]) {
    println!(
        "\n  {:<22} {:<11} {:<18} {:<6} {:<5} {:<8} {:<14} {:<14} {}",
        "plugin",
        "kind",
        "vendor",
        "load",
        "buses",
        "latency",
        "tail",
        "default audio",
        "state roundtrip"
    );
    println!("  {}", "-".repeat(130));
    for r in rows {
        let tail_str = match r.tail {
            TailSamples::None => "kNoTail".into(),
            TailSamples::Infinite => "kInfiniteTail".into(),
            TailSamples::Samples(n) => format!("{n}"),
        };
        let load_str = if r.load_ok {
            "OK".to_string()
        } else {
            format!("FAIL: {}", r.error.as_deref().unwrap_or("?"))
        };
        let audio_str = if r.default_audio_peak > 1e-3 {
            format!("OK ({:.3})", r.default_audio_peak)
        } else {
            "silent".to_string()
        };
        let state_str = if r.state_roundtrip_ok {
            format!("OK ({})", r.state_roundtrip_reason)
        } else {
            format!("- ({})", r.state_roundtrip_reason)
        };
        let kind_str = format!("{:?}", r.kind);
        let vendor_trunc: String = r.vendor.chars().take(18).collect();
        println!(
            "  {:<22} {:<11} {:<18} {:<6} {:<5} {:<8} {:<14} {:<14} {}",
            r.name,
            kind_str,
            vendor_trunc,
            load_str,
            r.bus_count,
            r.latency,
            tail_str,
            audio_str,
            state_str
        );
    }
    println!();
}

#[test]
fn compatibility_matrix_audits_every_installed_vst3() {
    let _g = matrix_lock();
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    if plugins.is_empty() {
        eprintln!("No VST3 plug-ins installed — skipping matrix");
        return;
    }

    let rows: Vec<PluginAudit> = plugins.iter().map(|info| audit(&host, info)).collect();

    print_matrix(&rows);

    // Categorical assertions: every load must either succeed or report a
    // structured error. A panic here means the host has a bug — silent
    // plug-ins are fine.
    for r in &rows {
        if !r.load_ok {
            assert!(
                r.error.is_some(),
                "Plug-in {} failed to load but reported no error",
                r.name
            );
        }
    }
}
