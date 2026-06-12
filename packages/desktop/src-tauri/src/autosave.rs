//! Crash-safe session journal.
//!
//! The engine state is continuously mirrored to
//! `app_data/autosave.mlsession.json`; on boot the journal is restored
//! before anything else, so a crash / force-quit / dev-server restart
//! never loses an unsaved project.
//!
//! SAFETY: the journal is captured from the window state STASH only
//! (event-driven, refreshed on notification-quiet/window-close/save) —
//! autosave NEVER calls `getState` on a live plug-in. A periodic timer
//! that probed plug-in state directly would reintroduce the
//! mid-load-getState crash this codebase already paid for once.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

const AUTOSAVE_FILE: &str = "autosave.mlsession.json";

/// The journal: the session plus the app-level context the frontend
/// needs to put the user back where they were.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutosaveEnvelope {
    /// The .mlsession file this state derives from; `None` = untitled.
    pub source_path: Option<String>,
    /// Were there unsaved changes relative to `source_path`?
    pub dirty: bool,
    pub session: moonlitt_session::persistence::Session,
}

/// Last-known project context, kept backend-side so the app-exit hook
/// can journal without asking the (already gone) frontend.
static LAST_META: Mutex<(Option<String>, bool)> = Mutex::new((None, false));

pub fn remember_meta(source_path: Option<String>, dirty: bool) {
    *LAST_META.lock().unwrap_or_else(|e| e.into_inner()) = (source_path, dirty);
}

fn autosave_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(AUTOSAVE_FILE))
}

/// Capture the current engine state into the journal. Plugin states
/// come from the stash — no plug-in code runs.
pub fn write(app: &tauri::AppHandle, engine: &crate::engine::Engine) {
    let plugin_states = stash_only_states();
    let session = engine.capture_session(&plugin_states);
    let (source_path, dirty) = LAST_META.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let envelope = AutosaveEnvelope {
        source_path,
        dirty,
        session,
    };
    if let (Some(path), Ok(json)) = (autosave_path(app), serde_json::to_vec(&envelope)) {
        // Write-then-rename so a crash mid-write can't corrupt the
        // only copy of the user's unsaved work.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub fn read(app: &tauri::AppHandle) -> Option<AutosaveEnvelope> {
    let bytes = std::fs::read(autosave_path(app)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn stash_only_states() -> std::collections::HashMap<String, Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        crate::plugin_window::stash_snapshot()
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::collections::HashMap::new()
    }
}
