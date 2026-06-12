//! VST3 plugin scanner
//!
//! Scans system directories for .vst3 bundles, loads each one, and
//! uses IPluginFactory to enumerate the audio classes inside.

use crate::component::enumerate_audio_classes;
use crate::module::load_module;
use crate::scan_cache::{bundle_mtime_ns, PluginScanCache};
use crate::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Broad routing classification derived from the VST3 `subCategories`
/// string. DAWs use this to decide whether a freshly-instantiated plug-in
/// should be fed MIDI (instrument) or audio (effect).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    /// Plug-in produces audio in response to MIDI events. Subcategories
    /// start with "Instrument".
    Instrument,
    /// Plug-in processes audio. Subcategories start with "Fx".
    Effect,
    /// Couldn't classify — old/buggy plug-in or non-standard subcategory
    /// (e.g. "Analyzer", "Generator", empty).
    Unknown,
}

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
    /// Pipe-separated subcategory tags from PClassInfo2 (e.g.
    /// "Instrument|Synth", "Fx|Reverb"). `None` when the factory only
    /// implements the legacy IPluginFactory (no PClassInfo2).
    pub subcategories: Option<String>,
    /// Vendor name from PClassInfo2 (e.g. "Modartt", "Surge Synth Team").
    pub vendor: Option<String>,
    /// Plug-in version string from PClassInfo2 (free-form).
    pub version: Option<String>,
}

impl PluginInfo {
    /// Routing classification derived from [`Self::subcategories`].
    pub fn kind(&self) -> PluginKind {
        let Some(sub) = self.subcategories.as_deref() else {
            return PluginKind::Unknown;
        };
        // VST3 spec: tags are pipe-separated. First tag is the primary
        // classification. Empty string → Unknown.
        let first = sub.split('|').next().unwrap_or("").trim();
        match first {
            "Instrument" => PluginKind::Instrument,
            "Fx" => PluginKind::Effect,
            _ => PluginKind::Unknown,
        }
    }
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

/// Like [`scan_default_paths`] but consults `cache` to skip bundles whose
/// mtime hasn't advanced since the last scan. Re-probes any new or
/// modified bundle, then evicts entries whose path no longer exists so
/// the cache file doesn't grow unboundedly across uninstall cycles.
pub fn scan_default_paths_cached(cache: &mut PluginScanCache) -> Result<Vec<PluginInfo>> {
    let mut plugins = Vec::new();
    let mut live_paths: HashSet<PathBuf> = HashSet::new();

    for dir in system_vst3_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "vst3") {
                continue;
            }
            live_paths.insert(path.clone());

            let mtime = bundle_mtime_ns(&path);
            if let Some(cached) = cache.fresh_entry(&path, mtime) {
                plugins.extend(cached);
                continue;
            }
            // Cold path — actually dlopen and probe.
            match probe_plugin(&path) {
                Ok(infos) => {
                    cache.upsert(path.clone(), mtime, &infos);
                    plugins.extend(infos);
                }
                Err(_) => {
                    // Don't cache failures — they may be transient (locked
                    // license file, missing hardware). Try again next scan.
                }
            }
        }
    }

    cache.retain_paths(&live_paths);
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
            subcategories: ci.subcategories,
            vendor: ci.vendor,
            version: ci.version,
        })
        .collect())
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
