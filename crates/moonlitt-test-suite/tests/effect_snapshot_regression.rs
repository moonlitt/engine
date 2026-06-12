//! L4 — Snapshot regression for effect output metrics.
//!
//! For each (input fixture, effect, parameter set) tuple, this test renders
//! the audio in memory, measures it via `moonlitt-analyze`, and compares the
//! resulting metrics against a JSON baseline committed under `snapshots/`.
//!
//! Per the L1-L4 strategy: snapshots are the **last** line of defence and
//! catch incidental drift only. They presuppose that the L1 invariant tests
//! (`effect_silence_invariants.rs`, `dattorro_compliance.rs`) already proved
//! the output is *correct*; snapshots simply lock in that correctness against
//! future regressions.
//!
//! ## Updating snapshots
//!
//! `MOONLITT_UPDATE_SNAPSHOTS=1 cargo test -p moonlitt-test-suite --test effect_snapshot_regression`
//!
//! Inspect the diff carefully — a legitimate metric change must be justified.
//! If a snapshot moves because a bug was introduced, do NOT bless it; fix
//! the bug first.

use moonlitt_analyze::{analyze_stereo, Report};
use moonlitt_core::AudioBackend;
use moonlitt_effects::{Compressor, DattorroReverb, ParametricEq};
use std::f64::consts::TAU;
use std::path::PathBuf;

const SR: u32 = 44100;
const BLOCK: usize = 256;

// =============================================================================
// Tolerances — tight enough to detect regressions, loose enough to absorb
// floating-point reordering across compiler / target tweaks. Per-field.
// =============================================================================

struct Tolerance {
    /// Sample peak in dBFS (and true peak in dBTP).
    peak_db: f64,
    /// RMS in dBFS.
    rms_db: f64,
    /// EBU R128 loudness in LU/LUFS.
    loudness_lu: f64,
    /// DC offset, absolute linear value.
    dc_abs: f64,
}

const DEFAULT_TOL: Tolerance = Tolerance {
    peak_db: 0.5,
    rms_db: 0.5,
    loudness_lu: 0.5,
    dc_abs: 1e-4,
};

// =============================================================================
// Input fixtures — deterministic, reproducible
// =============================================================================

fn fixture_silence(seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let n = (SR as f32 * seconds) as usize;
    (vec![0.0; n], vec![0.0; n])
}

fn fixture_sine(freq: f64, amp: f32, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let n = (SR as f32 * seconds) as usize;
    let buf: Vec<f32> = (0..n)
        .map(|i| (i as f64 / SR as f64 * freq * TAU).sin() as f32 * amp)
        .collect();
    (buf.clone(), buf)
}

/// Deterministic pseudo-noise: linear congruential generator scaled to
/// `[-amp, amp]`. Same seed → identical sequence across runs and platforms.
fn fixture_pseudo_noise(amp: f32, seconds: f32, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let n = (SR as f32 * seconds) as usize;
    let mut state_l = seed;
    let mut state_r = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let step = |s: &mut u64| -> f32 {
        *s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // Take the high 32 bits and map to [0, 1) — full range, zero-mean after centring.
        let bits = (*s >> 32) as u32;
        let v = bits as f32 / (u32::MAX as f32);
        (v * 2.0 - 1.0) * amp
    };
    let l: Vec<f32> = (0..n).map(|_| step(&mut state_l)).collect();
    let r: Vec<f32> = (0..n).map(|_| step(&mut state_r)).collect();
    (l, r)
}

// =============================================================================
// Pipeline runner — block-process and measure
// =============================================================================

fn render_through<E: AudioBackend>(
    effect: &mut E,
    in_l: &[f32],
    in_r: &[f32],
) -> (Vec<f32>, Vec<f32>) {
    let n = in_l.len();
    let mut out_l = vec![0.0f32; n];
    let mut out_r = vec![0.0f32; n];
    let mut i = 0;
    while i < n {
        let end = (i + BLOCK).min(n);
        effect.process_effect(
            &in_l[i..end],
            &in_r[i..end],
            &mut out_l[i..end],
            &mut out_r[i..end],
        );
        i = end;
    }
    (out_l, out_r)
}

fn measure(out_l: &[f32], out_r: &[f32]) -> Report {
    analyze_stereo(out_l, out_r, SR).expect("analyze_stereo should not fail on finite input")
}

