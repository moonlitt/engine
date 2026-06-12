//! Recent-projects list, persisted in the Tauri app-data directory.
//!
//! Stored as `recent.json` — a small JSON document with a fixed cap of
//! 10 entries (oldest evicted on overflow). Failures to read/write the
//! file are non-fatal — the UI just shows an empty recent list.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

const MAX_RECENT: usize = 10;
const RECENT_FILE: &str = "recent.json";

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecentState {
    /// Absolute paths to recently opened/saved `.mlsession` files,
    /// most-recent first.
    pub recent: Vec<PathBuf>,
    /// Last-opened project — auto-restored on next launch if present.
    /// `None` means "show the Welcome view".
    pub last_opened: Option<PathBuf>,
}

impl RecentState {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))
    }

    fn promote(&mut self, p: PathBuf) {
        // Remove any prior occurrence, then push to front.
        self.recent.retain(|existing| existing != &p);
        self.recent.insert(0, p.clone());
        if self.recent.len() > MAX_RECENT {
            self.recent.truncate(MAX_RECENT);
        }
        self.last_opened = Some(p);
    }
}

fn recent_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    Ok(dir.join(RECENT_FILE))
}

/// Read the recent list. Returns an empty `RecentState` if the file
/// doesn't exist or is malformed (best-effort UX).
pub fn read(app: &tauri::AppHandle) -> RecentState {
    match recent_path(app) {
        Ok(p) => RecentState::load(&p),
        Err(_) => RecentState::default(),
    }
}

/// Record a project as the most-recently-touched (open OR save).
/// Best-effort — failures don't propagate to the UI.
pub fn record(app: &tauri::AppHandle, project_path: &Path) {
    let Ok(file) = recent_path(app) else { return };
    let mut state = RecentState::load(&file);
    state.promote(project_path.to_path_buf());
    let _ = state.save(&file);
}

/// Clear the recent list — used by "Clear recent files" menu item.
pub fn clear(app: &tauri::AppHandle) -> Result<(), String> {
    let file = recent_path(app)?;
    let state = RecentState::default();
    state.save(&file)
}

/// Forget a specific entry — used when an opened file vanishes from disk.
pub fn forget(app: &tauri::AppHandle, project_path: &Path) {
    let Ok(file) = recent_path(app) else { return };
    let mut state = RecentState::load(&file);
    state.recent.retain(|p| p != project_path);
    if state.last_opened.as_deref() == Some(project_path) {
        state.last_opened = None;
    }
    let _ = state.save(&file);
}
