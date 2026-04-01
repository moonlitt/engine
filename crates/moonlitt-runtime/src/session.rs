//! Session save/load — serialize mixer state to JSON for project persistence.
//!
//! A session captures everything needed to reconstruct a mixer:
//! track routing, volumes, pans, send levels, insert chains, and
//! backend state (plugin state blobs where supported).
//!
//! Backend source files are referenced by path — they're re-loaded on restore.

use crate::mixer::{InsertEffect, Mixer, SendBus, Track};
use base64::Engine as Base64Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use moonlitt_core::AudioBackend;
use serde::{Deserialize, Serialize};

/// Session file format version.
const SESSION_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Session data model (serializable)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub sample_rate: u32,
    pub master: MasterState,
    pub tracks: Vec<TrackState>,
    pub send_buses: Vec<SendBusState>,
}

#[derive(Serialize, Deserialize)]
pub struct MasterState {
    pub volume: f32,
    pub limiter_threshold: f32,
}

#[derive(Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
pub struct InsertState {
    pub id: u32,
    pub bypass: bool,
    pub source: SourceState,
}

#[derive(Serialize, Deserialize)]
pub struct SendBusState {
    pub id: u32,
    pub level: f32,
    pub source: SourceState,
}

/// Captures how to reconstruct a backend: file path + optional state blob.
#[derive(Serialize, Deserialize)]
pub struct SourceState {
    /// File path used to load the backend (SF2, VST3, CLAP).
    pub path: Option<String>,
    /// Base64-encoded backend state (plugin presets, etc.).
    /// None if the backend doesn't support state save.
    pub state: Option<String>,
}

// ---------------------------------------------------------------------------
// Save: Mixer → Session
// ---------------------------------------------------------------------------

