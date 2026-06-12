//! macOS plugin GUI window.
//!
//! Hosts a VST3 plugin's `IPlugView` inside a sibling Tauri window so users
//! can interact with the plugin's native UI (Keyscape, Omnisphere, Kontakt,
//! Surge, …).
//!
//! **Single-instance design.** The plug-in this window drives is the *same*
//! `Vst3Plugin` the audio thread renders against — `Engine` holds the Arc,
//! we clone it. Picking a patch in the GUI is therefore audible the moment
//! the plug-in's internal sample-streamer finishes its fade-in. No
//! state-copy, no backend rebuild, no warm-up button. See
//! `moonlitt_engine::backends::vst3::Vst3Backend` for the locking
//! discipline this relies on.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use moonlitt_vst3::{platform, Vst3PluginView};
use parking_lot::Mutex;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

use crate::engine::Vst3PluginHandle;

/// Which engine slot a GUI window is editing. Used to look the plug-in
/// up in [`crate::engine::Engine`] at open time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowTarget {
    Default,
    Channel(u8),
}

/// One open plugin GUI: keeps the shared plug-in handle and its view
/// alive for the lifetime of the window. Dropping `OpenWindow` releases
/// the view (detached first) but does NOT drop the plug-in — `Engine`
/// holds another Arc clone for the audio thread.
struct OpenWindow {
    label: String,
    /// Absolute path to the .vst3 bundle this window is showing — used
    /// to key the state stash.
    path: String,
    /// Engine slot this window edits. Currently used only by tests and
    /// for diagnostics; the single-instance design means we no longer
    /// need it to route state pushes anywhere.
    #[allow(dead_code)]
    target: WindowTarget,
    /// Shared handle to the underlying plug-in. Lock briefly for ops
    /// that need plug-in mutation (state save, parameter reads); long
    /// holds will starve the audio thread.
    plugin: Vst3PluginHandle,
    /// The plug-in's editor view. Owns its own COM ref-count to
    /// `IPlugView` so we can drop the plug-in handle clone (and even
    /// the plug-in itself) without invalidating the view.
    view: Vst3PluginView,
}

// SAFETY: `Vst3PluginView` is `!Send` by default because it wraps an
// IPlugView COM pointer that AppKit expects on the main thread. We only
// touch this map from the main thread (Tauri's window-event thread + the
// JS command dispatcher), so the cross-thread move via `Mutex` is fine.
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

/// Latest known-good state captured per plug-in path. Populated when a
/// GUI window opens (initial state), refreshed on every session-save and
/// on window close. Survives across window close so the user can pick a
/// patch → close GUI → ⌘S and still get their patch in the saved
/// session.
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

