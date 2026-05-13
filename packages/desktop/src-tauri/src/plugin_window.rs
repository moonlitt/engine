//! macOS plugin GUI window.
//!
//! Hosts a VST3 plugin's `IPlugView` inside a sibling Tauri window so users
//! can interact with the plugin's native UI (Keyscape, Omnisphere, Kontakt,
//! Surge, …). Headless plugins still play through the audio path; this just
//! exposes their UI on demand.
//!
//! Trade-off (MVP): the plugin instance opened here is *separate* from the
//! one in the audio mixer. Patches loaded in the GUI do not affect playback
//! until we wire `IComponentHandler` parameter messaging in a follow-up.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use moonlitt_vst3::{platform, Vst3Host, Vst3Plugin, Vst3PluginView};
use parking_lot::Mutex;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

/// Which engine slot a GUI window is editing. Mirrors
/// `commands::CmdViewTarget` shapes so we can route state pushes to the
/// audio-thread back-end correctly when the user closes the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowTarget {
    Default,
    Channel(u8),
}

/// One open plugin GUI: keeps the plugin and its view alive for the
/// lifetime of the window.
struct OpenWindow {
    label: String,
    /// Absolute path to the .vst3 bundle this window is showing — used to
    /// key the state stash so closing the window doesn't lose the patch
    /// the user picked.
    path: String,
    /// Where this plug-in's state should be pushed when captured: the
    /// engine's default-instrument slot or a specific MIDI-channel
    /// override. Set at open time so close-time sync knows the
    /// destination.
    target: WindowTarget,
    /// Held only to keep the COM-loaded plugin alive while the GUI window
    /// is shown — view internals refer back into it via raw pointers.
    plugin: Vst3Plugin,
    view: Vst3PluginView,
}

// SAFETY: the underlying COM objects + AppKit views live on the main thread
// only — we only ever poke this map from the main-thread close handler and
// from Tauri commands that dispatch into the main thread. We use a Mutex
// for synchronisation across those two callers.
unsafe impl Send for OpenWindow {}

static OPEN_WINDOWS: OnceLock<Mutex<Vec<OpenWindow>>> = OnceLock::new();
static LABEL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Bytes + parsed patch name for one captured state. The name is
/// extracted at stash time via `state_metadata::extract_patch_name` so
/// lookups stay cheap.
struct StashedState {
    bytes: Vec<u8>,
    patch_name: Option<String>,
}

/// Latest known-good state captured per plug-in path. Populated when
/// a GUI window is opened (initial state) AND refreshed every time
/// session save is triggered. Survives across window close so the user
/// can pick a patch → close GUI → ⌘S and still get their patch in the
/// saved session.
static STATE_STASH: OnceLock<Mutex<HashMap<String, StashedState>>> = OnceLock::new();

fn registry() -> &'static Mutex<Vec<OpenWindow>> {
    OPEN_WINDOWS.get_or_init(|| Mutex::new(Vec::new()))
}

fn state_stash() -> &'static Mutex<HashMap<String, StashedState>> {
    STATE_STASH.get_or_init(|| Mutex::new(HashMap::new()))
}

fn put_in_stash(path: String, bytes: Vec<u8>) {
    let patch_name = crate::state_metadata::extract_patch_name(&bytes);
    state_stash()
        .lock()
        .insert(path, StashedState { bytes, patch_name });
}

