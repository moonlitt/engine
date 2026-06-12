//! Per-plugin "last used patch" cache.
//!
//! Sample-streamer plug-ins (Keyscape, Omnisphere) are SILENT with
//! their default state — picking one as an instrument and hearing
//! nothing is the single worst first-run experience in the app. This
//! cache remembers the last captured state blob per plug-in path, so
//! the next time the user picks that plug-in it comes up sounding like
//! the last patch they used, instantly, with warm-up handled.
//!
//! Written by the patch-poll loop (whenever a state change is observed
//! in an open GUI) and on project save; read whenever an instrument is
//! assigned without an explicit state.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use tauri::Manager;

fn cache_file(app: &tauri::AppHandle, plugin_path: &str) -> Option<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()?
        .join("plugin-default-states");
    std::fs::create_dir_all(&dir).ok()?;
    let stem = Path::new(plugin_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin".into());
    let mut h = DefaultHasher::new();
    plugin_path.hash(&mut h);
    Some(dir.join(format!("{stem}-{:08x}.mlstate", h.finish() as u32)))
}

/// Remember `state` as the default patch for `plugin_path`.
pub fn store(app: &tauri::AppHandle, plugin_path: &str, state: &[u8]) {
    if state.is_empty() {
        return;
    }
    if let Some(file) = cache_file(app, plugin_path) {
        let _ = std::fs::write(file, state);
    }
}

/// The last remembered patch for `plugin_path`, if any.
pub fn load(app: &tauri::AppHandle, plugin_path: &str) -> Option<Vec<u8>> {
    let file = cache_file(app, plugin_path)?;
    std::fs::read(file).ok().filter(|b| !b.is_empty())
}
