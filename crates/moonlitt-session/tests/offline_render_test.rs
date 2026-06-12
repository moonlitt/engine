//! Offline bounce: a session + MIDI clip must render to an audible WAV
//! on a fresh engine instance, independent of any live runtime.

use moonlitt_session::offline;
use moonlitt_session::persistence::{
    MasterState, Session, SourceState, TrackState, TransportSnapshot,
};

const SF2: &str = "/Users/wangyan/Desktop/stardew valley mods/soundfonts/GeneralUser_GS.sf2";

/// Minimal format-0 SMF: C-major chord at t=0 held one beat (120 BPM).
fn write_test_midi(path: &std::path::Path) {
    let mut track: Vec<u8> = Vec::new();
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
    for &note in &[60u8, 64, 67] {
        track.extend_from_slice(&[0x00, 0x90, note, 100]);
    }
    track.extend_from_slice(&[0x83, 0x60, 0x80, 60, 0]);
    for &note in &[64u8, 67] {
        track.extend_from_slice(&[0x00, 0x80, note, 0]);
    }
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);

    let mut data: Vec<u8> = Vec::new();
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&1u16.to_be_bytes());
    data.extend_from_slice(&480u16.to_be_bytes());
    data.extend_from_slice(b"MTrk");
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(&track);
    std::fs::write(path, data).expect("write test midi");
}

fn sf2_session() -> Session {
    Session {
        version: 2,
        sample_rate: 44100,
        master: MasterState {
            volume: 1.0,
            limiter_threshold: -0.1,
        },
        tracks: vec![TrackState {
            id: 0,
            channel_mask: 0xFFFF,
            volume: 1.0,
            trim_db: 0.0,
            pan: 0.5,
            mute: false,
            solo: false,
            send_levels: vec![],
            source: SourceState {
                path: Some(SF2.into()),
                state: None,
                warm_up_blocks: 0,
            },
            inserts: vec![],
            color: None,
        }],
        send_buses: vec![],
        transport: TransportSnapshot::default(),
        sequencer_source: None,
    }
}

#[test]
fn session_renders_midi_to_audible_wav() {
    if !std::path::Path::new(SF2).exists() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let dir = std::env::temp_dir().join("moonlitt-offline-render-test");
    std::fs::create_dir_all(&dir).unwrap();
    let midi = dir.join("chord.mid");
    let wav = dir.join("bounce.wav");
    write_test_midi(&midi);

    let stats = offline::render_to_wav(
        &sf2_session(),
        midi.to_str().unwrap(),
        wav.to_str().unwrap(),
        256,
    )
    .expect("offline render");

    assert!(
        stats.peak > 1e-3,
        "bounce must be audible (peak={})",
        stats.peak
    );
    // One beat of music + the render tail.
    assert!(
        stats.duration_secs > 2.0 && stats.duration_secs < 10.0,
        "duration sane (got {}s)",
        stats.duration_secs
    );

    // The file itself round-trips and is non-silent.
    let mut reader = hound::WavReader::open(&wav).expect("open wav");
    let file_peak = reader
        .samples::<f32>()
        .map(|s| s.unwrap_or(0.0).abs())
        .fold(0.0f32, f32::max);
    assert!(file_peak > 1e-3, "file content must be audible");
}

#[test]
fn render_fails_loudly_on_missing_midi() {
    if !std::path::Path::new(SF2).exists() {
        eprintln!("SF2 not found, skipping");
        return;
    }
    let dir = std::env::temp_dir().join("moonlitt-offline-render-test");
    std::fs::create_dir_all(&dir).unwrap();
    let err = offline::render_to_wav(
        &sf2_session(),
        "/no/such/clip.mid",
        dir.join("never.wav").to_str().unwrap(),
        256,
    )
    .unwrap_err();
    assert!(
        err.contains("clip.mid") || err.to_lowercase().contains("midi"),
        "error should name the problem: {err}"
    );
}
