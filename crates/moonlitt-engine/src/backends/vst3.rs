//! VST3 backend — wraps moonlitt_vst3 behind AudioBackend.

use crate::backend::{AudioBackend, BackendInfo, BackendType, PresetInfo};
use moonlitt_vst3::{Vst3Host, Vst3Plugin};

pub struct Vst3Backend {
    host: Vst3Host,
    plugin: Option<Vst3Plugin>,
    sample_rate: u32,
    buffer_size: u32,
}

impl Vst3Backend {
    pub fn new(sample_rate: u32, buffer_size: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let host = Vst3Host::new(sample_rate, buffer_size)
            .map_err(|e| format!("failed to create VST3 host: {e}"))?;
        Ok(Self {
            host,
            plugin: None,
            sample_rate,
            buffer_size,
        })
    }
}

impl AudioBackend for Vst3Backend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "VST3",
            backend_type: BackendType::PluginHost,
            extensions: &["vst3"],
        }
    }

    fn load(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.unload();

        // Scan the specific bundle to find the first audio class
        let plugins = self.host.scan()?;
        let plugin_info = plugins
            .into_iter()
            .find(|p| p.path.to_string_lossy() == path)
            .ok_or_else(|| {
                // If not found in default scan, try loading it directly
                format!("VST3 plugin not found at: {path}")
            });

        // If not found in scan, try a direct approach:
        // Scan the specific path by temporarily scanning it.
        let info = match plugin_info {
            Ok(info) => info,
            Err(_) => {
                // Try direct load by scanning the parent directory
                // For now, scan default paths and match
                let all_plugins = self.host.scan()?;
                all_plugins
                    .into_iter()
                    .find(|p| p.path.to_string_lossy() == path)
                    .ok_or_else(|| format!("VST3 plugin not found: {path}"))?
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
        // VST3 doesn't use MIDI program change directly;
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
        // VST3 volume is typically controlled via plugin parameters
        // or a gain stage — not directly applicable here
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn presets(&self) -> Vec<PresetInfo> {
        if let Some(ref plugin) = self.plugin {
            match plugin.presets() {
                Ok(presets) => presets
                    .into_iter()
                    .map(|p| PresetInfo {
                        id: p.program_index,
                        name: p.name,
                    })
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            vec![]
        }
    }

    fn load_preset(&mut self, id: i32) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref mut plugin) = self.plugin {
            plugin
                .load_preset(id)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
        } else {
            Err("no plugin loaded".into())
        }
    }
}
