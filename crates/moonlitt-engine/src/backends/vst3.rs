//! VST3 backend — wraps moonlitt_vst3 behind AudioBackend.

use crate::backend::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo, PresetInfo};
use moonlitt_vst3::{Vst3Host, Vst3Plugin};

pub struct Vst3Backend {
    host: Vst3Host,
    plugin: Option<Vst3Plugin>,
    sample_rate: u32,
    #[allow(dead_code)]
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

        // Probe the specific .vst3 bundle directly — no full system scan needed.
        let plugin = self
            .host
            .load_from_path(std::path::Path::new(path))
            .map_err(|e| format!("failed to load VST3 at {path}: {e}"))?;
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
            if let Err(e) = plugin.render(left, right) {
                eprintln!("[moonlitt] VST3 render error: {e}");
            }
        }
    }

    fn process_effect(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        if let Some(ref mut plugin) = self.plugin {
            if let Err(e) = plugin.process_effect(in_l, in_r, out_l, out_r) {
                eprintln!("[moonlitt] VST3 effect error: {e}");
            }
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // VST3 volume is typically controlled via plugin parameters
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

    fn param_count(&self) -> u32 {
        self.plugin.as_ref().map(|p| p.param_count()).unwrap_or(0)
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        let plugin = self.plugin.as_ref()?;
        let vinfo = plugin.param_info(index)?;
        let mut flags = ParamFlags::empty();
        if vinfo.is_hidden || vinfo.is_program_change { flags |= ParamFlags::HIDDEN; }
        if vinfo.is_readonly { flags |= ParamFlags::READONLY; }
        if vinfo.step_count > 0 { flags |= ParamFlags::STEPPED; }
        Some(ParamInfo {
            id: vinfo.id,
            name: if vinfo.name.is_empty() { vinfo.short_name.clone() } else { vinfo.name },
            group: String::new(), // VST3 units could be mapped here
            min: 0.0,
            max: 1.0, // VST3 uses normalized 0-1
            default: vinfo.default_normalized,
            step_count: vinfo.step_count,
            flags,
        })
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        self.plugin.as_ref()?.get_param(id)
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if let Some(ref mut plugin) = self.plugin {
            plugin.set_param(id, value);
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        self.plugin.as_ref()?.param_display(id, value)
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
