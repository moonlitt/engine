//! Diagnostic probe for Keyscape silence.
//!
//! Cycles through four hypotheses to show conclusively WHY Keyscape stays
//! silent in our host. Each variant prints its peak across all 9 audio
//! output buses over many render blocks.
//!
//!   H1  pure default: load → note_on → render. No preset, no state.
//!   H2  load_preset(0): the keyscape_test path — switch "Program 0-0".
//!   H3  long burn-in:  default → 4096 render blocks before note_on,
//!                       in case async sample streaming needs warm-up.
//!   H4  state-fixture: load → set_state(captured_blob). Requires a
//!                       fixture at tests/fixtures/keyscape-default.vstpreset
//!                       captured via the desktop GUI's "Save State" button.
//!
//! Skipped when Keyscape isn't installed.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use moonlitt_vst3::{Vst3Host, Vst3Plugin};

const NUM_BUSES: usize = 9;

/// Keyscape's STEAM sample library is not safe for parallel instances in
/// the same process — two simultaneous loads SIGSEGV on macOS. Serialize.
fn keyscape_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn peak_across_buses(plugin: &Vst3Plugin) -> Vec<f32> {
    (0..NUM_BUSES)
        .map(|b| {
            plugin
                .bus_output(b)
                .map(|(l, r)| {
                    l.iter()
                        .chain(r.iter())
                        .fold(0.0f32, |acc, &s| acc.max(s.abs()))
                })
                .unwrap_or(0.0)
        })
        .collect()
}

fn render_all_and_peak(plugin: &mut Vst3Plugin) -> Vec<f32> {
    plugin.render_all().unwrap();
    peak_across_buses(plugin)
}

fn run_blocks(plugin: &mut Vst3Plugin, blocks: usize, label: &str) -> f32 {
    let mut max_overall = 0.0f32;
    for i in 0..blocks {
        let peaks = render_all_and_peak(plugin);
        let max = peaks.iter().fold(0.0f32, |a, &p| a.max(p));
        if max > max_overall {
            max_overall = max;
        }
        if max > 1e-4 && i < 10 {
            println!(
                "  [{label}] block {i}: bus-peaks {:?}",
                peaks.iter().map(|p| format!("{p:.4}")).collect::<Vec<_>>()
            );
        }
    }
    max_overall
}

fn keyscape_info() -> Option<moonlitt_vst3::PluginInfo> {
    let host = Vst3Host::new(44100, 256).ok()?;
    let plugins = host.scan().ok()?;
    plugins.into_iter().find(|p| p.name == "Keyscape")
}

#[test]
fn keyscape_default_state_is_silent() {
    let _g = keyscape_lock();
    let Some(info) = keyscape_info() else {
        eprintln!("Keyscape not installed — skipping");
        return;
    };
    let host = Vst3Host::new(44100, 256).unwrap();

    println!("\n=== H1: pure default (no preset, no state) ===");
    let mut plugin = host.load(&info).unwrap();
    plugin.note_on(0, 60, 100);
    plugin.note_on(0, 64, 100);
    plugin.note_on(0, 67, 100);
    let peak = run_blocks(&mut plugin, 64, "H1");
    println!("  H1 max peak across 64 blocks × 9 buses: {peak:.6}");
}

#[test]
fn keyscape_with_program_change_is_silent() {
    let _g = keyscape_lock();
    let Some(info) = keyscape_info() else {
        eprintln!("Keyscape not installed — skipping");
        return;
    };
    let host = Vst3Host::new(44100, 256).unwrap();

    println!("\n=== H2: load_preset(0) program switch ===");
    let mut plugin = host.load(&info).unwrap();
    plugin.load_preset(0).unwrap();
    plugin.note_on(0, 60, 100);
    plugin.note_on(0, 64, 100);
    plugin.note_on(0, 67, 100);
    let peak = run_blocks(&mut plugin, 64, "H2");
    println!("  H2 max peak: {peak:.6}");
}

#[test]
fn keyscape_long_burn_in_is_silent() {
    let _g = keyscape_lock();
    let Some(info) = keyscape_info() else {
        eprintln!("Keyscape not installed — skipping");
        return;
    };
    let host = Vst3Host::new(44100, 256).unwrap();

    println!("\n=== H3: long burn-in (4096 blocks before note_on) ===");
    let mut plugin = host.load(&info).unwrap();
    let burn = run_blocks(&mut plugin, 4096, "H3 warmup");
    println!("  H3 warmup peak (no notes): {burn:.6}");
    plugin.note_on(0, 60, 100);
    plugin.note_on(0, 64, 100);
    plugin.note_on(0, 67, 100);
    let peak = run_blocks(&mut plugin, 256, "H3");
    println!("  H3 post-note peak: {peak:.6}");
}

/// Look for a Keyscape state fixture under tests/fixtures/. Accepts any of
/// the extensions the desktop "Save State" dialog produces (`.mlstate`,
/// `.bin`) plus the legacy `.vstpreset` name.
fn find_keyscape_fixture() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    for name in &[
        "keyscape-default.mlstate",
        "keyscape-default.bin",
        "keyscape-default.vstpreset",
    ] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[test]
fn keyscape_state_fixture_produces_audio() {
    let _g = keyscape_lock();
    let Some(info) = keyscape_info() else {
        eprintln!("Keyscape not installed — skipping");
        return;
    };
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let Some(fixture) = find_keyscape_fixture() else {
        let target = fixture_dir.join("keyscape-default.mlstate");
        eprintln!(
            "\n  Fixture missing in {}\n\
             \n\
             To activate this test, capture a Keyscape state from a real GUI session:\n\
             \n\
               1. Start the Tauri desktop shell:\n\
                    cd packages/desktop && pnpm tauri dev\n\
               2. In the moonlitt window: pick Keyscape as the default instrument.\n\
               3. Click '🎛 GUI', pick a patch in Keyscape's browser.\n\
               4. Click '💾 保存状态' and save to:\n\
                    {}\n\
               5. Re-run this test — it'll now assert that the rehydrated patch\n\
                  produces audio in headless mode.\n",
            fixture_dir.display(),
            target.display()
        );
        return;
    };
    let state = std::fs::read(&fixture).unwrap();

    let host = Vst3Host::new(44100, 256).unwrap();
    let mut plugin = host.load(&info).unwrap();
    plugin
        .set_state(&state)
        .expect("set_state should accept captured blob");

    // Sample-streamer patches load asynchronously after set_state. The
    // plug-in needs many process cycles before it can produce audio for
    // the restored patch. See `Vst3Plugin::warm_up` for the rationale.
    plugin.warm_up(8192).expect("warm_up must not error");

    plugin.note_on(0, 60, 100);
    plugin.note_on(0, 64, 100);
    plugin.note_on(0, 67, 100);
    let peak = run_blocks(&mut plugin, 512, "H4");
    assert!(
        peak > 1e-3,
        "captured Keyscape state should produce audio after warm-up, got peak={peak}"
    );
    println!("✅ Keyscape replays captured state → peak={peak:.4}");
}
