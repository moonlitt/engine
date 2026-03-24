//! # moonlitt-vst3
//!
//! Pure Rust VST3 plugin hosting. Load, play, and render any VST3 instrument or effect.
//!
//! ```no_run
//! use moonlitt_vst3::{Vst3Host, Vst3Plugin};
//!
//! let host = Vst3Host::new(44100, 256).unwrap();
//! let plugins = host.scan().unwrap();
//! let mut plugin = host.load(&plugins[0]).unwrap();
//! plugin.note_on(0, 60, 100);
//! // plugin.render(&mut left, &mut right).unwrap();
//! ```

mod module;
mod host;
mod component;
mod processor;
mod events;
mod scanner;
mod error;

pub use error::{Error, Result};
pub use scanner::PluginInfo;

/// VST3 host — scans, loads, and manages VST3 plugins
pub struct Vst3Host {
    sample_rate: f64,
    buffer_size: usize,
}

/// A loaded and initialized VST3 plugin instance
pub struct Vst3Plugin {
    // Will hold ComPtr<IComponent>, ComPtr<IAudioProcessor>, etc.
    _placeholder: (),
}

impl Vst3Host {
    pub fn new(sample_rate: u32, buffer_size: u32) -> Result<Self> {
        Ok(Self {
            sample_rate: sample_rate as f64,
            buffer_size: buffer_size as usize,
        })
    }

    /// Scan default system paths for VST3 plugins
    pub fn scan(&self) -> Result<Vec<PluginInfo>> {
        scanner::scan_default_paths()
    }

    /// Load a plugin from PluginInfo
    pub fn load(&self, _info: &PluginInfo) -> Result<Vst3Plugin> {
        todo!("Phase 1: implement plugin loading")
    }
}
