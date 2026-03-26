//! CLAP backend — wraps moonlitt_clap behind AudioBackend.

use crate::backend::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};
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

        // Probe the specific .clap bundle directly — no full system scan needed.
        let plugin = self
            .host
            .load_from_path(std::path::Path::new(path))
            .map_err(|e| format!("failed to load CLAP at {path}: {e}"))?;
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
            if let Err(e) = plugin.render(left, right) {
                eprintln!("[moonlitt] CLAP render error: {e}");
            }
        }
    }

    fn process_effect(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        if let Some(ref mut plugin) = self.plugin {
            if let Err(e) = plugin.process_effect(in_l, in_r, out_l, out_r) {
                eprintln!("[moonlitt] CLAP effect error: {e}");
            }
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // CLAP volume is typically controlled via plugin parameters
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn param_count(&self) -> u32 {
        self.plugin.as_ref().map(|p| p.param_count()).unwrap_or(0)
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        let plugin = self.plugin.as_ref()?;
        let cinfo = plugin.param_info(index)?;
        // Extract group from module path (e.g., "Synth/Oscillator" → "Synth")
        let group = cinfo.module.split('/').next().unwrap_or("").to_string();
        let mut flags = ParamFlags::empty();
        // CLAP param flag constants (from clap-sys)
        const IS_HIDDEN: u32 = 1 << 2;
        const IS_READONLY: u32 = 1 << 3;
        const IS_STEPPED: u32 = 1 << 0;
        if cinfo.flags & IS_HIDDEN != 0 { flags |= ParamFlags::HIDDEN; }
        if cinfo.flags & IS_READONLY != 0 { flags |= ParamFlags::READONLY; }
        if cinfo.flags & IS_STEPPED != 0 { flags |= ParamFlags::STEPPED; }
        Some(ParamInfo {
            id: cinfo.id,
            name: cinfo.name,
            group,
            min: cinfo.min,
            max: cinfo.max,
            default: cinfo.default,
            step_count: 0,
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
}
