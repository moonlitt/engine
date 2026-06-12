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
    ChannelOverrideState, Engine, InsertState, MidiState, PluginInfoView, ProjectState,
    SendBusView, ViewTarget,
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
struct LoopChanged {
    looping: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MetronomeChanged {
    enabled: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MasterUpdated {
    volume_db: f64,
}

#[derive(Serialize, Clone)]
struct DefaultInstrumentChanged {
    #[serde(rename = "instrumentPath")]
    instrument_path: Option<String>,
    /// Sample streamer with no patch yet — the UI should open the
    /// plug-in GUI and explain why there's no sound.
    #[serde(rename = "needsPatch")]
    needs_patch: bool,
}

#[derive(Serialize, Clone)]
struct ChannelOverrideAdded {
    #[serde(rename = "override")]
    o: ChannelOverrideState,
    #[serde(rename = "needsPatch")]
    needs_patch: bool,
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
    pan: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    solo: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "userProgram")]
    user_program: Option<u8>,
}

impl ChannelUpdated {
    fn for_channel(channel: u8) -> Self {
        Self {
            channel,
            volume: None,
            pan: None,
            muted: None,
            solo: None,
            color: None,
            user_program: None,
        }
    }
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
#[serde(rename_all = "camelCase")]
struct InsertBypassChanged {
    channel: u8,
    insert_id: u32,
    bypassed: bool,
}

#[derive(Serialize, Clone)]
struct SendBusAdded {
    bus: SendBusView,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelSendLevelChanged {
    channel: u8,
    bus_id: u32,
    level: f32,
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
pub fn cmd_transport_pause(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    state.engine.pause();
    // Note: position stays at its current value — pause preserves it.
    // We don't know the live tick here without reading from the sequencer,
    // so the frontend infers position from the snapshot it already has.
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
pub fn cmd_transport_set_loop(
    state: State<AppState>,
    app: AppHandle,
    looping: bool,
) -> Result<(), String> {
    state.engine.set_loop(looping);
    let _ = app.emit("transport:loop_changed", LoopChanged { looping });
    Ok(())
}

#[tauri::command]
pub fn cmd_transport_set_metronome(
    state: State<AppState>,
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    state.engine.set_metronome_enabled(enabled);
    let _ = app.emit("transport:metronome_changed", MetronomeChanged { enabled });
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
pub fn cmd_master_set_volume(
    state: State<AppState>,
    app: AppHandle,
    db: f64,
) -> Result<(), String> {
    state.engine.set_master_volume(db);
    let _ = app.emit("master:updated", MasterUpdated { volume_db: db });
    Ok(())
}

// --- Default instrument ---------------------------------------------------

#[tauri::command]
pub fn cmd_default_set_instrument(
    state: State<AppState>,
    app: AppHandle,
    path: String,
) -> Result<bool, String> {
    // Sample streamers are silent without a patch — seed the new
    // instance with the last patch this plug-in was seen using, so
    // picking Keyscape sounds immediately instead of confusing silence.
    let cached = crate::plugin_state_cache::load(&app, &path);
    let needs_patch = state
        .engine
        .set_default_instrument_with_state(&path, cached.as_deref())?;
    // Mirror the seeded state into the window stash so the snapshot's
    // patch-name lookup works before any GUI window ever opens.
    #[cfg(target_os = "macos")]
    if let Some(bytes) = &cached {
        crate::plugin_window::stash_state(path.clone(), bytes.clone());
    }
    let _ = app.emit(
        "default:instrument_changed",
        DefaultInstrumentChanged {
            instrument_path: Some(path),
            needs_patch,
        },
    );
    Ok(needs_patch)
}

// --- Per-channel overrides -----------------------------------------------

#[tauri::command]
pub fn cmd_channel_set_override(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    path: String,
) -> Result<bool, String> {
    let cached = crate::plugin_state_cache::load(&app, &path);
    let (ov, needs_patch) =
        state
            .engine
            .set_channel_override_with_state(channel, &path, cached.as_deref())?;
    #[cfg(target_os = "macos")]
    if let Some(bytes) = &cached {
        crate::plugin_window::stash_state(path.clone(), bytes.clone());
    }
    let _ = app.emit(
        "channel:override_added",
        ChannelOverrideAdded { o: ov, needs_patch },
    );
    Ok(needs_patch)
}

#[tauri::command]
pub fn cmd_channel_remove_override(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
) -> Result<(), String> {
    state.engine.remove_channel_override(channel)?;
    let _ = app.emit(
        "channel:override_removed",
        ChannelOverrideRemoved { channel },
    );
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
            volume: Some(db),
            ..ChannelUpdated::for_channel(channel)
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_channel_set_pan(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    pan: f64,
) -> Result<(), String> {
    state.engine.set_channel_pan(channel, pan)?;
    let _ = app.emit(
        "channel:updated",
        ChannelUpdated {
            pan: Some(pan),
            ..ChannelUpdated::for_channel(channel)
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
            muted: Some(muted),
            ..ChannelUpdated::for_channel(channel)
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
            solo: Some(solo),
            ..ChannelUpdated::for_channel(channel)
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_channel_set_color(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    color: Option<String>,
) -> Result<(), String> {
    state.engine.set_channel_color(channel, color.as_deref())?;
    let _ = app.emit(
        "channel:updated",
        ChannelUpdated {
            color: Some(color),
            ..ChannelUpdated::for_channel(channel)
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
            user_program: Some(program),
            ..ChannelUpdated::for_channel(channel)
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
    let _ = app.emit("insert:removed", InsertRemoved { channel, insert_id });
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

#[tauri::command]
pub fn cmd_send_bus_add(
    state: State<AppState>,
    app: AppHandle,
    effect_type: String,
) -> Result<SendBusView, String> {
    let bus = state.engine.add_send_bus(&effect_type)?;
    let _ = app.emit("send_bus:added", SendBusAdded { bus: bus.clone() });
    Ok(bus)
}

/// Set one parameter on a send bus's effect (e.g. reverb decay).
#[tauri::command]
pub fn cmd_send_bus_set_param(
    state: State<AppState>,
    bus_id: u32,
    param_id: u32,
    value: f64,
) -> Result<(), String> {
    state.engine.set_send_bus_param(bus_id, param_id, value)
}

#[tauri::command]
pub fn cmd_channel_set_send_level(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    bus_id: u32,
    level: f32,
) -> Result<(), String> {
    state
        .engine
        .set_channel_send_level(channel, bus_id, level)?;
    let _ = app.emit(
        "channel:send_level_changed",
        ChannelSendLevelChanged {
            channel,
            bus_id,
            level,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cmd_insert_set_bypass(
    state: State<AppState>,
    app: AppHandle,
    channel: u8,
    insert_id: u32,
    bypassed: bool,
) -> Result<(), String> {
    state
        .engine
        .set_insert_bypass(channel, insert_id, bypassed)?;
    let _ = app.emit(
        "insert:bypass_changed",
        InsertBypassChanged {
            channel,
            insert_id,
            bypassed,
        },
    );
    Ok(())
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
    let _ = app.emit("midi:loaded", MidiLoaded { midi: midi.clone() });
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
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let view_target: ViewTarget = target.into();
        let path = state.engine.instrument_path_for(view_target)?;
        let plugin = state
            .engine
            .vst3_plugin_handle(view_target)
            .ok_or_else(|| {
                "selected slot does not host a VST3 instrument — GUI is only available for VST3"
                    .to_string()
            })?;
        let window_target = match view_target {
            ViewTarget::Default => crate::plugin_window::WindowTarget::Default,
            ViewTarget::Channel(ch) => crate::plugin_window::WindowTarget::Channel(ch),
        };
        crate::plugin_window::open_plugin_window(app, path, window_target, plugin)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, app, target);
        Err("plugin GUI window currently only implemented on macOS".to_string())
    }
}

/// Capture the current plug-in state of an open GUI window. Used to
/// "configure once" sample-based plug-ins (Keyscape, Omnisphere) that
/// only pick patches through their private UI.
///
/// `label` comes from a prior `cmd_open_plugin_gui` call; `path` is
/// where the binary state blob will be written.
#[tauri::command]
pub fn cmd_save_plugin_state(label: String, path: String) -> Result<usize, String> {
    #[cfg(target_os = "macos")]
    {
        crate::plugin_window::save_state_for_label(&label, std::path::Path::new(&path))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (label, path);
        Err("plug-in state capture is only implemented on macOS".to_string())
    }
}

// ---------------------------------------------------------------------------
// Session (project file) commands
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSavedEvent {
    pub path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpenedEvent {
    pub path: String,
    pub project: ProjectState,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecentListView {
    pub recent: Vec<String>,
    pub last_opened: Option<String>,
}

fn collect_plugin_states() -> std::collections::HashMap<String, Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        crate::plugin_window::snapshot_all_open_states()
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::collections::HashMap::new()
    }
}

/// Save the engine's current state to a `.mlsession` JSON file.
/// `path` is the destination file (absolute path from the frontend's
/// save dialog).
#[tauri::command]
pub fn cmd_project_save_as(
    state: State<AppState>,
    app: AppHandle,
    path: String,
) -> Result<(), String> {
    let plugin_states = collect_plugin_states();
    // Project save doubles as a "remember this patch" point per plug-in.
    for (plugin_path, bytes) in &plugin_states {
        crate::plugin_state_cache::store(&app, plugin_path, bytes);
    }
    let session = state.engine.capture_session(&plugin_states);
    session
        .save_to_file(&path)
        .map_err(|e| format!("save session: {e}"))?;
    crate::recent_files::record(&app, std::path::Path::new(&path));
    let _ = app.emit("project_saved", ProjectSavedEvent { path });
    Ok(())
}

/// Load a `.mlsession` file and apply it to the engine.
#[tauri::command]
pub fn cmd_project_open(
    state: State<AppState>,
    app: AppHandle,
    path: String,
) -> Result<ProjectState, String> {
    use moonlitt_session::persistence::Session;

    let session = Session::load_from_file(&path).map_err(|e| {
        // If the file vanished, forget it from recent so the UI doesn't
        // keep showing a dead link.
        crate::recent_files::forget(&app, std::path::Path::new(&path));
        format!("open session: {e}")
    })?;
    let restored_states = state
        .engine
        .restore_session(&session)
        .map_err(|e| format!("restore: {e}"))?;

    // Refresh the desktop's plug-in-state stash so a subsequent ⌘S
    // captures the patches we just rehydrated (the user might re-save
    // without ever opening a GUI window).
    #[cfg(target_os = "macos")]
    for (p, b) in &restored_states {
        crate::plugin_window::stash_state(p.clone(), b.clone());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &restored_states;
    }

    crate::recent_files::record(&app, std::path::Path::new(&path));

    let project = state.engine.snapshot();
    let _ = app.emit(
        "project_opened",
        ProjectOpenedEvent {
            path,
            project: project.clone(),
        },
    );
    Ok(project)
}

/// Read the recent-projects list (most-recent first, capped at 10).
#[tauri::command]
pub fn cmd_project_recent_list(app: AppHandle) -> RecentListView {
    let st = crate::recent_files::read(&app);
    RecentListView {
        recent: st
            .recent
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        last_opened: st.last_opened.map(|p| p.to_string_lossy().into_owned()),
    }
}

/// Clear the entire recent list.
#[tauri::command]
pub fn cmd_project_clear_recent(app: AppHandle) -> Result<(), String> {
    crate::recent_files::clear(&app)
}

/// Remove a single entry — used when the UI detects a stale link.
#[tauri::command]
pub fn cmd_project_forget_recent(app: AppHandle, path: String) {
    crate::recent_files::forget(&app, std::path::Path::new(&path));
}

// ---------------------------------------------------------------------------
// Offline bounce
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RenderResultView {
    pub path: String,
    pub duration_secs: f64,
    pub peak: f32,
}

/// Bounce the current project (the same snapshot ⌘S would save, plus
/// the loaded MIDI clip) to a stereo float-32 WAV at `path`. Renders on
/// a **fresh engine instance** — live playback is never interrupted —
/// at full CPU speed rather than wall-clock speed. Sample streamers are
/// re-warmed automatically, so a Keyscape bounce takes a few extra
/// seconds before audio starts flowing.
#[tauri::command]
pub fn cmd_render_to_wav(state: State<AppState>, path: String) -> Result<RenderResultView, String> {
    let midi_path = state
        .engine
        .midi_path()
        .ok_or("没有已载入的 MIDI —— 先载入一个再导出")?;
    let plugin_states = collect_plugin_states();
    let session = state.engine.capture_session(&plugin_states);
    let stats = moonlitt_session::offline::render_to_wav(&session, &midi_path, &path, 256)?;
    Ok(RenderResultView {
        path,
        duration_secs: stats.duration_secs,
        peak: stats.peak,
    })
}

/// Jump the playhead to an absolute tick position. Held notes are
/// released; playback state is unchanged (seek-while-paused works).
#[tauri::command]
pub fn cmd_transport_seek(state: State<AppState>, ticks: f64) -> Result<(), String> {
    state.engine.seek(ticks)
}

// --- Spectrasonics patch library -------------------------------------------

/// Enumerate the STEAM library patches for the instrument in `target`.
/// Errors with a user-facing message when the slot isn't a
/// Spectrasonics plug-in (or its library isn't installed).
#[tauri::command]
pub fn cmd_patch_library_list(
    state: State<AppState>,
    target: CmdViewTarget,
) -> Result<Vec<crate::patch_library::PatchView>, String> {
    let path = state.engine.instrument_path_for(target.into())?;
    crate::patch_library::list(&path)
}

/// Load library patch `patch_id` (an id from `cmd_patch_library_list`)
/// into the live plug-in: current state + patch file → assembled state
/// → `set_state` on the same instance the audio thread renders. The
/// sound fades in as the streamer loads samples — no rebuild, no
/// warm-up button. Returns the parsed patch name.
#[tauri::command]
pub fn cmd_patch_library_load(
    state: State<AppState>,
    app: AppHandle,
    target: CmdViewTarget,
    patch_id: usize,
) -> Result<Option<String>, String> {
    let view_target: ViewTarget = target.into();
    let path = state.engine.instrument_path_for(view_target)?;
    let plugin = state
        .engine
        .vst3_plugin_handle(view_target)
        .ok_or_else(|| "该槽位不是 VST3 乐器".to_string())?;

    let patch = crate::patch_library::patch_at(&path, patch_id)?;
    let patch_file = crate::patch_library::patch_bytes(&patch)?;

    // One bounded lock: read current state, assemble, push. The audio
    // thread's try_lock renders silence for these few milliseconds —
    // same behaviour as picking a patch in the plug-in's own GUI.
    let new_state = {
        let mut p = plugin.lock();
        let current = p.get_state().map_err(|e| format!("get_state: {e}"))?;
        let assembled = crate::patch_library::assemble_state(&current, &patch_file)?;
        p.set_state(&assembled).map_err(|e| format!("set_state: {e}"))?;
        // Round-trip so caches hold the plug-in's own canonical bytes.
        p.get_state().unwrap_or(assembled)
    };

    let patch_name =
        crate::state_metadata::extract_patch_name(&new_state).or(Some(patch.name.clone()));

    // Same bookkeeping as a GUI-window patch pick: refresh the session
    // stash, remember as this plug-in's default, tell the UI.
    #[cfg(target_os = "macos")]
    crate::plugin_window::stash_state(path.clone(), new_state.clone());
    crate::plugin_state_cache::store(&app, &path, &new_state);

    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct PluginStateCaptured {
        path: String,
        patch_name: Option<String>,
    }
    let _ = app.emit(
        "plugin_state_captured",
        PluginStateCaptured {
            path,
            patch_name: patch_name.clone(),
        },
    );
    Ok(patch_name)
}
