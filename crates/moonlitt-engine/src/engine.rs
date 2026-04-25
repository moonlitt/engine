//! Engine — factory functions for creating audio backends.
//!
//! Auto-detects file format by extension and creates the right backend.
//! Returns `Box<dyn AudioBackend>` directly — no proxy wrapper.

use crate::backend::AudioBackend;
use crate::error::EngineError;
use crate::plugin_info::PluginInfo;
#[cfg(any(feature = "vst3", feature = "clap", feature = "sf2"))]
use crate::plugin_info::PluginFormat;
use std::path::Path;

/// Create an audio backend by auto-detecting format from file extension.
///
/// Supports `.sf2` (SoundFont), `.vst3` (VST3 plugin), `.clap` (CLAP plugin).
#[allow(unused_variables)]
pub fn create(path: &str, sample_rate: u32, buffer_size: u32) -> Result<Box<dyn AudioBackend>, EngineError> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        #[cfg(feature = "sf2")]
        Some("sf2") => {
            let mut backend = crate::backends::oxisynth::OxiSynthBackend::new(sample_rate)
                .map_err(|e| EngineError::BackendError(e.to_string()))?;
            backend
                .load(path)
                .map_err(|e| EngineError::BackendError(e.to_string()))?;
            Ok(Box::new(backend))
        }
        #[cfg(feature = "vst3")]
        Some("vst3") => {
            let mut backend =
                crate::backends::vst3::Vst3Backend::new(sample_rate, buffer_size)
                    .map_err(|e| EngineError::BackendError(e.to_string()))?;
            backend
                .load(path)
                .map_err(|e| EngineError::BackendError(e.to_string()))?;
            Ok(Box::new(backend))
        }
        #[cfg(feature = "clap")]
        Some("clap") => {
            let mut backend =
                crate::backends::clap::ClapBackend::new(sample_rate, buffer_size)
                    .map_err(|e| EngineError::BackendError(e.to_string()))?;
            backend
                .load(path)
                .map_err(|e| EngineError::BackendError(e.to_string()))?;
            Ok(Box::new(backend))
        }
        Some(ext) => Err(EngineError::UnsupportedFormat(ext.to_string())),
        None => Err(EngineError::UnsupportedFormat("no file extension".into())),
    }
}

/// Create an audio backend with highest quality interpolation (Sinc72 for SF2).
/// Use for offline rendering. Real-time uses SeventhOrder by default.
pub fn create_high_quality(path: &str, sample_rate: u32, buffer_size: u32) -> Result<Box<dyn AudioBackend>, EngineError> {
    #[cfg(feature = "sf2")]
    if path.to_lowercase().ends_with(".sf2") {
        let mut backend = crate::backends::oxisynth::OxiSynthBackend::new_high_quality(sample_rate)
            .map_err(|e| EngineError::BackendError(e.to_string()))?;
        backend.load(path).map_err(|e| EngineError::BackendError(e.to_string()))?;
        return Ok(Box::new(backend));
    }
    create(path, sample_rate, buffer_size)
}

/// Create an audio backend from a pre-loaded SF2 SoundFont (Arc-shared, no data copy).
#[cfg(feature = "sf2")]
pub fn create_from_shared_sf2(font: oxisynth::SoundFont, sample_rate: u32) -> Result<Box<dyn AudioBackend>, EngineError> {
    let backend = crate::backends::oxisynth::OxiSynthBackend::new_with_font(sample_rate, font)
        .map_err(|e| EngineError::BackendError(e.to_string()))?;
    Ok(Box::new(backend))
}

/// Create an SF2 backend using moonlitt-sampler (pure Rust, Sinc 72 interpolation).
///
/// Drop-in replacement for `create()` when feature `sf2-sampler` is enabled.
/// Only valid for `.sf2` files.
#[cfg(feature = "sf2-sampler")]
pub fn create_with_sampler(path: &str, sample_rate: u32, _buffer_size: u32) -> Result<Box<dyn AudioBackend>, EngineError> {
    if !path.to_lowercase().ends_with(".sf2") {
        return Err(EngineError::UnsupportedFormat(
            "create_with_sampler only supports .sf2 files".into()
        ));
    }
    let mut backend = crate::backends::sampler::SamplerBackend::new(sample_rate)
        .map_err(|e| EngineError::BackendError(e.to_string()))?;
    backend.load(path)
        .map_err(|e| EngineError::BackendError(e.to_string()))?;
    Ok(Box::new(backend))
}

