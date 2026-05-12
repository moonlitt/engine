//! Session save/load — DAW-style project persistence.
//!
//! A session captures everything needed to reproduce an audio engine
//! configuration from cold start:
//!
//!   * Mixer topology — tracks, sends, master, inserts, routing
//!   * Back-end state — plug-in path + chunked state blob (`MLST` for
//!     VST3, opaque for SF2 / CLAP) + per-back-end `warm_up_blocks`
//!     hint so sample streamers (Keyscape, Omnisphere) come up audible
//!   * Transport — tempo override, loop flag (playhead is transient
//!     and intentionally not captured)
//!   * Sequencer source — path to the MIDI file currently loaded, if any
//!
//! Schema is versioned. v1 was mixer-only; v2 adds transport + sequencer
//! + per-back-end warm-up. The user explicitly chose no v1→v2 migration
//! path (`first principles, can start over`), so loading a v1 session
//! fails loudly with `SessionError::UnsupportedVersion`.

use std::error::Error;
use std::fmt;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as Base64Engine;
use moonlitt_core::AudioBackend;
use moonlitt_mixer::{InsertEffect, Mixer, SendBus, Track};
use serde::{Deserialize, Serialize};

use crate::transport::Transport;

/// Session file format version. Bumped from 1 to 2 when the schema gained
/// transport + sequencer source + per-back-end warm-up hints. Mismatched
/// versions are rejected at load time.
pub const SESSION_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SessionError {
    UnsupportedVersion { found: u32, expected: u32 },
    InvalidJson(String),
    Io(String),
    BackendCreate { path: String, cause: String },
    BackendLoadState(String),
    BackendWarmUp(String),
    Base64(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found, expected } => write!(
                f,
                "unsupported session version {found} (this build expects {expected})"
            ),
            Self::InvalidJson(m) => write!(f, "invalid session JSON: {m}"),
            Self::Io(m) => write!(f, "session I/O error: {m}"),
            Self::BackendCreate { path, cause } => {
                write!(f, "failed to create back-end for {path}: {cause}")
            }
            Self::BackendLoadState(m) => write!(f, "failed to restore back-end state: {m}"),
            Self::BackendWarmUp(m) => write!(f, "failed to warm up back-end: {m}"),
            Self::Base64(m) => write!(f, "state blob is not valid base64: {m}"),
        }
    }
}

impl Error for SessionError {}

