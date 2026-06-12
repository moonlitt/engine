//! Control-side mirror of a live runtime's mixer.
//!
//! The audio thread owns the real `Mixer`, so a session can't be read
//! from it directly. Instead, every capi mutation keeps this shadow in
//! lock-step, and plugin patch state is pulled on demand through the
//! backends' [`StateCaptureHandle`]s (VST3's shared `Arc<Mutex<…>>`
//! plugin handles — a brief lock from the control thread while the
//! audio thread keeps rendering).
//!
//! `moonlitt_runtime_save_session` = shadow numbers + captured states
//! → `Session` v2 JSON.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as Base64Engine;
use moonlitt_audio_io::mixer::Mixer;
use moonlitt_core::{AudioBackend, StateCaptureHandle};
use moonlitt_session::persistence::{
    InsertState, MasterState, Session, SendBusState, SourceState, TrackState, TransportSnapshot,
};

pub(crate) struct ShadowSource {
    path: Option<String>,
    warm_up_blocks: u32,
    capture: Option<StateCaptureHandle>,
    /// State blob carried over from a loaded session when no live
    /// capture handle exists (keeps restore→save lossless for backends
    /// we can't re-capture).
    frozen_state: Option<String>,
}

impl ShadowSource {
    pub fn from_backend(backend: &dyn AudioBackend, path: Option<String>) -> Self {
        Self {
            path,
            warm_up_blocks: backend.recommended_warm_up_blocks() as u32,
            capture: backend.state_capture_handle(),
            frozen_state: None,
        }
    }

    fn to_state(&self) -> Result<SourceState, String> {
        let state = match &self.capture {
            Some(capture) => Some(BASE64.encode(capture()?)),
            None => self.frozen_state.clone(),
        };
        Ok(SourceState {
            path: self.path.clone(),
            state,
            warm_up_blocks: self.warm_up_blocks,
        })
    }
}

pub(crate) struct ShadowInsert {
    id: u32,
    bypass: bool,
    source: ShadowSource,
}

pub(crate) struct ShadowTrack {
    id: u32,
    channel_mask: u16,
    volume: f32,
    trim_db: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    send_levels: Vec<f32>,
    source: ShadowSource,
    inserts: Vec<ShadowInsert>,
}

pub(crate) struct ShadowBus {
    id: u32,
    level: f32,
    source: ShadowSource,
}

/// Mirror of one runtime's mixer topology and levels.
pub(crate) struct SessionShadow {
    sample_rate: u32,
    master_volume: f32,
    tracks: Vec<ShadowTrack>,
    buses: Vec<ShadowBus>,
}

impl SessionShadow {
    /// Shadow for the simple `runtime_create` path: one track, all
    /// channels, mixer defaults.
    pub fn single_track(sample_rate: u32, source: ShadowSource) -> Self {
        let mut shadow = Self::empty(sample_rate);
        shadow.add_track(0, 0xFFFF, source);
        shadow
    }

