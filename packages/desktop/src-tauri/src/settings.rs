//! App-level preferences, persisted as JSON in the app-data dir.
//!
//! Distinct from project files: these follow the USER across projects.
//! Currently just the preferred default instrument — the "GM playback
//! bed" every new project starts from (see `cmd_default_autopick`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;

const SETTINGS_FILE: &str = "settings.json";

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Instrument the user last picked as the project default — applied
    /// automatically to new projects. `None` until the first manual pick.
    pub preferred_default_instrument: Option<String>,
}

fn settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(SETTINGS_FILE))
}

pub fn load(app: &tauri::AppHandle) -> AppSettings {
    settings_path(app)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(app: &tauri::AppHandle, settings: &AppSettings) {
    if let (Some(path), Ok(json)) = (settings_path(app), serde_json::to_vec_pretty(settings)) {
        let _ = std::fs::write(path, json);
    }
}

/// Remember the user's manual default-instrument pick as the
/// cross-project preference.
pub fn remember_preferred_default(app: &tauri::AppHandle, path: &str) {
    let mut s = load(app);
    if s.preferred_default_instrument.as_deref() == Some(path) {
        return;
    }
    s.preferred_default_instrument = Some(path.to_string());
    save(app, &s);
}
