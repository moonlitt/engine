//! In-process audio engine for the desktop app.
//!
//! Direct port of `packages/server/src/engine.ts` with no IPC layer.
//! One master mixer track holds the default instrument and listens to the
//! union of MIDI channels NOT overridden. Zero or more "override" tracks
//! pin a single MIDI channel to its own backend.
//!
//! Master mask = 0xFFFF & ~(union of overridden channel bits).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use moonlitt_core::AudioBackend;
use moonlitt_engine::plugin_info::{PluginFormat, PluginInfo};
use moonlitt_vst3::Vst3Plugin;
use parking_lot::Mutex;
use serde::Serialize;

/// Shared handle to a hosted VST3 plug-in. Cloned by [`Engine`] and the
/// macOS plug-in GUI module so both ends drive the same instance — no
/// state-copy, no warm-up rebuild on patch changes. See
/// `moonlitt_engine::backends::vst3::Vst3Backend` for the locking
/// discipline this is meant to support.
pub type Vst3PluginHandle = Arc<Mutex<Vst3Plugin>>;

use crate::midi_analyze::{self, MidiInfo};

// ---------------------------------------------------------------------------
// Public state shapes (serde-serialised → identical to packages/protocol)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct ParamMeta {
    pub id: u32,
    pub name: String,
    pub group: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    #[serde(rename = "stepCount")]
    pub step_count: u32,
    pub value: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct InsertState {
    pub id: u32,
    pub name: String,
    pub bypassed: bool,
    pub params: Vec<ParamMeta>,
}