    pub fn empty(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            master_volume: 1.0,
            tracks: Vec::new(),
            buses: Vec::new(),
        }
    }

    /// Build a shadow from a pre-runtime mixer (session restore, the
    /// pre-built mixer path, multitrack_create) — the moment just
    /// before `Runtime` consumes it, while backends are still
    /// reachable for capture handles.
    pub fn from_mixer(sample_rate: u32, mixer: &Mixer) -> Self {
        let tracks = mixer
            .tracks()
            .iter()
            .map(|t| ShadowTrack {
                id: t.id,
                channel_mask: t.channel_mask,
                volume: t.volume,
                trim_db: t.trim_db,
                pan: t.pan,
                mute: t.mute,
                solo: t.solo,
                send_levels: t.send_levels.clone(),
                source: ShadowSource::from_backend(&*t.backend, t.source_path.clone()),
                inserts: t
                    .inserts
                    .iter()
                    .map(|i| ShadowInsert {
                        id: i.id,
                        bypass: i.bypass,
                        source: ShadowSource::from_backend(&*i.backend, i.source_path.clone()),
                    })
                    .collect(),
            })
            .collect();
        let buses = mixer
            .send_buses()
            .iter()
            .map(|b| ShadowBus {
                id: b.id,
                level: b.level,
                source: ShadowSource::from_backend(&*b.backend, b.source_path.clone()),
            })
            .collect();
        Self {
            sample_rate,
            master_volume: 1.0,
            tracks,
            buses,
        }
    }

    // --- Topology (mirrors the dynamic command-channel operations) ---

    pub fn add_track(&mut self, id: u32, channel_mask: u16, source: ShadowSource) {
        self.tracks.push(ShadowTrack {
            id,
            channel_mask,
            volume: 1.0,
            trim_db: 0.0,
            pan: 0.5,
            mute: false,
            solo: false,
            send_levels: vec![0.0; self.buses.len()],
            source,
            inserts: Vec::new(),
        });
    }

    pub fn remove_track(&mut self, id: u32) {
        self.tracks.retain(|t| t.id != id);
    }

    pub fn add_insert(&mut self, track_id: u32, insert_id: u32, source: ShadowSource) {
        if let Some(t) = self.track_mut(track_id) {
            t.inserts.push(ShadowInsert {
                id: insert_id,
                bypass: false,
                source,
            });
        }
    }

    pub fn remove_insert(&mut self, track_id: u32, insert_id: u32) {
        if let Some(t) = self.track_mut(track_id) {
            t.inserts.retain(|i| i.id != insert_id);
        }
    }

    pub fn add_send_bus(&mut self, id: u32, source: ShadowSource) {
        self.buses.push(ShadowBus {
            id,
            level: 1.0,
            source,
        });
        for t in &mut self.tracks {
            t.send_levels.push(0.0);
        }
    }

    // --- Level/flag mutators (mirror the SPSC mixer-control events) ---

    fn track_mut(&mut self, id: u32) -> Option<&mut ShadowTrack> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn set_track_volume(&mut self, id: u32, v: f32) {
        if let Some(t) = self.track_mut(id) {
            t.volume = v;
        }
    }
    pub fn set_track_trim(&mut self, id: u32, db: f32) {
        if let Some(t) = self.track_mut(id) {
            t.trim_db = db;
        }
    }
    pub fn set_track_pan(&mut self, id: u32, pan: f32) {
        if let Some(t) = self.track_mut(id) {
            t.pan = pan;
        }
    }
    pub fn set_track_mute(&mut self, id: u32, mute: bool) {
        if let Some(t) = self.track_mut(id) {
            t.mute = mute;
        }
    }
    pub fn set_track_solo(&mut self, id: u32, solo: bool) {
        if let Some(t) = self.track_mut(id) {
            t.solo = solo;
        }
    }
    pub fn set_track_send(&mut self, id: u32, bus_idx: usize, level: f32) {
        if let Some(t) = self.track_mut(id) {
            if let Some(slot) = t.send_levels.get_mut(bus_idx) {
                *slot = level;
            }
        }
    }
    pub fn set_master_volume(&mut self, v: f32) {
        self.master_volume = v;
    }
    pub fn set_insert_bypass(&mut self, track_id: u32, insert_id: u32, bypass: bool) {
        if let Some(t) = self.track_mut(track_id) {
            if let Some(i) = t.inserts.iter_mut().find(|i| i.id == insert_id) {
                i.bypass = bypass;
            }
        }
    }

    // --- Snapshot ---

    /// Build a `Session` from the shadow, pulling live plugin state
    /// through the capture handles (brief per-plugin lock; bounded).
    pub fn to_session(&self, metronome_enabled: bool) -> Result<Session, String> {
        let tracks = self
            .tracks
            .iter()
            .map(|t| {
                Ok(TrackState {
                    id: t.id,
                    channel_mask: t.channel_mask,
                    volume: t.volume,
                    trim_db: t.trim_db,
                    pan: t.pan,
                    mute: t.mute,
                    solo: t.solo,
                    send_levels: t.send_levels.clone(),
                    source: t.source.to_state().map_err(|e| {
                        format!("track {}: state capture failed: {e}", t.id)
                    })?,
                    inserts: t
                        .inserts
                        .iter()
                        .map(|i| {
                            Ok(InsertState {
                                id: i.id,
                                bypass: i.bypass,
                                source: i.source.to_state().map_err(|e| {
                                    format!(
                                        "track {} insert {}: state capture failed: {e}",
                                        t.id, i.id
                                    )
                                })?,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                    color: None,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let send_buses = self
            .buses
            .iter()
            .map(|b| {
                Ok(SendBusState {
                    id: b.id,
                    level: b.level,
                    source: b
                        .source
                        .to_state()
                        .map_err(|e| format!("send bus {}: state capture failed: {e}", b.id))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(Session {
            version: 2,
            sample_rate: self.sample_rate,
            master: MasterState {
                volume: self.master_volume,
                limiter_threshold: -0.1,
            },
            tracks,
            send_buses,
            transport: TransportSnapshot {
                // The C API exposes no tempo/loop setters yet, so the
                // shadow can't have diverged from these defaults.
                tempo_override_bpm: None,
                looping: false,
                metronome_enabled,
            },
            sequencer_source: None,
        })
    }
}
