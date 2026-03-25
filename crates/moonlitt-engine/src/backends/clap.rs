//! CLAP backend — wraps moonlitt_clap behind AudioBackend.

use crate::backend::{AudioBackend, BackendInfo, BackendType};
use moonlitt_clap::{ClapHost, ClapPlugin};

pub struct ClapBackend {
    host: ClapHost,
    plugin: Option<ClapPlugin>,
    sample_rate: u32,
    #[allow(dead_code)]
    buffer_size: u32,
}

impl ClapBackend {
    pub fn new(sample_rate: u32, buffer_size: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let host = ClapHost::new(sample_rate, buffer_size)
            .map_err(|e| format!("failed to create CLAP host: {e}"))?;
        Ok(Self {
            host,
            plugin: None,
            sample_rate,
            buffer_size,
        })
    }
}

impl AudioBackend for ClapBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "CLAP",
            backend_type: BackendType::PluginHost,
            extensions: &["clap"],
        }
    }

    fn load(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.unload();

        // Scan to find the plugin at this path, then load the first one
        let plugins = self.host.scan()?;
        let plugin_info = plugins
            .into_iter()
            .find(|p| p.path.to_string_lossy() == path);

        let info = match plugin_info {
            Some(info) => info,
            None => {
                // Try scanning again (may be a race)
                let all_plugins = self.host.scan()?;
                all_plugins
                    .into_iter()
                    .find(|p| p.path.to_string_lossy() == path)
                    .ok_or_else(|| format!("CLAP plugin not found: {path}"))?
            }
        };

        let plugin = self.host.load(&info)?;
        self.plugin = Some(plugin);
        Ok(())
    }

    fn unload(&mut self) {
        self.plugin = None;
    }

    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        if let Some(ref mut plugin) = self.plugin {
            plugin.note_on(channel, note, velocity);
        }
    }

    fn note_off(&mut self, channel: u8, note: u8) {
        if let Some(ref mut plugin) = self.plugin {
            plugin.note_off(channel, note);
        }
    }

    fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        if let Some(ref mut plugin) = self.plugin {
            plugin.cc(channel, cc, value);
        }
    }

    fn pitch_bend(&mut self, channel: u8, value: i16) {
        if let Some(ref mut plugin) = self.plugin {
            plugin.pitch_bend(channel, value);
        }
    }

    fn program_change(&mut self, _channel: u8, _program: u8) {
        // CLAP doesn't use MIDI program change directly;
        // use presets via load_preset instead
    }

    fn all_notes_off(&mut self) {
        if let Some(ref mut plugin) = self.plugin {
            plugin.all_notes_off();
        }
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Some(ref mut plugin) = self.plugin {
            let _ = plugin.render(left, right);
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // CLAP volume is typically controlled via plugin parameters
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