/// User-visible state of a send / aux bus. The bus's backend is a single
/// effect (reverb, delay, etc.) — Pro Tools-style "aux send".
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SendBusView {
    pub id: u32,
    pub name: String,
    /// Effect-type slug — the string the engine recognised in `add_send_bus`.
    pub effect_type: String,
    pub level: f32,
    pub params: Vec<ParamMeta>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChannelOverrideState {
    pub channel: u8,
    pub instrument_path: String,
    pub instrument_name: String,
    /// Patch name parsed from this plug-in's captured state, when one
    /// has been captured AND the state blob embeds a recognisable name.
    /// `None` for plug-ins whose state we can't introspect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_name: Option<String>,
    /// dB
    pub volume: f64,
    /// Stereo pan in [-1.0, 1.0] where -1.0 is full-left and +1.0 is full-right.
    pub pan: f64,
    pub muted: bool,
    pub solo: bool,
    pub inserts: Vec<InsertState>,
    /// One send level per send bus, indexed by bus ID. Missing IDs mean
    /// "no send to that bus" (= 0). Sized lazily as buses are added.
    #[serde(default)]
    pub send_levels: Vec<f32>,
    /// Optional user-assigned colour — hex like "#4a90d9". `None` =
    /// inherit the neutral default look.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MidiState {
    pub name: String,
    pub path: String,
    pub tempo_bpm: Option<f64>,
    pub time_signature: Option<[u8; 2]>,
    pub length_bars: f64,
    pub channels: Vec<midi_analyze::MidiChannelInfo>,
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct MasterStateView {
    /// Master bus gain in dB. UI clamps to roughly [-60, +6].
    pub volume_db: f64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    pub bpm: f64,
    pub playing: bool,
    pub looping: bool,
    pub metronome_enabled: bool,
    pub master: MasterStateView,
    pub default_instrument_path: Option<String>,
    /// Patch name parsed from the default instrument's captured state.
    /// See [`ChannelOverrideState::patch_name`] for the same field on overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_patch_name: Option<String>,
    pub midi: Option<MidiState>,
    pub overrides: Vec<ChannelOverrideState>,
    /// All send / aux buses, in the order they were added.
    pub send_buses: Vec<SendBusView>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfoView {
    pub name: String,
    pub path: String,
    /// "Sf2" | "Vst3" | "Clap" | "Sfz" — matches the legacy debug format.
    pub format: String,
}

/// Live peak levels for the master bus and every override track. Emitted
/// at ~60 Hz by the meter loop. Structured (not positional) so the
/// frontend can map a meter back to its MIDI channel without relying on
/// override-insertion order matching the backend's.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MeterSnapshot {
    /// `[L, R]` peak in the linear range [0.0, 1.0+]. Above 1.0 means
    /// the master signal clipped within the measurement window.
    pub master: [f32; 2],
    pub tracks: Vec<TrackMeter>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrackMeter {
    /// MIDI channel (0..15) this override is pinned to.
    pub channel: u8,
    pub l: f32,
    pub r: f32,
}

// ---------------------------------------------------------------------------
// Effect factory
// ---------------------------------------------------------------------------

fn make_effect(kind: &str, sr: u32) -> Option<(Box<dyn AudioBackend>, &'static str)> {
    use moonlitt_effects as fx;
    Some(match kind {
        "eq" => (Box::new(fx::ParametricEq::new(sr)), "EQ"),
        "compressor" => (Box::new(fx::Compressor::new(sr)), "Compressor"),
        "reverb" => (Box::new(fx::Reverb::new(sr)), "Reverb"),
        "dattorro-reverb" => (Box::new(fx::DattorroReverb::new(sr)), "Dattorro Reverb"),
        "limiter" => (Box::new(fx::Limiter::new(sr)), "Limiter"),
        "gate" => (Box::new(fx::Gate::new(sr)), "Gate"),
        "deesser" => (Box::new(fx::DeEsser::new(sr)), "De-esser"),
        "stereo-delay" | "delay" => (Box::new(fx::StereoDelay::new(sr)), "Stereo Delay"),
        "chorus" => (Box::new(fx::Chorus::new(sr)), "Chorus"),
        "flanger" => (Box::new(fx::Flanger::new(sr)), "Flanger"),
        "phaser" => (Box::new(fx::Phaser::new(sr)), "Phaser"),
        "tremolo" => (Box::new(fx::Tremolo::new(sr)), "Tremolo"),
        "saturator" => (Box::new(fx::Saturator::new(sr)), "Saturator"),
        "bitcrusher" => (Box::new(fx::Bitcrusher::new(sr)), "Bitcrusher"),
        "multiband-compressor" => (Box::new(fx::MultibandCompressor::new(sr)), "Multiband Compressor"),
        "auto-filter" => (Box::new(fx::AutoFilter::new(sr)), "Auto Filter"),
        "pitch-shifter" => (Box::new(fx::PitchShifter::new(sr)), "Pitch Shifter"),
        "gain" => (Box::new(fx::Gain::new(sr)), "Gain"),
        "stereo-width" => (Box::new(fx::StereoWidth::new(sr)), "Stereo Width"),
        _ => return None,
    })
}

fn snapshot_params(backend: &dyn AudioBackend) -> Vec<ParamMeta> {
    let count = backend.param_count();
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Some(info) = backend.param_info(i) else { continue };
        let value = backend.get_param(info.id).unwrap_or(info.default);
        out.push(ParamMeta {
            id: info.id,
            name: info.name,
            group: info.group,
            min: info.min,
            max: info.max,
            default: info.default,
            step_count: info.step_count,
            value,
        });
    }
    out
}

fn db_to_linear(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

fn linear_to_db(lin: f64) -> f64 {
    20.0 * lin.max(1e-6).log10()
}

fn format_format(f: PluginFormat) -> &'static str {
    match f {
        PluginFormat::Sf2 => "Sf2",
        PluginFormat::Sfz => "Sfz",
        PluginFormat::Vst3 => "Vst3",
        PluginFormat::Clap => "Clap",
    }
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct Override {
    channel: u8,
    native_track_id: u32,
    instrument_path: String,
    instrument_name: String,
    volume: f64,
    /// Stereo pan in [-1.0, 1.0]; 0.0 = center.
    pan: f64,
    muted: bool,
    solo: bool,
    inserts: Vec<InsertState>,
    /// Per-bus send level. Index = bus id. Sized to current bus count.
    send_levels: Vec<f32>,
    /// User-assigned colour (hex). `None` = neutral default.
    color: Option<String>,
    /// Cached `AudioBackend::recommended_warm_up_blocks` from when this
    /// override's back-end was loaded. Stored so session capture knows
    /// the value without re-loading the plug-in.
    warm_up_blocks: u32,
    /// Shared handle to the underlying `Vst3Plugin`, when the loaded
    /// instrument is a VST3. `None` for SF2 / CLAP / SFZ. Cloned by
    /// `plugin_window` so the GUI window drives the same instance the
    /// audio thread renders against.
    plugin_handle: Option<Vst3PluginHandle>,
}

struct Inner {
    sample_rate: u32,
    buffer_size: u32,
    runtime: Option<moonlitt_audio_io::Runtime>,
    master_track_id: Option<u32>,
    default_instrument_path: Option<String>,
    /// Cached warm-up recommendation for the default instrument's
    /// back-end. See `Override::warm_up_blocks` for rationale.
    default_warm_up_blocks: u32,
    /// Shared handle to the master track's VST3 plug-in, if any. See
    /// [`Override::plugin_handle`] for the symmetric override-side field.
    default_plugin_handle: Option<Vst3PluginHandle>,
    overrides: Vec<Override>,
    send_buses: Vec<SendBusView>,
    midi: Option<MidiState>,
    bpm: f64,
    playing: bool,
    looping: bool,
    metronome_enabled: bool,
    master_volume_db: f64,
    plugin_cache: Option<Vec<PluginInfo>>,
}

pub struct Engine {
    inner: Mutex<Inner>,
    /// Set to true once `ensure_runtime` succeeds at least once. Helps
    /// callers decide between "uninitialised" and "audio failed".
    runtime_started: AtomicBool,
}

// SAFETY: cpal::Stream contains a CoreAudio property listener that holds a
// raw pointer + non-Send closure, but we only ever touch the Stream via the
// owning Mutex, on the thread that built it (the audio control thread is
// independent and managed by CoreAudio). Wrapping the whole Engine as
// Send + Sync is the same trade-off the napi binding makes implicitly by
// pinning all access to the Node event loop.
unsafe impl Send for Engine {}
unsafe impl Sync for Engine {}

// ---------------------------------------------------------------------------
// Engine: public API
// ---------------------------------------------------------------------------

impl Engine {
    pub fn new(sample_rate: u32, buffer_size: u32) -> Self {
        Self {
            inner: Mutex::new(Inner {
                sample_rate,
                buffer_size,
                runtime: None,
                master_track_id: None,
                default_instrument_path: None,
                default_warm_up_blocks: 0,
                default_plugin_handle: None,
                overrides: Vec::new(),
                send_buses: Vec::new(),
                midi: None,
                bpm: 120.0,
                playing: false,
                looping: false,
                metronome_enabled: false,
                master_volume_db: 0.0,
                plugin_cache: None,
            }),
            runtime_started: AtomicBool::new(false),
        }
    }

    pub fn is_runtime_started(&self) -> bool {
        self.runtime_started.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> ProjectState {
        let s = self.inner.lock();
        ProjectState {
            bpm: s.bpm,
            playing: s.playing,
            looping: s.looping,
            metronome_enabled: s.metronome_enabled,
            master: MasterStateView {
                volume_db: s.master_volume_db,
            },
            default_instrument_path: s.default_instrument_path.clone(),
            default_patch_name: s
                .default_instrument_path
                .as_deref()
                .and_then(patch_name_for_path),
            midi: s.midi.clone(),
            overrides: s
                .overrides
                .iter()
                .map(|o| ChannelOverrideState {
                    channel: o.channel,
                    instrument_path: o.instrument_path.clone(),
                    instrument_name: o.instrument_name.clone(),
                    patch_name: patch_name_for_path(&o.instrument_path),
                    volume: o.volume,
                    pan: o.pan,
                    muted: o.muted,
                    solo: o.solo,
                    inserts: o.inserts.clone(),
                    send_levels: o.send_levels.clone(),
                    color: o.color.clone(),
                })
                .collect(),
            send_buses: s.send_buses.clone(),
        }
    }

    pub fn meter_snapshot(&self) -> MeterSnapshot {
        let s = self.inner.lock();
        let Some(rt) = s.runtime.as_ref() else {
            return MeterSnapshot {
                master: [0.0, 0.0],
                tracks: Vec::new(),
            };
        };
        let (ml, mr) = rt.master_levels();
        let tracks = s
            .overrides
            .iter()
            .map(|o| {
                let (l, r) = rt.track_levels(o.native_track_id);
                TrackMeter {
                    channel: o.channel,
                    l,
                    r,
                }
            })
            .collect();
        MeterSnapshot {
            master: [ml, mr],
            tracks,
        }
    }

    // --- Plugin scanning ---

    pub fn scan_plugins(&self, force: bool) -> Vec<PluginInfoView> {
        let (sr, buf, cached) = {
            let s = self.inner.lock();
            let cached = if force { None } else { s.plugin_cache.clone() };
            (s.sample_rate, s.buffer_size, cached)
        };
        if let Some(c) = cached {
            return c.into_iter().map(plugin_info_to_view).collect();
        }
        // Scan outside the mutex — it's slow.
        let scanned = moonlitt_engine::scan_plugins(sr, buf);
        let mut s = self.inner.lock();
        s.plugin_cache = Some(scanned.clone());
        scanned.into_iter().map(plugin_info_to_view).collect()
    }

    // --- Default instrument ---

    pub fn set_default_instrument(&self, path: &str) -> Result<(), String> {
        self.set_default_instrument_with_state(path, None)
    }

    /// Same as `set_default_instrument`, but also seeds the new back-end
    /// with a state blob and runs the back-end's recommended warm-up
    /// before the audio thread takes ownership. Used by session restore
    /// so Keyscape-class samplers come up audible without the caller
    /// needing to know about warm-up.
    pub fn set_default_instrument_with_state(
        &self,
        path: &str,
        state: Option<&[u8]>,
    ) -> Result<(), String> {
        self.ensure_runtime()?;
        let (sr, buf) = {
            let s = self.inner.lock();
            (s.sample_rate, s.buffer_size)
        };
        let (mut backend, handle) = create_backend_with_vst3_handle(path, sr, buf)?;
        let warm_up_blocks = backend.recommended_warm_up_blocks() as u32;
        if let Some(state_bytes) = state {
            backend
                .load_state(state_bytes)
                .map_err(|e| format!("load_state: {e}"))?;
            if warm_up_blocks > 0 {
                backend
                    .warm_up(warm_up_blocks as usize)
                    .map_err(|e| format!("warm_up: {e}"))?;
            }
        }
        let mut s = self.inner.lock();
        let track_id = s
            .master_track_id
            .ok_or_else(|| "master track missing".to_string())?;
        let rt = s
            .runtime
            .as_mut()
            .ok_or_else(|| "runtime missing".to_string())?;
        rt.swap_track_backend(track_id, backend);
        s.default_instrument_path = Some(path.to_string());
        s.default_warm_up_blocks = warm_up_blocks;
        s.default_plugin_handle = handle;
        Ok(())
    }

    // --- MIDI loading ---

    pub fn load_midi(&self, path: &str, name: &str) -> Result<MidiState, String> {
        let info: MidiInfo = midi_analyze::analyze(path)?;
        self.ensure_runtime()?;
        let mut s = self.inner.lock();

        // Auto-apply tempo from the file (user can override afterwards).
        if let Some(bpm) = info.tempo_bpm.filter(|b| b.is_finite()) {
            s.bpm = bpm;
            if let Some(rt) = s.runtime.as_ref() {
                rt.set_tempo(bpm);
            }
        }

        if let Some(rt) = s.runtime.as_mut() {
            rt.load_midi(path).map_err(|e| format!("loadMidi: {e}"))?;
        }

        let midi = MidiState {
            name: name.to_string(),
            path: path.to_string(),
            tempo_bpm: info.tempo_bpm,
            time_signature: info.time_signature,
            length_bars: info.length_bars,
            channels: info.channels,
        };
        s.midi = Some(midi.clone());
        Ok(midi)
    }

    // --- Per-channel overrides ---

    pub fn set_channel_override(&self, channel: u8, path: &str) -> Result<ChannelOverrideState, String> {
        self.set_channel_override_with_state(channel, path, None)
    }

    pub fn set_channel_override_with_state(
        &self,
        channel: u8,
        path: &str,
        state: Option<&[u8]>,
    ) -> Result<ChannelOverrideState, String> {
        if channel > 15 {
            return Err(format!("invalid channel {channel}"));
        }
        self.ensure_runtime()?;
        let (sr, buf) = {
            let s = self.inner.lock();
            (s.sample_rate, s.buffer_size)
        };
        let (mut backend, handle) = create_backend_with_vst3_handle(path, sr, buf)?;
        let warm_up_blocks = backend.recommended_warm_up_blocks() as u32;
        if let Some(state_bytes) = state {
            backend
                .load_state(state_bytes)
                .map_err(|e| format!("load_state: {e}"))?;
            if warm_up_blocks > 0 {
                backend
                    .warm_up(warm_up_blocks as usize)
                    .map_err(|e| format!("warm_up: {e}"))?;
            }
        }
        let instrument_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        let mut s = self.inner.lock();
        // existing override → swap backend.
        if let Some(existing) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            let id = existing.native_track_id;
            if let Some(rt) = s.runtime.as_mut() {
                rt.swap_track_backend(id, backend);
            }
            // Reborrow to apply path/name updates.
            let existing = s.overrides.iter_mut().find(|o| o.channel == channel).unwrap();
            existing.instrument_path = path.to_string();
            existing.instrument_name = instrument_name;
            existing.warm_up_blocks = warm_up_blocks;
            existing.plugin_handle = handle;
            return Ok(state_of(existing));
        }

        let mask: u16 = 1 << channel;
        let native_track_id = s
            .runtime
            .as_mut()
            .ok_or_else(|| "runtime missing".to_string())?
            .add_track(backend, mask);
        let ov = Override {
            channel,
            native_track_id,
            instrument_path: path.to_string(),
            instrument_name,
            volume: 0.0,
            pan: 0.0,
            muted: false,
            solo: false,
            inserts: Vec::new(),
            send_levels: vec![0.0; s.send_buses.len()],
            color: None,
            warm_up_blocks,
            plugin_handle: handle,
        };
        let st = state_of(&ov);
        s.overrides.push(ov);
        sync_master_mask(&mut s);
        Ok(st)
    }

    pub fn remove_channel_override(&self, channel: u8) -> Result<(), String> {
        let mut s = self.inner.lock();
        let idx = s
            .overrides
            .iter()
            .position(|o| o.channel == channel)
            .ok_or_else(|| format!("no override on channel {channel}"))?;
        let id = s.overrides[idx].native_track_id;
        if let Some(rt) = s.runtime.as_mut() {
            rt.remove_track(id);
        }
        s.overrides.remove(idx);
        sync_master_mask(&mut s);
        Ok(())
    }

    pub fn set_channel_volume(&self, channel: u8, db: f64) -> Result<(), String> {
        let mut s = self.inner.lock();
        let id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let id = id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.mixer_set_track_volume(id as u8, db_to_linear(db));
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            o.volume = db;
        }
        Ok(())
    }

    pub fn set_channel_mute(&self, channel: u8, muted: bool) -> Result<(), String> {
        let mut s = self.inner.lock();
        let id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let id = id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.mixer_set_track_mute(id as u8, muted);
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            o.muted = muted;
        }
        Ok(())
    }

    /// Set stereo pan for an override track. `pan` is clamped to [-1, 1].
    /// Returns `Err` if no override exists on that channel — the master
    /// track doesn't expose a separate pan since it fans out across many
    /// MIDI channels with potentially conflicting intents.
    pub fn set_channel_pan(&self, channel: u8, pan: f64) -> Result<(), String> {
        let pan = pan.clamp(-1.0, 1.0);
        let mut s = self.inner.lock();
        let id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let id = id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.mixer_set_track_pan(id as u8, pan as f32);
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            o.pan = pan;
        }
        Ok(())
    }

    /// Set a user-assigned color on an override, or pass `None` to clear.
    /// Validates hex format loosely — must start with `#` and have 3, 6,
    /// or 8 hex digits. Bad input is rejected so we don't smuggle a
    /// CSS-injection string into rendered UI.
    pub fn set_channel_color(
        &self,
        channel: u8,
        color: Option<&str>,
    ) -> Result<(), String> {
        if let Some(c) = color {
            let body = c.strip_prefix('#').ok_or_else(|| {
                format!("color must start with '#' (got {c:?})")
            })?;
            let len_ok = matches!(body.len(), 3 | 6 | 8);
            let hex_ok = body.chars().all(|ch| ch.is_ascii_hexdigit());
            if !(len_ok && hex_ok) {
                return Err(format!("invalid hex color {c:?}"));
            }
        }
        let mut s = self.inner.lock();
        let o = s
            .overrides
            .iter_mut()
            .find(|o| o.channel == channel)
            .ok_or_else(|| format!("no override on channel {channel}"))?;
        o.color = color.map(|s| s.to_string());
        Ok(())
    }

    pub fn set_channel_solo(&self, channel: u8, solo: bool) -> Result<(), String> {
        let mut s = self.inner.lock();
        let id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let id = id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.mixer_set_track_solo(id as u8, solo);
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            o.solo = solo;
        }
        Ok(())
    }

    pub fn set_channel_program(&self, channel: u8, program: u8) -> Result<(), String> {
        if channel > 15 || program > 127 {
            return Err("channel/program out of range".to_string());
        }
        let mut s = self.inner.lock();
        if let Some(rt) = s.runtime.as_mut() {
            rt.program_change(channel, program);
        }
        Ok(())
    }

    // --- Inserts ---

    pub fn add_insert(&self, channel: u8, effect_type: &str) -> Result<InsertState, String> {
        let sr = self.inner.lock().sample_rate;
        let (backend, friendly) = make_effect(effect_type, sr)
            .ok_or_else(|| format!("unknown effect type: {effect_type}"))?;
        let params = snapshot_params(backend.as_ref());

        let mut s = self.inner.lock();
        let track_id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let track_id = track_id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        let insert_id = s
            .runtime
            .as_mut()
            .ok_or_else(|| "runtime missing".to_string())?
            .add_insert(track_id, backend);

        let meta = InsertState {
            id: insert_id,
            name: friendly.to_string(),
            bypassed: false,
            params,
        };
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            o.inserts.push(meta.clone());
        }
        Ok(meta)
    }

    pub fn remove_insert(&self, channel: u8, insert_id: u32) -> Result<(), String> {
        let mut s = self.inner.lock();
        let track_id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let track_id = track_id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.remove_insert(track_id, insert_id);
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            o.inserts.retain(|i| i.id != insert_id);
        }
        Ok(())
    }

    // --- Send / aux buses ---

    pub fn add_send_bus(&self, effect_type: &str) -> Result<SendBusView, String> {
        let sr = self.inner.lock().sample_rate;
        let (backend, friendly) = make_effect(effect_type, sr)
            .ok_or_else(|| format!("unknown effect type: {effect_type}"))?;
        let params = snapshot_params(backend.as_ref());

        let mut s = self.inner.lock();
        let bus_id = s
            .runtime
            .as_mut()
            .ok_or_else(|| "runtime missing".to_string())?
            .add_send_bus(backend);

        let view = SendBusView {
            id: bus_id,
            name: friendly.to_string(),
            effect_type: effect_type.to_string(),
            level: 1.0,
            params,
        };
        s.send_buses.push(view.clone());
        // Mirror the mixer's per-track send_levels extension so the
        // engine-side cache stays in lock-step.
        for o in &mut s.overrides {
            o.send_levels.push(0.0);
        }
        Ok(view)
    }

    pub fn set_channel_send_level(
        &self,
        channel: u8,
        bus_id: u32,
        level: f32,
    ) -> Result<(), String> {
        // Match the mixer's convention: 0.0 = silent send, 1.0 = unity,
        // allow a little headroom above for boost — clamp at 4× (12 dB).
        let level = level.clamp(0.0, 4.0);
        let mut s = self.inner.lock();
        if !s.send_buses.iter().any(|b| b.id == bus_id) {
            return Err(format!("no send bus with id {bus_id}"));
        }
        let id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let track_id = id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.mixer_set_track_send(track_id as u8, bus_id as u8, level);
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            while o.send_levels.len() <= bus_id as usize {
                o.send_levels.push(0.0);
            }
            o.send_levels[bus_id as usize] = level;
        }
        Ok(())
    }

    pub fn set_insert_bypass(
        &self,
        channel: u8,
        insert_id: u32,
        bypass: bool,
    ) -> Result<(), String> {
        let mut s = self.inner.lock();
        let track_id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let track_id = track_id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.mixer_set_insert_bypass(track_id as u8, insert_id as u8, bypass);
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            if let Some(ins) = o.inserts.iter_mut().find(|i| i.id == insert_id) {
                ins.bypassed = bypass;
            }
        }
        Ok(())
    }

    pub fn set_insert_param(
        &self,
        channel: u8,
        insert_id: u32,
        param_id: u32,
        value: f64,
    ) -> Result<(), String> {
        let mut s = self.inner.lock();
        let track_id_opt = s
            .overrides
            .iter()
            .find(|o| o.channel == channel)
            .map(|o| o.native_track_id);
        let track_id = track_id_opt.ok_or_else(|| format!("no override on channel {channel}"))?;
        if let Some(rt) = s.runtime.as_mut() {
            rt.set_insert_param(track_id as u8, insert_id as u8, param_id as u16, value);
        }
        if let Some(o) = s.overrides.iter_mut().find(|o| o.channel == channel) {
            if let Some(ins) = o.inserts.iter_mut().find(|i| i.id == insert_id) {
                if let Some(p) = ins.params.iter_mut().find(|p| p.id == param_id) {
                    p.value = value;
                }
            }
        }
        Ok(())
    }

    // --- Transport / master ---

    pub fn play(&self) -> Result<(), String> {
        let s = self.inner.lock();
        let rt = s.runtime.as_ref().ok_or_else(|| {
            "no session yet — load a MIDI or pick a default instrument first".to_string()
        })?;
        rt.play();
        drop(s);
        self.inner.lock().playing = true;
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(rt) = self.inner.lock().runtime.as_ref() {
            rt.stop_playback();
        }
        self.inner.lock().playing = false;
    }

    /// Pause playback while preserving the current sequencer position —
    /// distinct from [`Self::stop`] which rewinds. Used by the play/pause
    /// toggle in the transport bay.
    pub fn pause(&self) {
        if let Some(rt) = self.inner.lock().runtime.as_ref() {
            rt.pause_playback();
        }
        self.inner.lock().playing = false;
    }

    pub fn set_loop(&self, enabled: bool) {
        let mut s = self.inner.lock();
        s.looping = enabled;
        if let Some(rt) = s.runtime.as_ref() {
            rt.set_loop(enabled);
        }
    }

    pub fn set_metronome_enabled(&self, enabled: bool) {
        let mut s = self.inner.lock();
        s.metronome_enabled = enabled;
        if let Some(rt) = s.runtime.as_ref() {
            rt.set_metronome_enabled(enabled);
        }
    }

    pub fn set_bpm(&self, bpm: f64) {
        let mut s = self.inner.lock();
        s.bpm = bpm;
        if let Some(rt) = s.runtime.as_ref() {
            rt.set_tempo(bpm);
        }
    }

    pub fn set_master_volume(&self, db: f64) {
        let mut s = self.inner.lock();
        s.master_volume_db = db;
        if let Some(rt) = s.runtime.as_mut() {
            rt.mixer_set_master_volume(db_to_linear(db));
        }
    }

    /// Resolve `target` → an instrument path so the plugin-window module can
    /// open a dedicated GUI plugin instance without holding the engine mutex
    /// while it talks to AppKit.
    pub fn instrument_path_for(&self, target: ViewTarget) -> Result<String, String> {
        let s = self.inner.lock();
        match target {
            ViewTarget::Default => s
                .default_instrument_path
                .clone()
                .ok_or_else(|| "no default instrument loaded".to_string()),
            ViewTarget::Channel(ch) => s
                .overrides
                .iter()
                .find(|o| o.channel == ch)
                .map(|o| o.instrument_path.clone())
                .ok_or_else(|| format!("no override on channel {ch}")),
        }
    }

    /// Resolve `target` → shared `Vst3Plugin` handle, when the slot holds
    /// a VST3 instrument. `None` for empty slots or for SF2/CLAP/SFZ
    /// back-ends. The plug-in GUI window uses this to attach its view to
    /// the same instance the audio thread is rendering — so picking a
    /// patch in the GUI is immediately audible.
    pub fn vst3_plugin_handle(&self, target: ViewTarget) -> Option<Vst3PluginHandle> {
        let s = self.inner.lock();
        match target {
            ViewTarget::Default => s.default_plugin_handle.clone(),
            ViewTarget::Channel(ch) => s
                .overrides
                .iter()
                .find(|o| o.channel == ch)
                .and_then(|o| o.plugin_handle.clone()),
        }
    }

    // --- Session capture / restore ---

    /// Snapshot the current engine state into a `moonlitt_session::Session`
    /// JSON-ready struct. `plugin_states` provides the state blobs keyed by
    /// instrument path (the desktop layer harvests these from the GUI plug-in
    /// registry — see `plugin_window::snapshot_all_open_states`).
    pub fn capture_session(
        &self,
        plugin_states: &std::collections::HashMap<String, Vec<u8>>,
    ) -> moonlitt_session::persistence::Session {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;
        use moonlitt_session::persistence::{
            MasterState, Session, SourceState, TrackState, TransportSnapshot,
        };

        let s = self.inner.lock();

        let mut master_mask: u16 = 0xFFFF;
        for o in &s.overrides {
            master_mask &= !(1u16 << o.channel);
        }

        let encode_state =
            |path: &Option<String>| -> Option<String> {
                path.as_ref()
                    .and_then(|p| plugin_states.get(p))
                    .map(|b| BASE64.encode(b))
            };

        let mut tracks = vec![TrackState {
            id: 0,
            channel_mask: master_mask,
            volume: 1.0,
            trim_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            send_levels: vec![],
            source: SourceState {
                path: s.default_instrument_path.clone(),
                state: encode_state(&s.default_instrument_path),
                warm_up_blocks: s.default_warm_up_blocks,
            },
            inserts: vec![],
            color: None,
        }];

        for o in &s.overrides {
            let path = Some(o.instrument_path.clone());
            tracks.push(TrackState {
                id: o.native_track_id,
                channel_mask: 1u16 << o.channel,
                volume: db_to_linear(o.volume),
                trim_db: 0.0,
                pan: o.pan as f32,
                mute: o.muted,
                solo: o.solo,
                send_levels: o.send_levels.clone(),
                source: SourceState {
                    path: path.clone(),
                    state: encode_state(&path),
                    warm_up_blocks: o.warm_up_blocks,
                },
                inserts: vec![],
                color: o.color.clone(),
            });
        }

        Session {
            version: 2,
            sample_rate: s.sample_rate,
            master: MasterState {
                volume: db_to_linear(s.master_volume_db),
                limiter_threshold: 0.95,
            },
            tracks,
            send_buses: vec![],
            transport: TransportSnapshot {
                tempo_override_bpm: Some(s.bpm),
                looping: s.looping,
                metronome_enabled: s.metronome_enabled,
            },
            sequencer_source: s.midi.as_ref().map(|m| m.path.clone()),
        }
    }

    /// Apply a `Session` to this engine — wipes overrides, sets default
    /// instrument, restores tempo and (optionally) reloads the MIDI file.
    /// Plug-in state blobs and warm-up are applied inline before the audio
    /// thread takes the back-end, so sample streamers come up audible.
    ///
    /// Returns the list of state blobs that were applied so the desktop
    /// layer can refresh its plug-in-state stash (otherwise a subsequent
    /// ⌘S would lose the patches we just restored).
    pub fn restore_session(
        &self,
        session: &moonlitt_session::persistence::Session,
    ) -> Result<std::collections::HashMap<String, Vec<u8>>, String> {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;

        self.ensure_runtime()?;

        // Clear existing overrides on the audio thread.
        {
            let mut s = self.inner.lock();
            let override_ids: Vec<u32> =
                s.overrides.iter().map(|o| o.native_track_id).collect();
            if let Some(rt) = s.runtime.as_mut() {
                for id in &override_ids {
                    rt.remove_track(*id);
                }
            }
            s.overrides.clear();
        }

        let mut restored_states = std::collections::HashMap::new();

        // The session always has a master track at index 0 — apply it as
        // the default instrument. Skip silently if the slot is empty.
        if let Some(master) = session.tracks.first() {
            if let Some(path) = master.source.path.as_deref() {
                let state_bytes = master
                    .source
                    .state
                    .as_deref()
                    .map(|b64| BASE64.decode(b64))
                    .transpose()
                    .map_err(|e| format!("decode default state: {e}"))?;
                self.set_default_instrument_with_state(path, state_bytes.as_deref())?;
                if let Some(b) = state_bytes {
                    restored_states.insert(path.to_string(), b);
                }
            }
        }

        // Restore overrides — every track after index 0 maps to one MIDI
        // channel via its channel_mask. Sessions captured by this engine
        // always set a single bit; anything else gets ignored.
        for track in session.tracks.iter().skip(1) {
            let Some(path) = track.source.path.as_deref() else {
                continue;
            };
            let mask = track.channel_mask;
            if mask == 0 || mask.count_ones() != 1 {
                continue;
            }
            let channel = mask.trailing_zeros() as u8;
            let state_bytes = track
                .source
                .state
                .as_deref()
                .map(|b64| BASE64.decode(b64))
                .transpose()
                .map_err(|e| format!("decode override state ch{channel}: {e}"))?;
            self.set_channel_override_with_state(channel, path, state_bytes.as_deref())?;
            // Apply the saved mixer state on top of the freshly-created
            // override. The session stores volume as a linear gain factor;
            // the engine talks dB, so convert.
            let _ = self.set_channel_volume(channel, linear_to_db(track.volume as f64));
            let _ = self.set_channel_pan(channel, track.pan as f64);
            let _ = self.set_channel_mute(channel, track.mute);
            let _ = self.set_channel_solo(channel, track.solo);
            if let Some(color) = track.color.as_deref() {
                let _ = self.set_channel_color(channel, Some(color));
            }
            if let Some(b) = state_bytes {
                restored_states.insert(path.to_string(), b);
            }
        }

        // Master bus volume — session stores the linear gain factor;
        // the engine caches dB so the UI doesn't have to convert.
        {
            let mut s = self.inner.lock();
            s.master_volume_db = linear_to_db(session.master.volume as f64);
            if let Some(rt) = s.runtime.as_mut() {
                rt.mixer_set_master_volume(session.master.volume);
            }
        }

        // Transport loop state + metronome.
        self.set_loop(session.transport.looping);
        self.set_metronome_enabled(session.transport.metronome_enabled);

        // Transport tempo.
        if let Some(bpm) = session.transport.tempo_override_bpm {
            let mut s = self.inner.lock();
            s.bpm = bpm;
            if let Some(rt) = s.runtime.as_ref() {
                rt.set_tempo(bpm);
            }
        }

        // Sequencer source — best-effort: if the MIDI file is missing
        // we still consider the session "loaded" but emit no MIDI state.
        if let Some(midi_path) = session.sequencer_source.as_deref() {
            let name = std::path::Path::new(midi_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(midi_path)
                .to_string();
            let _ = self.load_midi(midi_path, &name);
        }

        Ok(restored_states)
    }

    // --- Internals ---

    fn ensure_runtime(&self) -> Result<(), String> {
        let mut s = self.inner.lock();
        if s.runtime.is_some() {
            return Ok(());
        }
        let sr = s.sample_rate;
        let buf = s.buffer_size;
        // Bootstrap with the chosen default instrument if set, else a silent
        // gain placeholder. The first instrument load swaps it in.
        let initial: Box<dyn AudioBackend> = match s.default_instrument_path.as_deref() {
            Some(p) => moonlitt_engine::create(p, sr, buf).map_err(|e| format!("{e}"))?,
            None => Box::new(moonlitt_effects::Gain::new(sr)),
        };
        let rt = moonlitt_audio_io::Runtime::new(initial, sr, buf)
            .map_err(|(e, _)| format!("Runtime::new: {e}"))?;
        rt.start()
            .map_err(|e| format!("audio device unavailable: {e}"))?;
        rt.set_tempo(s.bpm);
        s.runtime = Some(rt);
        s.master_track_id = Some(0);
        self.runtime_started.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ViewTarget {
    Default,
    Channel(u8),
}

fn state_of(o: &Override) -> ChannelOverrideState {
    ChannelOverrideState {
        channel: o.channel,
        instrument_path: o.instrument_path.clone(),
        instrument_name: o.instrument_name.clone(),
        patch_name: patch_name_for_path(&o.instrument_path),
        volume: o.volume,
        pan: o.pan,
        muted: o.muted,
        solo: o.solo,
        inserts: o.inserts.clone(),
        send_levels: o.send_levels.clone(),
        color: o.color.clone(),
    }
}

fn sync_master_mask(s: &mut Inner) {
    let Some(track_id) = s.master_track_id else { return };
    let Some(rt) = s.runtime.as_mut() else { return };
    let mut mask: u16 = 0xFFFF;
    for o in &s.overrides {
        mask &= !(1u16 << o.channel);
    }
    rt.set_track_channel_mask(track_id, mask);
}

fn plugin_info_to_view(p: PluginInfo) -> PluginInfoView {
    PluginInfoView {
        name: p.name,
        path: p.path,
        format: format_format(p.format).to_string(),
    }
}

/// Build a backend for `path`, and — only for `.vst3` paths — also
/// return a clone of the shared `Vst3Plugin` handle so the GUI window
/// can later drive the same instance. SF2 / CLAP / SFZ paths return
/// `(backend, None)`.
///
/// Equivalent in behaviour to `moonlitt_engine::create`; the divergence
/// is that we go directly through `Vst3Backend::new` so the typed
/// `plugin_handle()` is reachable before the backend is type-erased
/// into `Box<dyn AudioBackend>`.
fn create_backend_with_vst3_handle(
    path: &str,
    sr: u32,
    buf: u32,
) -> Result<(Box<dyn AudioBackend>, Option<Vst3PluginHandle>), String> {
    use moonlitt_engine::backends::vst3::Vst3Backend;
    if path.to_ascii_lowercase().ends_with(".vst3") {
        let mut b = Vst3Backend::new(sr, buf).map_err(|e| format!("create vst3: {e}"))?;
        b.load(path).map_err(|e| format!("load vst3: {e}"))?;
        let handle = b.plugin_handle();
        Ok((Box::new(b), handle))
    } else {
        let b = moonlitt_engine::create(path, sr, buf).map_err(|e| format!("create: {e}"))?;
        Ok((b, None))
    }
}

/// Cross-platform helper that returns the parsed patch name for a
/// plug-in path if one has been captured (macOS only; other platforms
/// don't run the GUI window registry).
fn patch_name_for_path(path: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        crate::plugin_window::patch_name_for(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

// ---------------------------------------------------------------------------
// Unit tests — pure state checks. Methods that require live audio device
// initialization (set_channel_override) are not exercised here; their
// audio side is covered by the runtime + mixer crates.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_override(channel: u8) -> Override {
        Override {
            channel,
            native_track_id: u32::from(channel) + 1,
            instrument_path: "fixture".into(),
            instrument_name: "fixture".into(),
            volume: 0.0,
            pan: 0.0,
            muted: false,
            solo: false,
            inserts: Vec::new(),
            send_levels: Vec::new(),
            color: None,
            warm_up_blocks: 0,
            plugin_handle: None,
        }
    }

    fn fake_send_bus(id: u32) -> SendBusView {
        SendBusView {
            id,
            name: "Reverb".into(),
            effect_type: "reverb".into(),
            level: 1.0,
            params: Vec::new(),
        }
    }

    #[test]
    fn set_channel_pan_updates_cached_state() {
        let eng = Engine::new(44100, 256);
        eng.inner.lock().overrides.push(fake_override(3));

        eng.set_channel_pan(3, -0.7).expect("set_channel_pan");

        let s = eng.inner.lock();
        let o = s.overrides.iter().find(|o| o.channel == 3).unwrap();
        assert!((o.pan + 0.7).abs() < 1e-9, "pan={}", o.pan);
    }

    #[test]
    fn set_channel_pan_clamps_to_unit_range() {
        let eng = Engine::new(44100, 256);
        eng.inner.lock().overrides.push(fake_override(0));

        eng.set_channel_pan(0, 2.5).unwrap();
        assert_eq!(eng.inner.lock().overrides[0].pan, 1.0);

        eng.set_channel_pan(0, -42.0).unwrap();
        assert_eq!(eng.inner.lock().overrides[0].pan, -1.0);
    }

    #[test]
    fn set_channel_pan_rejects_unknown_channel() {
        let eng = Engine::new(44100, 256);
        let err = eng.set_channel_pan(7, 0.3).unwrap_err();
        assert!(err.contains("no override on channel 7"), "got: {err}");
    }

    #[test]
    fn linear_to_db_roundtrips_with_db_to_linear() {
        for db in [-60.0, -24.0, -6.0, 0.0, 3.0, 6.0] {
            let lin = db_to_linear(db);
            let back = linear_to_db(lin as f64);
            assert!((back - db).abs() < 0.01, "db={db}, back={back}");
        }
    }

    #[test]
    fn set_insert_bypass_updates_cached_state() {
        let eng = Engine::new(44100, 256);
        {
            let mut s = eng.inner.lock();
            let mut o = fake_override(2);
            o.inserts.push(InsertState {
                id: 42,
                name: "Reverb".into(),
                bypassed: false,
                params: Vec::new(),
            });
            s.overrides.push(o);
        }

        eng.set_insert_bypass(2, 42, true).expect("set_insert_bypass");
        assert_eq!(eng.inner.lock().overrides[0].inserts[0].bypassed, true);

        eng.set_insert_bypass(2, 42, false).unwrap();
        assert_eq!(eng.inner.lock().overrides[0].inserts[0].bypassed, false);
    }

    #[test]
    fn set_channel_color_accepts_hex_and_clears_on_none() {
        let eng = Engine::new(44100, 256);
        eng.inner.lock().overrides.push(fake_override(0));

        eng.set_channel_color(0, Some("#4a90d9")).unwrap();
        assert_eq!(eng.inner.lock().overrides[0].color.as_deref(), Some("#4a90d9"));

        eng.set_channel_color(0, None).unwrap();
        assert!(eng.inner.lock().overrides[0].color.is_none());
    }

    #[test]
    fn set_channel_color_rejects_garbage() {
        let eng = Engine::new(44100, 256);
        eng.inner.lock().overrides.push(fake_override(0));

        assert!(eng.set_channel_color(0, Some("4a90d9")).is_err()); // missing #
        assert!(eng.set_channel_color(0, Some("#xyz")).is_err());    // bad hex
        assert!(eng.set_channel_color(0, Some("#12345")).is_err()); // bad length
        assert!(eng.set_channel_color(0, Some("javascript:alert(1)")).is_err());
    }

    #[test]
    fn set_channel_send_level_updates_cached_state() {
        let eng = Engine::new(44100, 256);
        eng.inner.lock().overrides.push(fake_override(2));
        eng.inner.lock().send_buses.push(fake_send_bus(0));

        eng.set_channel_send_level(2, 0, 0.5)
            .expect("set_channel_send_level");
        assert_eq!(eng.inner.lock().overrides[0].send_levels, vec![0.5]);
    }

    #[test]
    fn set_channel_send_level_rejects_unknown_bus() {
        let eng = Engine::new(44100, 256);
        eng.inner.lock().overrides.push(fake_override(0));
        let err = eng.set_channel_send_level(0, 99, 0.5).unwrap_err();
        assert!(err.contains("send bus"), "got: {err}");
    }

    #[test]
    fn set_channel_send_level_clamps_at_4x() {
        let eng = Engine::new(44100, 256);
        eng.inner.lock().overrides.push(fake_override(0));
        eng.inner.lock().send_buses.push(fake_send_bus(0));
        eng.set_channel_send_level(0, 0, 999.0).unwrap();
        assert_eq!(eng.inner.lock().overrides[0].send_levels[0], 4.0);
    }

    #[test]
    fn set_metronome_enabled_caches_state() {
        let eng = Engine::new(44100, 256);
        assert_eq!(eng.inner.lock().metronome_enabled, false);
        eng.set_metronome_enabled(true);
        assert_eq!(eng.inner.lock().metronome_enabled, true);
    }

    #[test]
    fn set_loop_caches_state() {
        let eng = Engine::new(44100, 256);
        assert_eq!(eng.inner.lock().looping, false);
        eng.set_loop(true);
        assert_eq!(eng.inner.lock().looping, true);
        eng.set_loop(false);
        assert_eq!(eng.inner.lock().looping, false);
    }

    #[test]
    fn set_insert_bypass_rejects_unknown_channel() {
        let eng = Engine::new(44100, 256);
        let err = eng.set_insert_bypass(9, 1, true).unwrap_err();
        assert!(err.contains("no override on channel 9"), "got: {err}");
    }

    #[test]
    fn set_master_volume_caches_db() {
        let eng = Engine::new(44100, 256);
        eng.set_master_volume(-3.5);
        assert!((eng.inner.lock().master_volume_db + 3.5).abs() < 1e-9);
    }
}

