//! In-process audio engine for the desktop app.
//!
//! Direct port of `packages/server/src/engine.ts` with no IPC layer.
//! One master mixer track holds the default instrument and listens to the
//! union of MIDI channels NOT overridden. Zero or more "override" tracks
//! pin a single MIDI channel to its own backend.
//!
//! Master mask = 0xFFFF & ~(union of overridden channel bits).

use std::sync::atomic::{AtomicBool, Ordering};

use moonlitt_core::AudioBackend;
use moonlitt_engine::plugin_info::{PluginFormat, PluginInfo};
use parking_lot::Mutex;
use serde::Serialize;

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

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChannelOverrideState {
    pub channel: u8,
    pub instrument_path: String,
    pub instrument_name: String,
    /// dB
    pub volume: f64,
    pub muted: bool,
    pub solo: bool,
    pub inserts: Vec<InsertState>,
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

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    pub bpm: f64,
    pub playing: bool,
    pub default_instrument_path: Option<String>,
    pub midi: Option<MidiState>,
    pub overrides: Vec<ChannelOverrideState>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfoView {
    pub name: String,
    pub path: String,
    /// "Sf2" | "Vst3" | "Clap" | "Sfz" — matches the legacy debug format.
    pub format: String,
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
    muted: bool,
    solo: bool,
    inserts: Vec<InsertState>,
}

struct Inner {
    sample_rate: u32,
    buffer_size: u32,
    runtime: Option<moonlitt_audio_io::Runtime>,
    master_track_id: Option<u32>,
    default_instrument_path: Option<String>,
    overrides: Vec<Override>,
    midi: Option<MidiState>,
    bpm: f64,
    playing: bool,
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
                overrides: Vec::new(),
                midi: None,
                bpm: 120.0,
                playing: false,
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
            default_instrument_path: s.default_instrument_path.clone(),
            midi: s.midi.clone(),
            overrides: s
                .overrides
                .iter()
                .map(|o| ChannelOverrideState {
                    channel: o.channel,
                    instrument_path: o.instrument_path.clone(),
                    instrument_name: o.instrument_name.clone(),
                    volume: o.volume,
                    muted: o.muted,
                    solo: o.solo,
                    inserts: o.inserts.clone(),
                })
                .collect(),
        }
    }

    pub fn meter_snapshot(&self) -> Vec<f32> {
        let s = self.inner.lock();
        let Some(rt) = s.runtime.as_ref() else {
            return vec![0.0, 0.0];
        };
        let mut out = Vec::with_capacity(2 + s.overrides.len() * 2);
        let (ml, mr) = rt.master_levels();
        out.push(ml);
        out.push(mr);
        for o in &s.overrides {
            let (l, r) = rt.track_levels(o.native_track_id);
            out.push(l);
            out.push(r);
        }
        out
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
        self.ensure_runtime()?;
        let (sr, buf) = {
            let s = self.inner.lock();
            (s.sample_rate, s.buffer_size)
        };
        let backend = moonlitt_engine::create(path, sr, buf).map_err(|e| format!("{e}"))?;
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
        if channel > 15 {
            return Err(format!("invalid channel {channel}"));
        }
        self.ensure_runtime()?;
        let (sr, buf) = {
            let s = self.inner.lock();
            (s.sample_rate, s.buffer_size)
        };
        let backend = moonlitt_engine::create(path, sr, buf).map_err(|e| format!("{e}"))?;
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
            muted: false,
            solo: false,
            inserts: Vec::new(),
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
            rt.set_insert_param(track_id as u8, insert_id as u8, param_id as u16, value as f32);
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

    pub fn set_bpm(&self, bpm: f64) {
        let mut s = self.inner.lock();
        s.bpm = bpm;
        if let Some(rt) = s.runtime.as_ref() {
            rt.set_tempo(bpm);
        }
    }

    pub fn set_master_volume(&self, db: f64) {
        let mut s = self.inner.lock();
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

    pub fn audio_settings(&self) -> (u32, u32) {
        let s = self.inner.lock();
        (s.sample_rate, s.buffer_size)
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
        volume: o.volume,
        muted: o.muted,
        solo: o.solo,
        inserts: o.inserts.clone(),
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

