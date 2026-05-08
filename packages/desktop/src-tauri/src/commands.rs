//! Tauri command handlers — thin facade around `Engine`.
//!
//! Wire-format mirrors `packages/protocol`: each command takes its
//! arguments, mutates state, and the relevant change is broadcast as a
//! Tauri event. The frontend listens via `@tauri-apps/api/event::listen`.
//!
//! All `cmd_*` functions return `Result<T, String>` so JS gets a thrown
//! error on failure (matching the WebSocket-era `error` event semantics).

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::engine::{
    ChannelOverrideState, Engine, InsertState, MidiState, PluginInfoView, ProjectState, ViewTarget,
};

pub struct AppState {
    pub engine: Engine,
}

// --- Event emit helpers ---------------------------------------------------

#[derive(Serialize, Clone)]
struct TransportState {
    playing: bool,
    position: u64,
}

#[derive(Serialize, Clone)]
struct TempoChanged {
    bpm: f64,
}

#[derive(Serialize, Clone)]
struct DefaultInstrumentChanged {
    #[serde(rename = "instrumentPath")]
    instrument_path: Option<String>,
}

#[derive(Serialize, Clone)]
struct ChannelOverrideAdded {
    #[serde(rename = "override")]
    o: ChannelOverrideState,
}

#[derive(Serialize, Clone)]
struct ChannelOverrideRemoved {
    channel: u8,
}

#[derive(Serialize, Clone)]
struct ChannelUpdated {
    channel: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    solo: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "userProgram")]
    user_program: Option<u8>,
}

#[derive(Serialize, Clone)]
struct InsertAdded {
    channel: u8,
    insert: InsertState,
}

#[derive(Serialize, Clone)]
struct InsertRemoved {
    channel: u8,
    #[serde(rename = "insertId")]
    insert_id: u32,
}

#[derive(Serialize, Clone)]
struct PluginsList {
    plugins: Vec<PluginInfoView>,
}

#[derive(Serialize, Clone)]
struct MidiLoaded {
    midi: MidiState,
}

// --- Project snapshot -----------------------------------------------------

#[tauri::command]
pub fn cmd_snapshot(state: State<AppState>) -> ProjectState {
    state.engine.snapshot()
}

// --- Transport ------------------------------------------------------------

