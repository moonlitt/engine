//! Regression test: offline render must warm up sample-streamer plugins
//! (Keyscape, Omnisphere) after loading saved state, or the rendered WAV
//! is silence — the streamer is still loading samples while the whole
//! file renders.
//!
//! Gated: skips when Keyscape or the captured state fixture is absent.
//! The test name contains "keyscape" so CI's `--skip keyscape` filters it.

use std::path::{Path, PathBuf};
use std::process::Command;

const KEYSCAPE_VST3: &str = "/Library/Audio/Plug-Ins/VST3/Keyscape.vst3";

fn state_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../moonlitt-vst3/tests/fixtures/keyscape-default.mlstate")
}

/// Minimal format-0 SMF: C-major chord at t=0, held one beat (0.5 s at
/// 120 BPM), so the render has real audio to produce.
fn write_test_midi(path: &Path) {
    let mut track: Vec<u8> = Vec::new();
    // Tempo: 500_000 µs per beat = 120 BPM
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
    for &note in &[60u8, 64, 67] {
        track.extend_from_slice(&[0x00, 0x90, note, 100]);
    }
    // Note-offs one beat later (delta 480 = VLQ 0x83 0x60)
    track.extend_from_slice(&[0x83, 0x60, 0x80, 60, 0]);
    for &note in &[64u8, 67] {
        track.extend_from_slice(&[0x00, 0x80, note, 0]);
    }
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]); // end of track

    let mut data: Vec<u8> = Vec::new();
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes()); // format 0
    data.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    data.extend_from_slice(&480u16.to_be_bytes()); // ticks per beat
    data.extend_from_slice(b"MTrk");
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(&track);
    std::fs::write(path, data).expect("write test midi");
}

fn wav_peak(path: &Path) -> f32 {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    let spec = reader.spec();
    match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.unwrap_or(0.0).abs())
            .fold(0.0f32, f32::max),
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap_or(0) as f32 / max)
                .map(f32::abs)
                .fold(0.0f32, f32::max)
        }
    }
}

#[test]
fn keyscape_offline_render_with_state_is_audible() {
    if !Path::new(KEYSCAPE_VST3).exists() {
        eprintln!("Keyscape not installed — skipping");
        return;
    }
    let state = state_fixture();
    if !state.exists() {
        eprintln!("Keyscape state fixture missing — skipping");
        return;
    }

    let dir = std::env::temp_dir().join("moonlitt-keyscape-warmup-test");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let midi = dir.join("chord.mid");
    let wav = dir.join("render.wav");
    write_test_midi(&midi);

    let out = Command::new(env!("CARGO_BIN_EXE_moonlitt"))
        .args([
            "midi",
            midi.to_str().unwrap(),
            "--sound",
            KEYSCAPE_VST3,
            "--state",
            state.to_str().unwrap(),
            "--output",
            wav.to_str().unwrap(),
        ])
        .output()
        .expect("run moonlitt binary");
    assert!(
        out.status.success(),
        "render exited with {}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let peak = wav_peak(&wav);
    assert!(
        peak > 0.01,
        "rendered WAV is silent (peak={peak}); sample-streamer warm-up missing after load_state"
    );
}