pub fn open_plugin_window(
    app: AppHandle,
    path: String,
    target: WindowTarget,
    sr: u32,
    buf: u32,
) -> Result<String, String> {
    let extension_ok = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("vst3"))
        .unwrap_or(false);
    if !extension_ok {
        return Err(format!(
            "plugin {path} has no native GUI (only .vst3 supported here)"
        ));
    }

    // Build a dedicated GUI plugin instance. Doing this on the calling
    // thread is fine — VST3 component creation does not require the main
    // thread; only attaching the view to an NSView does.
    let host = Vst3Host::new(sr, buf).map_err(|e| format!("vst3 host: {e}"))?;
    let plugin = host
        .load_from_path(Path::new(&path))
        .map_err(|e| format!("vst3 load: {e}"))?;
    let view = plugin
        .create_view()
        .ok_or_else(|| "plugin has no editor view".to_string())?;

    if !view.is_platform_supported(platform::NS_VIEW) {
        return Err("plugin's IPlugView does not support NSView embedding".to_string());
    }
    let (w, h) = view.get_size();

    let label = format!(
        "plugin_gui_{}",
        LABEL_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let title = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Plugin GUI")
        .to_string();

    // Build the Tauri window. The webview is empty/transparent — the
    // plugin will add its own NSView on top of it.
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("about:blank".into()))
        .title(title)
        .inner_size(w as f64, h as f64)
        .resizable(false)
        .build()
        .map_err(|e| format!("create window: {e}"))?;

    let ns_view_ptr = ns_view_ptr_from(&window)?;
    view.attach(ns_view_ptr, platform::NS_VIEW)
        .map_err(|e| format!("IPlugView::attached: {e}"))?;
    let _ = view.on_size(w, h);

    // Wire close → drop plugin + view from the registry.
    let close_label = label.clone();
    let close_app = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed) {
            cleanup_window(&close_app, &close_label);
        }
    });

    let label_for_caller = label.clone();
    registry().lock().push(OpenWindow {
        label,
        path: path.clone(),
        target,
        plugin,
        view,
    });
    Ok(label_for_caller)
}

/// Walk every currently-open GUI window, call `getState()` on its
/// plug-in, and merge into the stash keyed by .vst3 path. Returns the
/// full stash bytes-only (open windows + previously-captured
/// closed-window state) so callers get a single map keyed by plug-in
/// path.
///
/// Used by session save: ⌘S calls this to refresh the stash from any
/// patches the user has just been editing before serialising.
pub fn snapshot_all_open_states() -> HashMap<String, Vec<u8>> {
    {
        let reg = registry().lock();
        let paths_and_bytes: Vec<(String, Vec<u8>)> = reg
            .iter()
            .filter_map(|entry| {
                entry
                    .plugin
                    .get_state()
                    .ok()
                    .map(|b| (entry.path.clone(), b))
            })
            .collect();
        drop(reg);
        for (path, bytes) in paths_and_bytes {
            put_in_stash(path, bytes);
        }
    }
    state_stash()
        .lock()
        .iter()
        .map(|(k, v)| (k.clone(), v.bytes.clone()))
        .collect()
}

/// Manually inject a state blob into the stash. Used when the engine
/// loads a session — the audio-thread back-end gets the state directly,
/// but we also remember it here so a subsequent ⌘S doesn't lose it.
pub fn stash_state(path: String, state: Vec<u8>) {
    put_in_stash(path, state);
}

/// Look up the parsed patch name for a plug-in path. Returns `None` if
/// no state has been captured yet for this path, or if the plug-in's
/// state blob doesn't embed a recognisable patch name (most non-
/// Spectrasonics plug-ins).
pub fn patch_name_for(path: &str) -> Option<String> {
    state_stash()
        .lock()
        .get(path)
        .and_then(|s| s.patch_name.clone())
}