impl Session {
    /// Capture a snapshot of the mixer's current state.
    ///
    /// This reads mixer parameters and backend state. Must be called from
    /// the same thread that owns the mixer (typically via command channel
    /// or before the audio thread starts).
    pub fn from_mixer(mixer: &Mixer) -> Self {
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
        }
    }

    /// Serialize session to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize session from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Save session to a file.
    pub fn save_to_file(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json = self.to_json()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load session from a file.
    pub fn load_from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let session = Self::from_json(&json)?;
        Ok(session)
    }

    /// Restore a mixer from this session.
    ///
    /// Creates a new Mixer, loads backends from source paths, restores
    /// state, and applies all mixing parameters.
    pub fn restore(&self, buffer_size: usize) -> Result<Mixer, Box<dyn std::error::Error>> {
        let mut mixer = Mixer::new(self.sample_rate, buffer_size);

        // Restore master
        mixer.set_master_volume(self.master.volume);
        mixer.master_mut().limiter_threshold = self.master.limiter_threshold;

        // Restore tracks
        for ts in &self.tracks {
            let (backend, source_path) = restore_backend(&ts.source, self.sample_rate, buffer_size as u32)?;
            mixer.add_track_with_id(ts.id, backend, source_path, ts.channel_mask);

            if let Some(track) = mixer.track_mut(ts.id) {
                track.volume = ts.volume;
                track.trim_db = ts.trim_db;
                track.pan = ts.pan;
                track.mute = ts.mute;
                track.solo = ts.solo;
                track.send_levels = ts.send_levels.clone();
            }

            // Restore inserts
            for is in &ts.inserts {
                let (insert_backend, insert_source_path) =
                    restore_backend(&is.source, self.sample_rate, buffer_size as u32)?;
                mixer.add_insert_with_id(ts.id, is.id, insert_backend, insert_source_path);
                if is.bypass {
                    mixer.set_insert_bypass(ts.id, is.id, true);
                }
            }
        }

        // Restore send buses
        for bs in &self.send_buses {
            let (backend, source_path) = restore_backend(&bs.source, self.sample_rate, buffer_size as u32)?;
            mixer.add_send_bus_with_id(bs.id, backend, source_path);
            // Set bus level
            if let Some(bus) = mixer.send_bus_mut(bs.id) {
                bus.level = bs.level;
            }
        }

        Ok(mixer)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn source_from_track(track: &Track) -> SourceState {
    let state = track.backend.save_state().ok().map(|data| BASE64.encode(&data));
    SourceState {
        path: track.source_path.clone(),
        state,
    }
}

fn source_from_insert(insert: &InsertEffect) -> SourceState {
    let state = insert.backend.save_state().ok().map(|data| BASE64.encode(&data));
    SourceState {
        path: insert.source_path.clone(),
        state,
    }
}

fn source_from_send_bus(bus: &SendBus) -> SourceState {
    let state = bus.backend.save_state().ok().map(|data| BASE64.encode(&data));
    SourceState {
        path: bus.source_path.clone(),
        state,
    }
}

fn restore_backend(
    source: &SourceState,
    sample_rate: u32,
    buffer_size: u32,
) -> Result<(Box<dyn AudioBackend>, Option<String>), Box<dyn std::error::Error>> {
    let source_path = source.path.clone();

    let mut backend: Box<dyn AudioBackend> = if let Some(ref path) = source.path {
        moonlitt_engine::create(path, sample_rate, buffer_size)
            .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?
    } else {
        Box::new(moonlitt_core::NullBackend::new(sample_rate))
    };

    if let Some(ref state_b64) = source.state {
        let data = BASE64.decode(state_b64)?;
        let _ = backend.load_state(&data); // Best effort — state may not be supported
    }

    Ok((backend, source_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonlitt_core::NullBackend;

    fn null(sr: u32) -> Box<dyn AudioBackend> {
        Box::new(NullBackend::new(sr))
    }

    #[test]
    fn test_session_roundtrip_empty_mixer() {
        let mixer = Mixer::new(44100, 256);
        let session = Session::from_mixer(&mixer);
        let json = session.to_json().unwrap();
        let restored = Session::from_json(&json).unwrap();

        assert_eq!(restored.version, SESSION_VERSION);
        assert_eq!(restored.sample_rate, 44100);
        assert_eq!(restored.tracks.len(), 0);
        assert_eq!(restored.send_buses.len(), 0);
        assert!((restored.master.volume - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_session_roundtrip_with_tracks() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);

        mixer.track_mut(t0).unwrap().volume = 0.8;
        mixer.track_mut(t0).unwrap().pan = -0.5;
        mixer.track_mut(t1).unwrap().mute = true;

        let session = Session::from_mixer(&mixer);
        let json = session.to_json().unwrap();
        let restored = Session::from_json(&json).unwrap();

        assert_eq!(restored.tracks.len(), 2);
        assert!((restored.tracks[0].volume - 0.8).abs() < 0.001);
        assert!((restored.tracks[0].pan - (-0.5)).abs() < 0.001);
        assert_eq!(restored.tracks[0].channel_mask, 0x0001);
        assert!(restored.tracks[1].mute);
    }

    #[test]
    fn test_session_roundtrip_with_inserts() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        let i0 = mixer.add_insert(t0, null(44100)).unwrap();
        mixer.set_insert_bypass(t0, i0, true);

        let session = Session::from_mixer(&mixer);
        let json = session.to_json().unwrap();
        let restored = Session::from_json(&json).unwrap();

        assert_eq!(restored.tracks[0].inserts.len(), 1);
        assert!(restored.tracks[0].inserts[0].bypass);
    }

    #[test]
    fn test_session_version() {
        let mixer = Mixer::new(44100, 256);
        let session = Session::from_mixer(&mixer);
        assert_eq!(session.version, 1);
    }

    #[test]
    fn test_session_master_state() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.set_master_volume(0.7);

        let session = Session::from_mixer(&mixer);
        assert!((session.master.volume - 0.7).abs() < 0.001);
        assert!((session.master.limiter_threshold - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_session_send_levels() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        mixer.add_send_bus(null(44100));

        // Set send level
        mixer.track_mut(t0).unwrap().send_levels[0] = 0.6;

        let session = Session::from_mixer(&mixer);
        assert_eq!(session.tracks[0].send_levels.len(), 1);
        assert!((session.tracks[0].send_levels[0] - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_session_restore_mixer() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(null(44100), 0x0001);
        mixer.add_track(null(44100), 0x0002);
        mixer.track_mut(0).unwrap().volume = 0.5;
        mixer.set_master_volume(0.9);

        let session = Session::from_mixer(&mixer);
        let restored_mixer = session.restore(256).unwrap();

        assert_eq!(restored_mixer.tracks().len(), 2);
        assert!((restored_mixer.tracks()[0].volume - 0.5).abs() < 0.001);
        assert!((restored_mixer.master().volume - 0.9).abs() < 0.001);
    }
}