/// Open a native plug-in window driving the same `Vst3Plugin` instance
/// the audio thread renders against. Caller is responsible for passing
/// the handle obtained from [`crate::engine::Engine::vst3_plugin_handle`].
pub fn open_plugin_window(
    app: AppHandle,
    path: String,
    target: WindowTarget,
    plugin: Vst3PluginHandle,
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

    // Create the editor view and seed the stash from the live plug-in
    // state up front. Lock scope is bounded to these two operations —
    // we release before building the Tauri window (slow) and attaching
    // the NSView (might call back into the plug-in).
    let (view, initial_state) = {
        let p = plugin.lock();
        let view = p
            .create_view()
            .ok_or_else(|| "plugin has no editor view".to_string())?;
        let initial_state = p.get_state().ok();
        (view, initial_state)
    };
    if let Some(bytes) = initial_state {
        put_in_stash(path.clone(), bytes);
    }

    if !view.is_platform_supported(platform::NS_VIEW) {
        return Err("plugin's IPlugView does not support NSView embedding".to_string());
    }
    let (w, h) = view.get_size();
    let resizable = view.can_resize();

    let label = format!(
        "plugin_gui_{}",
        LABEL_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let title = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Plugin GUI")
        .to_string();

    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("about:blank".into()))
        .title(title)
        .inner_size(w as f64, h as f64)
        .resizable(resizable)
        .build()
        .map_err(|e| format!("create window: {e}"))?;

    // NOTE: deliberately NOT calling IPlugViewContentScaleSupport here.
    // On macOS, NSView carries the backing scale natively and ViewRect
    // stays in logical points; passing 2.0 makes Spectrasonics double
    // its UI size (2320×1340 window). The call only belongs on a future
    // Windows/Linux port.

    // Host frame: plug-ins that discover their real editor size during
    // attach (Spectrasonics reports a small default before, then asks
    // for its real size — often huge when its UI zoom is cranked up)
    // request it through IPlugFrame::resizeView. Fit-and-centre; the
    // resulting Resized event negotiates the final size back through
    // checkSizeConstraint/onSize below.
    let frame_window = window.clone();
    if let Err(e) = view.set_frame(move |fw, fh| {
        fit_window_to_screen(&frame_window, fw, fh);
    }) {
        eprintln!("[plugin-window] set_frame failed (resize requests ignored): {e}");
    }

    let ns_view_ptr = ns_view_ptr_from(&window)?;
    view.attach(ns_view_ptr, platform::NS_VIEW)
        .map_err(|e| format!("IPlugView::attached: {e}"))?;

    // Some plug-ins only report their true size once attached — sync
    // the window to whatever the view settled on.
    let (aw, ah) = view.get_size();
    fit_window_to_screen(&window, aw, ah);
    let _ = view.on_size(aw, ah);

    let event_label = label.clone();
    let event_app = app.clone();
    let event_window = window.clone();
    window.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
                cleanup_window(&event_app, &event_label);
            }
            tauri::WindowEvent::Resized(size) => {
                // User-driven resize: negotiate via checkSizeConstraint,
                // then tell the plug-in. Plugin-initiated resizes echo
                // through here too — the extra onSize with an identical
                // rect is harmless per spec.
                let scale = event_window.scale_factor().unwrap_or(1.0);
                let logical: tauri::LogicalSize<f64> = size.to_logical(scale);
                let (rw, rh) = (logical.width as i32, logical.height as i32);
                let reg = registry().lock();
                if let Some(entry) = reg.iter().find(|o| o.label == event_label) {
                    let (cw, ch) = entry.view.check_size_constraint(rw, rh);
                    if (cw, ch) != (rw, rh) {
                        let _ = event_window
                            .set_size(tauri::LogicalSize::new(cw as f64, ch as f64));
                    }
                    let _ = entry.view.on_size(cw, ch);
                }
            }
            _ => {}
        }
    });

    let label_for_caller = label.clone();
    registry().lock().push(OpenWindow {
        label,
        path,
        target,
        plugin,
        view,
    });
    Ok(label_for_caller)
}

/// Number of open GUI windows. Cheap check used by the patch-name poll
/// loop to skip work when there's nothing to do.
pub fn open_window_count() -> usize {
    registry().lock().len()
}

/// Drain host-side notification queues (`IComponentHandler2::setDirty`,
/// `IUnitHandler` program/unit changes, …) for every open GUI window
/// and return the paths that showed activity.
///
/// SAFETY-CRITICAL DESIGN NOTE: this reads OUR queues — plain Rust,
/// never calls into plug-in code — so it is safe to run at any moment,
/// including while the plug-in's own threads are mid-patch-load.
/// `getState`, by contrast, segfaults inside Spectrasonics when called
/// during an internal instrument load (observed 2026-06-12: SIGSEGV in
/// Keyscape under a 500 ms state poll). State capture therefore only
/// happens in [`capture_state_for`] after a quiet period, on window
/// close, on save, and after library patch loads — never on a blind
/// timer.
pub fn poll_activity() -> Vec<String> {
    let reg = registry().lock();
    reg.iter()
        .filter_map(|entry| {
            // `try_lock` ↔ "if the audio thread isn't using it right
            // now". A miss is fine — we poll the queues often.
            entry.plugin.try_lock().and_then(|mut p| {
                let notes = p.take_host_notifications();
                (!notes.is_empty()).then(|| entry.path.clone())
            })
        })
        .collect()
}

