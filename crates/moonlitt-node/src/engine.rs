//! Engine factory — create audio backends from file paths.

use napi::Result;
use napi_derive::napi;

use moonlitt_core::AudioBackend;

/// Opaque handle to an audio backend (instrument or effect).
///
/// Created via `create()` or effect factory functions.
/// Consumed when passed to `Session.create()` or `Session.addTrack()`.
#[napi]
pub struct Backend {
    pub(crate) inner: Option<Box<dyn AudioBackend>>,
}

#[napi]
impl Backend {
    /// Sample rate this backend was created with.
    #[napi]
    pub fn sample_rate(&self) -> u32 {
        self.inner.as_ref().map(|b| b.sample_rate()).unwrap_or(0)
    }

    /// Number of controllable parameters.
    #[napi]
    pub fn param_count(&self) -> u32 {
        self.inner.as_ref().map(|b| b.param_count()).unwrap_or(0)
    }

    /// Set a parameter value.
    #[napi]
    pub fn set_param(&mut self, id: u32, value: f64) {
        if let Some(b) = self.inner.as_mut() {
            b.set_param(id, value);
        }
    }

    /// Get a parameter's current value.
    #[napi]
    pub fn get_param(&self, id: u32) -> Option<f64> {
        self.inner.as_ref().and_then(|b| b.get_param(id))
    }

    /// Metadata for the parameter at the given index (0..param_count).
    /// Returns None if the index is out of range or the backend has been consumed.
    #[napi]
    pub fn param_info(&self, index: u32) -> Option<crate::types::ParamInfo> {
        self.inner.as_ref().and_then(|b| b.param_info(index)).map(|info| {
            crate::types::ParamInfo {
                id: info.id,
                name: info.name,
                group: info.group,
                min: info.min,
                max: info.max,
                default: info.default,
                step_count: info.step_count,
            }
        })
    }

    /// Human-readable display string for a parameter value (e.g., "+3.5 dB").
    #[napi]
    pub fn param_display(&self, id: u32, value: f64) -> Option<String> {
        self.inner.as_ref().and_then(|b| b.param_display(id, value))
    }

    /// Whether this backend has been consumed (passed to a Session).
    #[napi]
    pub fn is_consumed(&self) -> bool {
        self.inner.is_none()
    }
}

/// Create an audio backend from a file path (.sf2, .vst3, .clap).
///
/// Auto-detects format by file extension.
#[napi]
pub fn create(path: String, sample_rate: u32, buffer_size: u32) -> Result<Backend> {
    let backend = moonlitt_engine::create(&path, sample_rate, buffer_size)
        .map_err(|e| napi::Error::from_reason(format!("{e}")))?;
    Ok(Backend {
        inner: Some(backend),
    })
}

/// Scan system directories for available audio plugins (VST3, CLAP).
///
/// Returns metadata for each discovered plugin.
#[napi]
pub fn scan_plugins(sample_rate: u32, buffer_size: u32) -> Vec<crate::types::PluginInfo> {
    moonlitt_engine::scan_plugins(sample_rate, buffer_size)
        .into_iter()
        .map(|p| crate::types::PluginInfo {
            name: p.name,
            path: p.path,
            format: format!("{:?}", p.format),
        })
        .collect()
}

/// List file extensions the engine can load.
#[napi]
pub fn supported_formats() -> Vec<String> {
    moonlitt_engine::supported_formats()
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}
