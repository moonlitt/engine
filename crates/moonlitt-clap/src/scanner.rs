//! CLAP plugin scanner.
//!
//! Scans system directories for .clap bundles, loads each one,
//! and enumerates the plugins inside via clap_plugin_factory.

use crate::module::ClapModule;
use crate::Result;
use std::path::{Path, PathBuf};

/// Information about a discovered CLAP plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// Human-readable name from the plugin descriptor.
    pub name: String,
    /// Path to the .clap bundle on disk.
    pub path: PathBuf,
    /// Plugin ID string (reverse-dns style, e.g. "com.modartt.pianoteq").
    pub plugin_id: String,
    /// Plugin description (may be empty).
    pub description: String,
    /// Plugin vendor.
    pub vendor: String,
    /// Index within the factory (for multi-plugin bundles).
    pub index: u32,
}

/// Scan default system paths for CLAP plugins.
pub fn scan_default_paths() -> Result<Vec<PluginInfo>> {
    let mut plugins = Vec::new();
    for dir in system_clap_dirs() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if is_clap_bundle(&path) {
                    match probe_plugin(&path) {
                        Ok(mut infos) => plugins.append(&mut infos),
                        Err(_) => {
                            // Skip plugins that fail to load
                        }
                    }
                }
            }
        }
    }
    Ok(plugins)
}

/// Probe a specific .clap bundle path and return all discovered plugins.
/// This avoids scanning all system directories.
pub fn probe_path(path: &Path) -> Result<Vec<PluginInfo>> {
    probe_plugin(path)
}

/// Probe a .clap bundle: load it, get the factory, enumerate plugins.
fn probe_plugin(path: &Path) -> Result<Vec<PluginInfo>> {
    let module = ClapModule::load(path)?;
    let count = module.plugin_count();

    let mut plugins = Vec::with_capacity(count as usize);
    for i in 0..count {
        if let Some(desc) = module.plugin_descriptor(i) {
            plugins.push(PluginInfo {
                name: desc.name,
                path: path.to_path_buf(),
                plugin_id: desc.id,
                description: desc.description,
                vendor: desc.vendor,
                index: i,
            });
        }
    }

    Ok(plugins)
}

/// Check if a path looks like a .clap bundle.
fn is_clap_bundle(path: &Path) -> bool {
    path.extension().map_or(false, |e| e == "clap")
}

/// Return system directories where CLAP plugins are typically installed.
fn system_clap_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(format!(
                "{home}/Library/Audio/Plug-Ins/CLAP"
            )));
        }
    }

    #[cfg(target_os = "windows")]
    {
        dirs.push(PathBuf::from(r"C:\Program Files\Common Files\CLAP"));
        if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
            dirs.push(PathBuf::from(format!("{localappdata}\\Programs\\Common\\CLAP")));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(format!("{home}/.clap")));
        }
        dirs.push(PathBuf::from("/usr/lib/clap"));
        dirs.push(PathBuf::from("/usr/local/lib/clap"));
    }

    dirs
}
