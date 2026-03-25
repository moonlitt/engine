//! Engine — the main entry point for moonlitt audio.
//!
//! Auto-detects file format by extension, creates the right backend,
//! and delegates all operations to it.

use crate::backend::{AudioBackend, BackendInfo, PresetInfo};
use crate::error::EngineError;
use crate::plugin_info::{PluginFormat, PluginInfo};
use std::path::Path;

pub struct Engine {
    backend: Option<Box<dyn AudioBackend>>,
    sample_rate: u32,
    buffer_size: u32,
    volume: f32,
}

impl Engine {
    /// Create a new engine with the given sample rate and buffer size.
    pub fn new(sample_rate: u32, buffer_size: u32) -> Self {
        Self {
            backend: None,
            sample_rate,
            buffer_size,
            volume: 1.0,
        }
    }

    /// Auto-detect format by file extension and load.
    pub fn load(&mut self, path: &str) -> Result<(), EngineError> {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        match ext.as_deref() {
            #[cfg(feature = "sf2")]
            Some("sf2") => {
                let mut backend = crate::backends::sf2::Sf2Backend::new(self.sample_rate)
                    .map_err(|e| EngineError::BackendError(e.to_string()))?;
                backend
                    .load(path)
                    .map_err(|e| EngineError::BackendError(e.to_string()))?;
                backend.set_volume(self.volume);
                self.backend = Some(Box::new(backend));
                Ok(())
            }
            #[cfg(feature = "vst3")]
            Some("vst3") => {
                let mut backend =
                    crate::backends::vst3::Vst3Backend::new(self.sample_rate, self.buffer_size)
                        .map_err(|e| EngineError::BackendError(e.to_string()))?;
                backend
                    .load(path)
                    .map_err(|e| EngineError::BackendError(e.to_string()))?;
                self.backend = Some(Box::new(backend));
                Ok(())
            }
            #[cfg(feature = "clap")]
            Some("clap") => {
                let mut backend =
                    crate::backends::clap::ClapBackend::new(self.sample_rate, self.buffer_size)
                        .map_err(|e| EngineError::BackendError(e.to_string()))?;
                backend
                    .load(path)
                    .map_err(|e| EngineError::BackendError(e.to_string()))?;
                self.backend = Some(Box::new(backend));
                Ok(())
            }
            Some(ext) => Err(EngineError::UnsupportedFormat(ext.to_string())),
            None => Err(EngineError::UnsupportedFormat("no file extension".into())),
        }
    }

    /// Unload current backend.
    pub fn unload(&mut self) {
        if let Some(ref mut backend) = self.backend {
            backend.unload();
        }
        self.backend = None;
    }

    /// Is a backend loaded and ready?
    pub fn is_loaded(&self) -> bool {
        self.backend.is_some()
    }

    /// Get backend info.
    pub fn backend_info(&self) -> Option<BackendInfo> {
        self.backend.as_ref().map(|b| b.info())
    }

    // --- MIDI pass-through ---

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        if let Some(ref mut backend) = self.backend {
            backend.note_on(channel, note, velocity);
        }
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        if let Some(ref mut backend) = self.backend {
            backend.note_off(channel, note);
        }
    }

    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        if let Some(ref mut backend) = self.backend {
            backend.cc(channel, cc, value);
        }
    }

    pub fn pitch_bend(&mut self, channel: u8, value: i16) {
        if let Some(ref mut backend) = self.backend {
            backend.pitch_bend(channel, value);
        }
    }

    pub fn program_change(&mut self, channel: u8, program: u8) {
        if let Some(ref mut backend) = self.backend {
            backend.program_change(channel, program);
        }
    }

    pub fn all_notes_off(&mut self) {
        if let Some(ref mut backend) = self.backend {
            backend.all_notes_off();
        }
    }

    // --- Audio ---

    /// Render one buffer of audio. Fills with silence if no backend.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Some(ref mut backend) = self.backend {
            backend.render(left, right);
        } else {
            left.fill(0.0);
            right.fill(0.0);
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn buffer_size(&self) -> u32 {
        self.buffer_size
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(ref mut backend) = self.backend {
            backend.set_volume(volume);
        }
    }

    // --- Plugin scanning ---

    /// Scan system paths for available plugins (VST3, CLAP, SF2).
    pub fn scan_plugins(&self) -> Vec<PluginInfo> {
        let mut plugins = Vec::new();

        #[cfg(feature = "vst3")]
        {
            if let Ok(host) = moonlitt_vst3::Vst3Host::new(self.sample_rate, self.buffer_size) {
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
            if let Ok(host) = moonlitt_clap::ClapHost::new(self.sample_rate, self.buffer_size) {
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

    // --- Presets ---

    pub fn presets(&self) -> Vec<PresetInfo> {
        self.backend
            .as_ref()
            .map(|b| b.presets())
            .unwrap_or_default()
    }

    pub fn load_preset(&mut self, id: i32) -> Result<(), EngineError> {
        match self.backend.as_mut() {
            Some(backend) => backend
                .load_preset(id)
                .map_err(|e| EngineError::BackendError(e.to_string())),
            None => Err(EngineError::NoBackendLoaded),
        }
    }
}
