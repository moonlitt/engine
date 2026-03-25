//! VST3 plugin scanner
//!
//! Scans system directories for .vst3 bundles, loads each one, and
//! uses IPluginFactory to enumerate the audio classes inside.

use crate::component::enumerate_audio_classes;
use crate::module::load_module;
use crate::Result;
use std::path::{Path, PathBuf};

/// Information about a discovered VST3 plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// Human-readable name from the factory's PClassInfo.
    pub name: String,
    /// Path to the .vst3 bundle on disk.
    pub path: PathBuf,
    /// 16-byte class ID used to instantiate this plugin.
    pub class_id: [u8; 16],
    /// Category string (e.g. "Audio Module Class").
    pub category: String,
}

/// Scan default system paths for VST3 plugins.
pub fn scan_default_paths() -> Result<Vec<PluginInfo>> {
    let mut plugins = Vec::new();
    for dir in system_vst3_dirs() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "vst3") {
                    match probe_plugin(&path) {
                        Ok(mut infos) => plugins.append(&mut infos),
                        Err(_) => {
                            // Skip plugins that fail to load — common for
                            // plugins requiring specific hardware or licenses
                        }
                    }
                }
            }
        }
    }
    Ok(plugins)
}

/// Probe a specific .vst3 bundle path and return all discovered plugins.
/// This avoids scanning all system directories.
pub fn probe_path(path: &Path) -> Result<Vec<PluginInfo>> {
    probe_plugin(path)
}

/// Probe a .vst3 bundle: load it, get the factory, enumerate audio classes.
fn probe_plugin(path: &Path) -> Result<Vec<PluginInfo>> {
    let module = load_module(path)?;
    let classes = enumerate_audio_classes(&module)?;

    Ok(classes
        .into_iter()
        .map(|ci| PluginInfo {
            name: ci.name,
            path: path.to_path_buf(),
            class_id: ci.cid,
            category: ci.category,
        })
        .collect())
}

fn system_vst3_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(format!(
                "{home}/Library/Audio/Plug-Ins/VST3"
            )));
        }
    }

    #[cfg(target_os = "windows")]
    {
        dirs.push(PathBuf::from(r"C:\Program Files\Common Files\VST3"));
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(format!("{home}/.vst3")));
        }
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
    }

    dirs
}
