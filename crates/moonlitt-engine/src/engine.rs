//! Engine — factory functions for creating audio backends.
//!
//! Auto-detects file format by extension and creates the right backend.
//! Returns `Box<dyn AudioBackend>` directly — no proxy wrapper.

use crate::backend::AudioBackend;
use crate::error::EngineError;
use crate::plugin_info::PluginInfo;
#[cfg(any(feature = "vst3", feature = "clap"))]
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

    // TODO: scan for SF2 files in common directories

    plugins
}