/// Capture the current plug-in state for the GUI window identified by
/// `label` and write it to `path`. Returns the number of bytes written.
///
/// This is the "configure once, replay forever" pivot for sample-based
/// plug-ins (Keyscape, Omnisphere) that pick patches via private GUI
/// rather than via VST3 program change. After the user picks a patch in
/// the plug-in's UI, calling this gives a binary blob that
/// `Vst3Plugin::set_state` can rehydrate later from a headless context.
pub fn save_state_for_label(label: &str, path: &Path) -> Result<usize, String> {
    let bytes = {
        let reg = registry().lock();
        let entry = reg
            .iter()
            .find(|o| o.label == label)
            .ok_or_else(|| format!("no open plug-in window with label \"{label}\""))?;
        entry
            .plugin
            .get_state()
            .map_err(|e| format!("get_state: {e}"))?
    };
    std::fs::write(path, &bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(bytes.len())
}

fn cleanup_window(app: &AppHandle, label: &str) {
    let captured: Option<(String, WindowTarget, Vec<u8>, Option<String>)> = {
        let mut reg = registry().lock();
        if let Some(idx) = reg.iter().position(|o| o.label == label) {
            let entry = reg.remove(idx);
            let path = entry.path.clone();
            let target = entry.target;
            // Capture state BEFORE we drop the plug-in. The plug-in is
            // about to drop; any later session-save would otherwise see
            // no state for this path.
            let result = entry.plugin.get_state().ok().map(|bytes| {
                let parsed = crate::state_metadata::extract_patch_name(&bytes);
                state_stash().lock().insert(
                    path.clone(),
                    StashedState {
                        bytes: bytes.clone(),
                        patch_name: parsed.clone(),
                    },
                );
                (path, target, bytes, parsed)
            });
            // Detach the plugin's view from our NSView before dropping —
            // otherwise the plugin's view holds a reference to a soon-dead
            // parent and may crash on its own cleanup pass.
            let _ = entry.view.detach();
            drop(entry); // explicit; clarifies intent
            result
        } else {
            None
        }
    };

    let Some((path, target, bytes, patch_name)) = captured else {
        return;
    };

    // Sync to the audio-thread back-end so the freshly-picked patch
    // actually plays — without this, the GUI instance has the patch
    // but the audio instance is still empty and notes are silent.
    // Heavy work (dlopen + load_state + warm_up ~1 s) so done on a
    // background thread; we just hand it off and return.
    apply_state_to_engine(app.clone(), path.clone(), target, bytes);

    // Tell the frontend the new patch name is available so the UI can
    // refresh without polling.
    use serde::Serialize;
    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct PluginStateCaptured {
        path: String,
        patch_name: Option<String>,
    }
    use tauri::Emitter;
    let _ = app.emit(
        "plugin_state_captured",
        PluginStateCaptured { path, patch_name },
    );
}

/// Spawn a background thread that rebuilds the audio-thread back-end
/// with the captured state. Used by `cleanup_window` and by the
/// manual "apply" command so the user can sync without closing.
fn apply_state_to_engine(
    app: AppHandle,
    path: String,
    target: WindowTarget,
    bytes: Vec<u8>,
) {
    use tauri::Manager;
    std::thread::spawn(move || {
        let Some(state) = app.try_state::<crate::commands::AppState>() else {
            return;
        };
        let result = match target {
            WindowTarget::Default => state
                .engine
                .set_default_instrument_with_state(&path, Some(&bytes))
                .map(|_| ()),
            WindowTarget::Channel(ch) => state
                .engine
                .set_channel_override_with_state(ch, &path, Some(&bytes))
                .map(|_| ()),
        };
        if let Err(e) = result {
            eprintln!("apply_state_to_engine: {e}");
        }
    });
}

/// Capture the state of a still-open GUI window and apply it to the
/// audio engine without closing. Used by the "🎵 应用音色" button so
/// users can iterate on patches without close/open cycles.
pub fn apply_open_state_to_engine(app: AppHandle, label: &str) -> Result<Option<String>, String> {
    let info: Option<(String, WindowTarget, Vec<u8>, Option<String>)> = {
        let reg = registry().lock();
        let entry = reg
            .iter()
            .find(|o| o.label == label)
            .ok_or_else(|| format!("no open plug-in window with label \"{label}\""))?;
        let bytes = entry
            .plugin
            .get_state()
            .map_err(|e| format!("get_state: {e}"))?;
        let parsed = crate::state_metadata::extract_patch_name(&bytes);
        Some((entry.path.clone(), entry.target, bytes, parsed))
    };
    let (path, target, bytes, patch_name) = info.unwrap();
    state_stash().lock().insert(
        path.clone(),
        StashedState {
            bytes: bytes.clone(),
            patch_name: patch_name.clone(),
        },
    );
    apply_state_to_engine(app, path, target, bytes);
    Ok(patch_name)
}

fn ns_view_ptr_from(window: &tauri::WebviewWindow) -> Result<*mut c_void, String> {
    let handle = window
        .window_handle()
        .map_err(|e| format!("window_handle: {e}"))?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(h) => Ok(h.ns_view.as_ptr()),
        other => Err(format!("expected AppKit window handle, got {other:?}")),
    }
}