// =============================================================================
// Snapshot I/O
// =============================================================================

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snapshots")
}

fn snapshot_path(name: &str) -> PathBuf {
    snapshot_dir().join(format!("{name}.json"))
}

fn load_snapshot(name: &str) -> Option<Report> {
    let path = snapshot_path(name);
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("failed to parse snapshot {}: {e}", path.display()))
}

fn write_snapshot(name: &str, report: &Report) {
    let path = snapshot_path(name);
    std::fs::create_dir_all(snapshot_dir()).expect("create snapshot dir");
    std::fs::write(&path, report.to_json())
        .unwrap_or_else(|e| panic!("failed to write snapshot {}: {e}", path.display()));
    eprintln!("wrote snapshot {}", path.display());
}

fn updating() -> bool {
    std::env::var("MOONLITT_UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

// =============================================================================
// Comparison — per-field tolerance, all violations reported in one panic.
// =============================================================================

fn compare(name: &str, current: &Report, baseline: &Report, tol: &Tolerance) {
    let mut diffs: Vec<String> = Vec::new();

    let mut check_db = |label: &str, cur: f64, base: f64, allowed: f64| {
        // dBFS values can be -∞; treat both being non-finite as equal.
        if !cur.is_finite() && !base.is_finite() {
            return;
        }
        if (cur - base).abs() > allowed {
            diffs.push(format!(
                "  {label:<24}  baseline {base:>10.4}   current {cur:>10.4}   Δ {:+.4} > {allowed}",
                cur - base
            ));
        }
    };

    check_db(
        "peak L (dBFS)",
        current.peak.sample_peak_l_dbfs,
        baseline.peak.sample_peak_l_dbfs,
        tol.peak_db,
    );
    check_db(
        "peak R (dBFS)",
        current.peak.sample_peak_r_dbfs,
        baseline.peak.sample_peak_r_dbfs,
        tol.peak_db,
    );
    check_db(
        "true peak L (dBTP)",
        current.peak.true_peak_l_dbtp,
        baseline.peak.true_peak_l_dbtp,
        tol.peak_db,
    );
    check_db(
        "true peak R (dBTP)",
        current.peak.true_peak_r_dbtp,
        baseline.peak.true_peak_r_dbtp,
        tol.peak_db,
    );
    check_db(
        "rms L (dBFS)",
        current.rms.l_dbfs,
        baseline.rms.l_dbfs,
        tol.rms_db,
    );
    check_db(
        "rms R (dBFS)",
        current.rms.r_dbfs,
        baseline.rms.r_dbfs,
        tol.rms_db,
    );
    check_db(
        "integrated (LUFS)",
        current.loudness.integrated_lufs,
        baseline.loudness.integrated_lufs,
        tol.loudness_lu,
    );
    check_db(
        "short-term max (LUFS)",
        current.loudness.short_term_max_lufs,
        baseline.loudness.short_term_max_lufs,
        tol.loudness_lu,
    );
    check_db(
        "momentary max (LUFS)",
        current.loudness.momentary_max_lufs,
        baseline.loudness.momentary_max_lufs,
        tol.loudness_lu,
    );
    check_db(
        "loudness range (LU)",
        current.loudness.lra_lu,
        baseline.loudness.lra_lu,
        tol.loudness_lu,
    );

    let mut check_abs = |label: &str, cur: f64, base: f64, allowed: f64| {
        if (cur - base).abs() > allowed {
            diffs.push(format!(
                "  {label:<24}  baseline {base:>+12.6}   current {cur:>+12.6}   Δ {:+.6} > {allowed}",
                cur - base
            ));
        }
    };
    check_abs(
        "dc offset L",
        current.anomalies.dc_offset_l,
        baseline.anomalies.dc_offset_l,
        tol.dc_abs,
    );
    check_abs(
        "dc offset R",
        current.anomalies.dc_offset_r,
        baseline.anomalies.dc_offset_r,
        tol.dc_abs,
    );

    // Anomaly counts must match exactly.
    if current.anomalies.nan_count != baseline.anomalies.nan_count {
        diffs.push(format!(
            "  nan_count               baseline {}  current {}",
            baseline.anomalies.nan_count, current.anomalies.nan_count
        ));
    }
    if current.anomalies.inf_count != baseline.anomalies.inf_count {
        diffs.push(format!(
            "  inf_count               baseline {}  current {}",
            baseline.anomalies.inf_count, current.anomalies.inf_count
        ));
    }

    if !diffs.is_empty() {
        panic!(
            "snapshot drift in `{name}`:\n{}\n\nIf this change is intentional, regenerate with:\n  \
             MOONLITT_UPDATE_SNAPSHOTS=1 cargo test -p moonlitt-test-suite --test effect_snapshot_regression",
            diffs.join("\n")
        );
    }
}

fn check_or_update(name: &str, report: Report, tol: &Tolerance) {
    if updating() {
        write_snapshot(name, &report);
        return;
    }
    let baseline = load_snapshot(name).unwrap_or_else(|| {
        panic!(
            "missing snapshot `{name}`. Generate with:\n  \
             MOONLITT_UPDATE_SNAPSHOTS=1 cargo test -p moonlitt-test-suite --test effect_snapshot_regression"
        );
    });
    compare(name, &report, &baseline, tol);
}

// =============================================================================
// Snapshots
// =============================================================================

#[test]
fn snapshot_dattorro_default_sine_440() {
    let mut rev = DattorroReverb::new(SR);
    // Default params (decay=0.5, damping=0.5, dry_wet=0.5, etc.)
    let (in_l, in_r) = fixture_sine(440.0, 0.5, 4.0);
    let (out_l, out_r) = render_through(&mut rev, &in_l, &in_r);
    let report = measure(&out_l, &out_r);
    check_or_update("dattorro_default_sine_440", report, &DEFAULT_TOL);
}

#[test]
fn snapshot_dattorro_user_chain_sine_440() {
    // The exact configuration that surfaced the historical DC bug.
    let mut rev = DattorroReverb::new(SR);
    rev.set_param(1, 0.6); // PARAM_DECAY
    rev.set_param(7, 0.2); // PARAM_DRY_WET
    let (in_l, in_r) = fixture_sine(440.0, 0.5, 4.0);
    let (out_l, out_r) = render_through(&mut rev, &in_l, &in_r);
    let report = measure(&out_l, &out_r);
    check_or_update("dattorro_user_chain_sine_440", report, &DEFAULT_TOL);
}

#[test]
fn snapshot_compressor_minus18_4to1_sine() {
    // Hot signal hitting a typical mix-bus compressor.
    let mut comp = Compressor::new(SR);
    comp.set_param(0, -18.0); // threshold
    comp.set_param(1, 4.0); // ratio
    comp.set_param(2, 5.0); // attack ms
    comp.set_param(3, 100.0); // release ms
    comp.set_param(4, 0.0); // knee
    comp.set_param(5, 0.0); // makeup
    let (in_l, in_r) = fixture_sine(440.0, 0.7, 4.0);
    let (out_l, out_r) = render_through(&mut comp, &in_l, &in_r);
    let report = measure(&out_l, &out_r);
    check_or_update("compressor_minus18_4to1_sine", report, &DEFAULT_TOL);
}

#[test]
fn snapshot_eq_default_pseudo_noise() {
    // Default ParametricEq is bypass-ish (all bands flat). Pseudo-noise gives
    // a broad spectrum so any future EQ-default change shows up.
    let mut eq = ParametricEq::new(SR);
    let (in_l, in_r) = fixture_pseudo_noise(0.3, 4.0, 0xCAFE_BABE);
    let (out_l, out_r) = render_through(&mut eq, &in_l, &in_r);
    let report = measure(&out_l, &out_r);
    check_or_update("eq_default_pseudo_noise", report, &DEFAULT_TOL);
}

#[test]
fn snapshot_silence_through_dattorro() {
    // Documents that silence in → silence out on every metric.
    let mut rev = DattorroReverb::new(SR);
    let (in_l, in_r) = fixture_silence(2.0);
    let (out_l, out_r) = render_through(&mut rev, &in_l, &in_r);
    let report = measure(&out_l, &out_r);
    // Stricter DC tolerance for silence — true zero state must be exact.
    let tol = Tolerance {
        dc_abs: 1e-9,
        ..DEFAULT_TOL
    };
    check_or_update("silence_through_dattorro", report, &tol);
}
