use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Information about a discovered VST3 plugin
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub path: PathBuf,
    pub class_id: [u8; 16],
}

/// Scan default system paths for VST3 plugins
pub fn scan_default_paths() -> Result<Vec<PluginInfo>> {
    let mut plugins = Vec::new();
    for dir in system_vst3_dirs() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "vst3") {
                    if let Ok(info) = probe_plugin(&path) {
                        plugins.push(info);
                    }
                }
            }
        }
    }
    Ok(plugins)
}

/// Probe a .vst3 bundle and extract plugin info
fn probe_plugin(path: &Path) -> Result<PluginInfo> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    // TODO: actually load the bundle, get IPluginFactory, read class info
    Ok(PluginInfo {
        name,
        path: path.to_path_buf(),
        class_id: [0u8; 16], // placeholder
    })
}

fn system_vst3_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(format!("{home}/Library/Audio/Plug-Ins/VST3")));
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