/// Capture the live state for one open window's plug-in, refresh the
/// stash, and return the parsed patch name + bytes. `None` when the
/// window is gone or the plug-in is busy (caller re-arms and retries).
pub fn capture_state_for(path: &str) -> Option<(Option<String>, Vec<u8>)> {
    let plugin = {
        let reg = registry().lock();
        reg.iter().find(|o| o.path == path)?.plugin.clone()
    };
    let bytes = plugin.try_lock()?.get_state().ok()?;
    let parsed = crate::state_metadata::extract_patch_name(&bytes);
    put_in_stash(path.to_string(), bytes.clone());
    Some((parsed, bytes))
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
                    .lock()
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
/// Still useful as a one-plug-in export escape hatch even though normal
/// project save goes through the session capture path.
pub fn save_state_for_label(label: &str, path: &Path) -> Result<usize, String> {
    let plugin = {
        let reg = registry().lock();
        reg.iter()
            .find(|o| o.label == label)
            .ok_or_else(|| format!("no open plug-in window with label \"{label}\""))?
            .plugin
            .clone()
    };
    let bytes = plugin
        .lock()
        .get_state()
        .map_err(|e| format!("get_state: {e}"))?;
    std::fs::write(path, &bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(bytes.len())
}

fn cleanup_window(app: &AppHandle, label: &str) {
    let removed: Option<(String, Vst3PluginHandle)> = {
        let mut reg = registry().lock();
        if let Some(idx) = reg.iter().position(|o| o.label == label) {
            let entry = reg.remove(idx);
            let path = entry.path.clone();
            let plugin = entry.plugin.clone();
            // Detach view before dropping the entry — the plug-in keeps
            // running on the audio thread; the view is what we're
            // letting go of.
            let _ = entry.view.detach();
            drop(entry);
            Some((path, plugin))
        } else {
            None
        }
    };

    let Some((path, plugin)) = removed else {
        return;
    };

    // Final state capture, OFF the main thread and after a settle
    // delay: the user may close the window a beat after picking a
    // patch, while the sampler's own threads are still loading it —
    // Spectrasonics' getState crashes when probed mid-load.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let Ok(bytes) = plugin.lock().get_state() else {
            return;
        };
        let patch_name = crate::state_metadata::extract_patch_name(&bytes);
        put_in_stash(path.clone(), bytes.clone());
        crate::plugin_state_cache::store(&app, &path, &bytes);

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
    });
}

/// Size the window to `(w, h)` logical points, clamped to the current
/// monitor's dimensions (small margin for the title bar / dock), then
/// centre it. Plug-ins with a cranked-up UI zoom can legitimately ask
/// for editors wider than the screen — a clipped-off-screen editor is
/// useless, so we cap and let checkSizeConstraint negotiate.
fn fit_window_to_screen(window: &tauri::WebviewWindow, w: i32, h: i32) {
    const MARGIN: f64 = 60.0;
    let monitor = window.current_monitor().ok().flatten();
    let Some(monitor) = monitor else {
        let _ = window.set_size(tauri::LogicalSize::new(w as f64, h as f64));
        return;
    };
    let scale = monitor.scale_factor();
    let msize = monitor.size().to_logical::<f64>(scale);
    let mpos = monitor.position().to_logical::<f64>(scale);

    let fit_w = (w as f64).min(msize.width - MARGIN).max(200.0);
    let fit_h = (h as f64).min(msize.height - MARGIN).max(150.0);
    let _ = window.set_size(tauri::LogicalSize::new(fit_w, fit_h));

    // Centre manually on THIS monitor — Tauri's `center()` spans the
    // whole virtual desktop on multi-monitor setups, which can park a
    // wide editor half-way across two screens.
    let x = mpos.x + (msize.width - fit_w) / 2.0;
    let y = mpos.y + (msize.height - fit_h) / 2.0;
    let _ = window.set_position(tauri::LogicalPosition::new(x.max(mpos.x), y.max(mpos.y + 24.0)));
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