#[tauri::command]
pub fn cmd_transport_play(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    state.engine.play()?;
    let _ = app.emit(
        "transport:state",
        TransportState {
            playing: true,
            position: 0,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_transport_stop(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    state.engine.stop();
    let _ = app.emit(
        "transport:state",
        TransportState {
            playing: false,
            position: 0,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_transport_set_bpm(
    state: State<AppState>,
    app: AppHandle,
    bpm: f64,
) -> Result<(), String> {
    state.engine.set_bpm(bpm);
    let _ = app.emit("transport:tempo_changed", TempoChanged { bpm });
    Ok(())
}

// --- Master ---------------------------------------------------------------

#[tauri::command]
pub fn cmd_master_set_volume(state: State<AppState>, db: f64) -> Result<(), String> {
    state.engine.set_master_volume(db);
    Ok(())
}

// --- Default instrument ---------------------------------------------------

#[tauri::command]
pub fn cmd_default_set_instrument(
    state: State<AppState>,
    app: AppHandle,
    path: String,
) -> Result<(), String> {
    state.engine.set_default_instrument(&path)?;
    let _ = app.emit(
        "default:instrument_changed",
        DefaultInstrumentChanged {
            instrument_path: Some(path),
        },
    );
    Ok(())
}

// --- Per-channel overrides -----------------------------------------------

#[tauri::command]
pub fn cmd_channel_set_override(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    path: String,
) -> Result<(), String> {
    let ov = state.engine.set_channel_override(channel, &path)?;
    let _ = app.emit("channel:override_added", ChannelOverrideAdded { o: ov });
    Ok(())
}

#[tauri::command]
pub fn cmd_channel_remove_override(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
) -> Result<(), String> {
    state.engine.remove_channel_override(channel)?;
    let _ = app.emit("channel:override_removed", ChannelOverrideRemoved { channel });
    Ok(())
}

#[tauri::command]
pub fn cmd_channel_set_volume(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    db: f64,
) -> Result<(), String> {
    state.engine.set_channel_volume(channel, db)?;
    let _ = app.emit(
        "channel:updated",
        ChannelUpdated {
            channel,
            volume: Some(db),
            muted: None,
            solo: None,
            user_program: None,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_channel_set_mute(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    muted: bool,
) -> Result<(), String> {
    state.engine.set_channel_mute(channel, muted)?;
    let _ = app.emit(
        "channel:updated",
        ChannelUpdated {
            channel,
            volume: None,
            muted: Some(muted),
            solo: None,
            user_program: None,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_channel_set_solo(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    solo: bool,
) -> Result<(), String> {
    state.engine.set_channel_solo(channel, solo)?;
    let _ = app.emit(
        "channel:updated",
        ChannelUpdated {
            channel,
            volume: None,
            muted: None,
            solo: Some(solo),
            user_program: None,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_channel_set_program(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    program: u8,
) -> Result<(), String> {
    state.engine.set_channel_program(channel, program)?;
    let _ = app.emit(
        "channel:updated",
        ChannelUpdated {
            channel,
            volume: None,
            muted: None,
            solo: None,
            user_program: Some(program),
        },
    );
    Ok(())
}

// --- Inserts --------------------------------------------------------------

#[tauri::command]
pub fn cmd_insert_add(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    effect_type: String,
) -> Result<InsertState, String> {
    let insert = state.engine.add_insert(channel, &effect_type)?;
    let _ = app.emit(
        "insert:added",
        InsertAdded {
            channel,
            insert: insert.clone(),
        },
    );
    Ok(insert)
}

#[tauri::command]
pub fn cmd_insert_remove(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    insert_id: u32,
) -> Result<(), String> {
    state.engine.remove_insert(channel, insert_id)?;
    let _ = app.emit(
        "insert:removed",
        InsertRemoved {
            channel,
            insert_id,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_insert_set_param(
    state: State<AppState>,
    channel: u8,
    insert_id: u32,
    param_id: u32,
    value: f64,
) -> Result<(), String> {
    state
        .engine
        .set_insert_param(channel, insert_id, param_id, value)
}

// --- Plugin discovery ----------------------------------------------------

#[tauri::command]
pub fn cmd_plugins_scan(
    state: State<AppState>,
    app: AppHandle,
    force: Option<bool>,
) -> Vec<PluginInfoView> {
    let list = state.engine.scan_plugins(force.unwrap_or(false));
    let _ = app.emit(
        "plugins:list",
        PluginsList {
            plugins: list.clone(),
        },
    );
    list
}

// --- MIDI loading --------------------------------------------------------

#[tauri::command]
pub fn cmd_load_midi(
    state: State<AppState>,
    app: AppHandle,
    path: String,
) -> Result<MidiState, String> {
    let name = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path)
        .to_string();
    let midi = state.engine.load_midi(&path, &name)?;
    let _ = app.emit(
        "midi:loaded",
        MidiLoaded {
            midi: midi.clone(),
        },
    );
    if let Some(bpm) = midi.tempo_bpm.filter(|b| b.is_finite()) {
        let _ = app.emit("transport:tempo_changed", TempoChanged { bpm });
    }
    Ok(midi)
}

// --- VST3 GUI window ------------------------------------------------------
//
// Callers identify the target by either {kind: "default"} or
// {kind: "override", channel: N}.

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CmdViewTarget {
    Default,
    Override { channel: u8 },
}

impl From<CmdViewTarget> for ViewTarget {
    fn from(c: CmdViewTarget) -> Self {
        match c {
            CmdViewTarget::Default => ViewTarget::Default,
            CmdViewTarget::Override { channel } => ViewTarget::Channel(channel),
        }
    }
}

#[tauri::command]
pub fn cmd_open_plugin_gui(
    state: State<AppState>,
    app: AppHandle,
    target: CmdViewTarget,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let target: ViewTarget = target.into();
        let path = state.engine.instrument_path_for(target)?;
        let (sr, buf) = state.engine.audio_settings();
        crate::plugin_window::open_plugin_window(app, path, sr, buf)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, app, target);
        Err("plugin GUI window currently only implemented on macOS".to_string())
    }
}