pub type SessionResult<T> = std::result::Result<T, SessionError>;

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
pub struct Session {
    pub version: u32,
    pub sample_rate: u32,
    pub master: MasterState,
    pub tracks: Vec<TrackState>,
    pub send_buses: Vec<SendBusState>,
    /// Transport state (tempo override, loop). Default = stopped, no tempo
    /// override. Older v1 files without this field deserialize as default.
    #[serde(default)]
    pub transport: TransportSnapshot,
    /// Path to the MIDI file that should be re-loaded into the sequencer
    /// when this session restores. `None` if no clip is loaded.
    #[serde(default)]
    pub sequencer_source: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct TransportSnapshot {
    /// `Some(bpm)` overrides whatever tempo map is in the MIDI file.
    /// `None` defers to the MIDI file's embedded tempo.
    #[serde(default)]
    pub tempo_override_bpm: Option<f64>,
    #[serde(default)]
    pub looping: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MasterState {
    pub volume: f32,
    pub limiter_threshold: f32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TrackState {
    pub id: u32,
    pub channel_mask: u16,
    pub volume: f32,
    #[serde(default)]
    pub trim_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub send_levels: Vec<f32>,
    pub source: SourceState,
    pub inserts: Vec<InsertState>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InsertState {
    pub id: u32,
    pub bypass: bool,
    pub source: SourceState,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SendBusState {
    pub id: u32,
    pub level: f32,
    pub source: SourceState,
}

/// Captures how to reconstruct a back-end: file path + optional state blob
/// + recommended warm-up cycles for sample-streamers.
#[derive(Serialize, Deserialize, Debug)]
pub struct SourceState {
    /// File path used to load the back-end (.sf2, .vst3, .clap).
    pub path: Option<String>,
    /// Base64-encoded back-end state. `None` if the back-end doesn't
    /// implement state save (e.g. NullBackend).
    pub state: Option<String>,
    /// Silent process cycles to run after `load_state`. Sample streamers
    /// (Spectrasonics) need ~8192; native synths / SF2 use 0.
    /// Auto-captured from `AudioBackend::recommended_warm_up_blocks()` at
    /// save time.
    #[serde(default)]
    pub warm_up_blocks: u32,
}

/// Fully-reconstructed audio engine handed back by `Session::restore`.
/// Caller plugs `mixer` into its render path, `transport` into its
/// control surface, and optionally loads `sequencer_source` into a
/// `Sequencer` instance.
pub struct RestoredSession {
    pub mixer: Mixer,
    pub transport: Transport,
    pub sequencer_source: Option<String>,
}

// ---------------------------------------------------------------------------
// Capture: live engine → Session
// ---------------------------------------------------------------------------

impl Session {
    /// Snapshot a complete audio engine state. Single-threaded — must run
    /// from the thread that owns the mixer (typically the control thread,
    /// before the audio thread starts processing).
    pub fn from_state(
        mixer: &Mixer,
        transport: &Transport,
        sequencer_source: Option<String>,
    ) -> Self {
        let tracks = mixer
            .tracks()
            .iter()
            .map(|t| TrackState {
                id: t.id,
                channel_mask: t.channel_mask,
                volume: t.volume,
                trim_db: t.trim_db,
                pan: t.pan,
                mute: t.mute,
                solo: t.solo,
                send_levels: t.send_levels.clone(),
                source: source_from_track(t),
                inserts: t
                    .inserts
                    .iter()
                    .map(|i| InsertState {
                        id: i.id,
                        bypass: i.bypass,
                        source: source_from_insert(i),
                    })
                    .collect(),
            })
            .collect();

        let send_buses = mixer
            .send_buses()
            .iter()
            .map(|b| SendBusState {
                id: b.id,
                level: b.level,
                source: source_from_send_bus(b),
            })
            .collect();

        Session {
            version: SESSION_VERSION,
            sample_rate: mixer.sample_rate(),
            master: MasterState {
                volume: mixer.master().volume,
                limiter_threshold: mixer.master().limiter_threshold,
            },
            tracks,
            send_buses,
            transport: TransportSnapshot::from(transport),
            sequencer_source,
        }
    }

    pub fn to_json(&self) -> SessionResult<String> {
        serde_json::to_string_pretty(self).map_err(|e| SessionError::InvalidJson(e.to_string()))
    }

    pub fn from_json(json: &str) -> SessionResult<Self> {
        let session: Session =
            serde_json::from_str(json).map_err(|e| SessionError::InvalidJson(e.to_string()))?;
        if session.version != SESSION_VERSION {
            return Err(SessionError::UnsupportedVersion {
                found: session.version,
                expected: SESSION_VERSION,
            });
        }
        Ok(session)
    }

    pub fn save_to_file(&self, path: &str) -> SessionResult<()> {
        let json = self.to_json()?;
        std::fs::write(path, json).map_err(|e| SessionError::Io(e.to_string()))
    }

    pub fn load_from_file(path: &str) -> SessionResult<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| SessionError::Io(e.to_string()))?;
        Self::from_json(&json)
    }

    /// Reconstruct a live audio engine from this session. Loads every
    /// back-end, restores its state, runs the captured warm-up cycles
    /// for sample streamers, and applies all mixer/transport parameters.
    ///
    /// Errors propagate from the first failing back-end — partial restore
    /// is not exposed because a session that's half-loaded is a worse
    /// state than refusing to load at all.
    pub fn restore(&self, buffer_size: usize) -> SessionResult<RestoredSession> {
        let mut mixer = Mixer::new(self.sample_rate, buffer_size);

        // Master.
        mixer.set_master_volume(self.master.volume);
        mixer.master_mut().limiter_threshold = self.master.limiter_threshold;

        // Tracks (with inserts).
        for ts in &self.tracks {
            let (backend, source_path) =
                restore_backend(&ts.source, self.sample_rate, buffer_size as u32)?;
            mixer.add_track_with_id(ts.id, backend, source_path, ts.channel_mask);

            if let Some(track) = mixer.track_mut(ts.id) {
                track.volume = ts.volume;
                track.trim_db = ts.trim_db;
                track.pan = ts.pan;
                track.mute = ts.mute;
                track.solo = ts.solo;
                track.send_levels = ts.send_levels.clone();
            }

            for is in &ts.inserts {
                let (insert_backend, insert_source_path) =
                    restore_backend(&is.source, self.sample_rate, buffer_size as u32)?;
                mixer.add_insert_with_id(ts.id, is.id, insert_backend, insert_source_path);
                if is.bypass {
                    mixer.set_insert_bypass(ts.id, is.id, true);
                }
            }
        }

        // Send buses.
        for bs in &self.send_buses {
            let (backend, source_path) =
                restore_backend(&bs.source, self.sample_rate, buffer_size as u32)?;
            mixer.add_send_bus_with_id(bs.id, backend, source_path);
            if let Some(bus) = mixer.send_bus_mut(bs.id) {
                bus.level = bs.level;
            }
        }

        // Transport.
        let transport = Transport::new();
        self.transport.apply_to(&transport);

        Ok(RestoredSession {
            mixer,
            transport,
            sequencer_source: self.sequencer_source.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// TransportSnapshot ↔ Transport
// ---------------------------------------------------------------------------

impl From<&Transport> for TransportSnapshot {
    fn from(t: &Transport) -> Self {
        Self {
            tempo_override_bpm: t.tempo(),
            looping: t.looping(),
        }
    }
}

impl TransportSnapshot {
    fn apply_to(&self, t: &Transport) {
        match self.tempo_override_bpm {
            Some(bpm) => t.set_tempo(bpm),
            None => t.clear_tempo(),
        }
        t.set_loop(self.looping);
    }
}

// ---------------------------------------------------------------------------
// Per-source capture / restore helpers
// ---------------------------------------------------------------------------

fn source_from_track(track: &Track) -> SourceState {
    source_from_backend(&*track.backend, track.source_path.clone())
}

fn source_from_insert(insert: &InsertEffect) -> SourceState {
    source_from_backend(&*insert.backend, insert.source_path.clone())
}

fn source_from_send_bus(bus: &SendBus) -> SourceState {
    source_from_backend(&*bus.backend, bus.source_path.clone())
}

fn source_from_backend(backend: &dyn AudioBackend, path: Option<String>) -> SourceState {
    let state = backend.save_state().ok().map(|d| BASE64.encode(&d));
    SourceState {
        path,
        state,
        warm_up_blocks: backend.recommended_warm_up_blocks() as u32,
    }
}

fn restore_backend(
    source: &SourceState,
    sample_rate: u32,
    buffer_size: u32,
) -> SessionResult<(Box<dyn AudioBackend>, Option<String>)> {
    let source_path = source.path.clone();

    let mut backend: Box<dyn AudioBackend> = if let Some(ref path) = source.path {
        moonlitt_engine::create(path, sample_rate, buffer_size).map_err(|e| {
            SessionError::BackendCreate {
                path: path.clone(),
                cause: e.to_string(),
            }
        })?
    } else {
        Box::new(moonlitt_core::NullBackend::new(sample_rate))
    };

    if let Some(ref state_b64) = source.state {
        let data = BASE64
            .decode(state_b64)
            .map_err(|e| SessionError::Base64(e.to_string()))?;
        backend
            .load_state(&data)
            .map_err(|e| SessionError::BackendLoadState(e.to_string()))?;

        // Run warm-up cycles for sample streamers. Only kicks in when the
        // back-end self-identified a non-zero recommendation at save time
        // (captured in `warm_up_blocks` per back-end).
        if source.warm_up_blocks > 0 {
            backend
                .warm_up(source.warm_up_blocks as usize)
                .map_err(|e| SessionError::BackendWarmUp(e.to_string()))?;
        }
    }

    Ok((backend, source_path))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use moonlitt_core::NullBackend;

    fn null(sr: u32) -> Box<dyn AudioBackend> {
        Box::new(NullBackend::new(sr))
    }

    fn snapshot(mixer: &Mixer) -> Session {
        Session::from_state(mixer, &Transport::new(), None)
    }

    #[test]
    fn empty_mixer_roundtrips() {
        let mixer = Mixer::new(44100, 256);
        let session = snapshot(&mixer);
        let json = session.to_json().unwrap();
        let restored = Session::from_json(&json).unwrap();

        assert_eq!(restored.version, SESSION_VERSION);
        assert_eq!(restored.sample_rate, 44100);
        assert!(restored.tracks.is_empty());
        assert!(restored.send_buses.is_empty());
        assert!((restored.master.volume - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn track_params_roundtrip() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);
        mixer.track_mut(t0).unwrap().volume = 0.8;
        mixer.track_mut(t0).unwrap().pan = -0.5;
        mixer.track_mut(t1).unwrap().mute = true;

        let json = snapshot(&mixer).to_json().unwrap();
        let restored = Session::from_json(&json).unwrap();

        assert_eq!(restored.tracks.len(), 2);
        assert!((restored.tracks[0].volume - 0.8).abs() < 0.001);
        assert!((restored.tracks[0].pan - (-0.5)).abs() < 0.001);
        assert!(restored.tracks[1].mute);
    }

    #[test]
    fn inserts_roundtrip() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        let i0 = mixer.add_insert(t0, null(44100)).unwrap();
        mixer.set_insert_bypass(t0, i0, true);

        let json = snapshot(&mixer).to_json().unwrap();
        let restored = Session::from_json(&json).unwrap();

        assert_eq!(restored.tracks[0].inserts.len(), 1);
        assert!(restored.tracks[0].inserts[0].bypass);
    }

    #[test]
    fn version_field_is_v2() {
        let mixer = Mixer::new(44100, 256);
        let session = snapshot(&mixer);
        assert_eq!(session.version, 2);
    }

    #[test]
    fn master_state_roundtrip() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.set_master_volume(0.7);
        let session = snapshot(&mixer);
        assert!((session.master.volume - 0.7).abs() < 0.001);
        assert!((session.master.limiter_threshold - 0.95).abs() < 0.001);
    }

    #[test]
    fn send_levels_roundtrip() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        mixer.add_send_bus(null(44100));
        mixer.track_mut(t0).unwrap().send_levels[0] = 0.6;

        let session = snapshot(&mixer);
        assert_eq!(session.tracks[0].send_levels.len(), 1);
        assert!((session.tracks[0].send_levels[0] - 0.6).abs() < 0.001);
    }

    #[test]
    fn restore_brings_back_mixer_params() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(null(44100), 0x0001);
        mixer.add_track(null(44100), 0x0002);
        mixer.track_mut(0).unwrap().volume = 0.5;
        mixer.set_master_volume(0.9);

        let restored = snapshot(&mixer).restore(256).unwrap();
        assert_eq!(restored.mixer.tracks().len(), 2);
        assert!((restored.mixer.tracks()[0].volume - 0.5).abs() < 0.001);
        assert!((restored.mixer.master().volume - 0.9).abs() < 0.001);
    }

    #[test]
    fn transport_roundtrip_with_tempo_override() {
        let mixer = Mixer::new(44100, 256);
        let transport = Transport::new();
        transport.set_tempo(128.5);
        transport.set_loop(true);

        let session = Session::from_state(&mixer, &transport, None);
        let json = session.to_json().unwrap();
        let restored = Session::from_json(&json).unwrap().restore(256).unwrap();

        assert_eq!(restored.transport.tempo(), Some(128.5));
        assert!(restored.transport.looping());
    }

    #[test]
    fn transport_roundtrip_without_tempo_override() {
        let mixer = Mixer::new(44100, 256);
        let transport = Transport::new();
        transport.clear_tempo();

        let restored = Session::from_state(&mixer, &transport, None)
            .restore(256)
            .unwrap();
        assert_eq!(restored.transport.tempo(), None);
        assert!(!restored.transport.looping());
    }

    #[test]
    fn sequencer_source_roundtrips() {
        let mixer = Mixer::new(44100, 256);
        let transport = Transport::new();
        let session = Session::from_state(
            &mixer,
            &transport,
            Some("examples/midi-test/Prelude1.mid".to_string()),
        );
        let json = session.to_json().unwrap();
        let restored = Session::from_json(&json).unwrap().restore(256).unwrap();
        assert_eq!(
            restored.sequencer_source.as_deref(),
            Some("examples/midi-test/Prelude1.mid")
        );
    }

    #[test]
    fn rejects_unsupported_version() {
        // Hand-craft a v1-style JSON: the value of version is the discriminator.
        let v1 = r#"{
          "version": 1,
          "sample_rate": 44100,
          "master": {"volume": 1.0, "limiter_threshold": 0.95},
          "tracks": [],
          "send_buses": []
        }"#;
        match Session::from_json(v1) {
            Err(SessionError::UnsupportedVersion { found: 1, expected: 2 }) => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn corrupted_state_blob_fails_loudly() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(null(44100), 0x0001);
        let mut session = snapshot(&mixer);
        // Inject garbage state — NullBackend's load_state currently
        // returns "not supported" error from the trait default, which is
        // the correct failure path: restore should surface it.
        session.tracks[0].source.state = Some(BASE64.encode(b"<corrupt>"));

        match session.restore(256) {
            Err(SessionError::BackendLoadState(_)) => {}
            Err(e) => panic!("expected BackendLoadState err, got different error: {e}"),
            Ok(_) => panic!("expected BackendLoadState err, but restore succeeded"),
        }
    }

    #[test]
    fn warm_up_blocks_captured_from_backend_recommendation() {
        // NullBackend recommends 0 — verify it's captured.
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(null(44100), 0xFFFF);
        let session = snapshot(&mixer);
        assert_eq!(session.tracks[0].source.warm_up_blocks, 0);
    }
}