/// Return the list of file extensions supported by the engine.
#[allow(clippy::vec_init_then_push)]
pub fn supported_formats() -> Vec<&'static str> {
    let mut formats = Vec::new();
    #[cfg(feature = "sf2")]
    formats.push("sf2");
    #[cfg(feature = "vst3")]
    formats.push("vst3");
    #[cfg(feature = "clap")]
    formats.push("clap");
    formats
}

/// Scan system paths for available plugins (VST3, CLAP, SF2).
#[allow(unused_variables, unused_mut)]
pub fn scan_plugins(sample_rate: u32, buffer_size: u32) -> Vec<PluginInfo> {
    let mut plugins = Vec::new();

    #[cfg(feature = "vst3")]
    {
        if let Ok(host) = moonlitt_vst3::Vst3Host::new(sample_rate, buffer_size) {
            if let Ok(vst3_plugins) = host.scan() {
                for p in vst3_plugins {
                    plugins.push(PluginInfo {
                        name: p.name,
                        path: p.path.to_string_lossy().into_owned(),
                        format: PluginFormat::Vst3,
                    });
                }
            }
        }
    }

    #[cfg(feature = "clap")]
    {
        if let Ok(host) = moonlitt_clap::ClapHost::new(sample_rate, buffer_size) {
            if let Ok(clap_plugins) = host.scan() {
                for p in clap_plugins {
                    plugins.push(PluginInfo {
                        name: p.name,
                        path: p.path.to_string_lossy().into_owned(),
                        format: PluginFormat::Clap,
                    });
                }
            }
        }
    }

    #[cfg(feature = "sf2")]
    {
        let _ = (sample_rate, buffer_size); // unused in SF2 scan, kept for signature parity
        scan_sf2_into(&mut plugins);
    }

    plugins
}

/// Scan common SoundFont locations and append discovered `.sf2` files to `out`.
///
/// Search order (each entry recursively walked, max depth 4):
/// 1. `$MOONLITT_SF2_DIR` (env var, colon-separated)
/// 2. `~/Library/Audio/Sounds/Banks` (macOS standard SoundFont dir)
/// 3. `~/Documents/Soundfonts`
/// 4. `<workspace>/tests` and `<workspace>/deps/oxisynth/testdata` for dev convenience
///
/// Caps at 100 entries to bound the cost of pathological dirs. Files smaller
/// than 4 KiB are skipped — too small to be a real soundfont and likely test
/// fixtures or junk.
#[cfg(feature = "sf2")]
fn scan_sf2_into(out: &mut Vec<PluginInfo>) {
    use std::path::PathBuf;

    const MAX_RESULTS: usize = 100;
    const MAX_DEPTH: usize = 4;
    const MIN_SIZE_BYTES: u64 = 4 * 1024;

    let mut search_dirs: Vec<PathBuf> = Vec::new();

    if let Ok(env) = std::env::var("MOONLITT_SF2_DIR") {
        for raw in env.split(':') {
            if !raw.is_empty() {
                search_dirs.push(PathBuf::from(raw));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        search_dirs.push(home.join("Library/Audio/Sounds/Banks"));
        search_dirs.push(home.join("Documents/Soundfonts"));
    }
    // Dev convenience — works when binary is run from the workspace root.
    search_dirs.push(PathBuf::from("tests"));
    search_dirs.push(PathBuf::from("deps/oxisynth/testdata"));

    let mut seen = std::collections::HashSet::<PathBuf>::new();

    for root in search_dirs {
        if out.len() >= MAX_RESULTS {
            break;
        }
        walk_sf2(&root, 0, MAX_DEPTH, MIN_SIZE_BYTES, MAX_RESULTS, &mut seen, out);
    }
}

#[cfg(feature = "sf2")]
fn walk_sf2(
    dir: &std::path::Path,
    depth: usize,
    max_depth: usize,
    min_size: u64,
    max_results: usize,
    seen: &mut std::collections::HashSet<std::path::PathBuf>,
    out: &mut Vec<PluginInfo>,
) {
    if depth > max_depth || out.len() >= max_results {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if out.len() >= max_results {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_sf2(&path, depth + 1, max_depth, min_size, max_results, seen, out);
        } else if path.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("sf2")).unwrap_or(false) {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(canonical.clone()) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.len() < min_size {
                    continue;
                }
            }
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("(unnamed)").to_string();
            out.push(PluginInfo {
                name,
                path: canonical.to_string_lossy().into_owned(),
                format: PluginFormat::Sf2,
            });
        }
    }
}
