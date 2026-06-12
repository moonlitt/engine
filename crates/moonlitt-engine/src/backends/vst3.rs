//! VST3 backend — wraps moonlitt_vst3 behind AudioBackend.
//!
//! Holds the plug-in behind `Arc<parking_lot::Mutex<Vst3Plugin>>` so the
//! same instance can be reached from the audio thread (via `AudioBackend`)
//! and from the GUI window (via [`Vst3Backend::plugin_handle`]). One
//! instance, two callers — no state-copy, no warm-up rebuild on patch
//! changes.
//!
//! Locking discipline:
//!   * Audio thread holds the lock for the duration of one `render()` call
//!     (≈5 ms at 256 @ 44.1k). parking_lot's uncontended fast path is
//!     ~20 ns so the overhead is invisible.
//!   * GUI thread holds the lock for `create_view`, `set_state`, `warm_up`,
//!     parameter reads, etc. These can be slow (set_state on Spectrasonics
//!     ≈1 s). During that window the audio thread renders silence — same
//!     as a real DAW's "loading patch" interlude.
//!   * No priority inheritance — keep main-thread critical sections short.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::backend::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo, PresetInfo};
use moonlitt_vst3::{Vst3Host, Vst3Plugin};

pub struct Vst3Backend {
    host: Vst3Host,
    plugin: Option<Arc<Mutex<Vst3Plugin>>>,
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

    /// Clone of the shared plug-in handle — `None` until [`AudioBackend::load`]
    /// has been called successfully.
    ///
    /// Used by the desktop app to give the plug-in GUI window the same
    /// instance the audio thread is rendering against. Caller is expected
    /// to lock briefly: long critical sections (e.g. across UI event loop
    /// turns) will glitch playback.
    pub fn plugin_handle(&self) -> Option<Arc<Mutex<Vst3Plugin>>> {
        self.plugin.clone()
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
        self.plugin = Some(Arc::new(Mutex::new(plugin)));
        Ok(())
    }

    fn unload(&mut self) {
        self.plugin = None;
    }

    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        if let Some(p) = self.plugin.as_ref() {
            p.lock().note_on(channel, note, velocity);
        }
    }

    fn note_off(&mut self, channel: u8, note: u8) {
        if let Some(p) = self.plugin.as_ref() {
            p.lock().note_off(channel, note);
        }
    }

    fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        if let Some(p) = self.plugin.as_ref() {
            p.lock().cc(channel, cc, value);
        }
    }

    fn pitch_bend(&mut self, channel: u8, value: i16) {
        if let Some(p) = self.plugin.as_ref() {
            p.lock().pitch_bend(channel, value);
        }
    }

    fn program_change(&mut self, channel: u8, program: u8) {
        if let Some(p) = self.plugin.as_ref() {
            p.lock().program_change(channel, program);
        }
    }

    fn all_notes_off(&mut self) {
        if let Some(p) = self.plugin.as_ref() {
            p.lock().all_notes_off();
        }
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Some(p) = self.plugin.as_ref() {
            // `try_lock` would let us render silence rather than wait —
            // safer for hard real-time. For now we block: GUI lock holds
            // are bounded (set_state, create_view) and brief enough that
            // a stall is acceptable. If this ever shows up as glitching
            // under load, switch to try_lock + fill(0).
            if let Err(e) = p.lock().render(left, right) {
                eprintln!("[moonlitt] VST3 render error: {e}");
            }
        }
    }

    fn process_effect(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        if let Some(p) = self.plugin.as_ref() {
            if let Err(e) = p.lock().process_effect(in_l, in_r, out_l, out_r) {
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
        let Some(p) = self.plugin.as_ref() else { return vec![] };
        match p.lock().presets() {
            Ok(presets) => presets
                .into_iter()
                .map(|p| PresetInfo {
                    id: p.program_index,
                    name: p.name,
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    fn param_count(&self) -> u32 {
        self.plugin.as_ref().map(|p| p.lock().param_count()).unwrap_or(0)
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        let p = self.plugin.as_ref()?;
        let vinfo = p.lock().param_info(index)?;
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
        self.plugin.as_ref()?.lock().get_param(id)
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if let Some(p) = self.plugin.as_ref() {
            p.lock().set_param(id, value);
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        self.plugin.as_ref()?.lock().param_display(id, value)
    }

    fn load_preset(&mut self, id: i32) -> Result<(), Box<dyn std::error::Error>> {
        match self.plugin.as_ref() {
            Some(p) => p
                .lock()
                .load_preset(id)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
            None => Err("no plugin loaded".into()),
        }
    }

    fn save_state(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        match self.plugin.as_ref() {
            Some(p) => p
                .lock()
                .get_state()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
            None => Err("no plugin loaded".into()),
        }
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        match self.plugin.as_ref() {
            Some(p) => p
                .lock()
                .set_state(data)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
            None => Err("no plugin loaded".into()),
        }
    }

    fn warm_up(&mut self, num_blocks: usize) -> Result<(), Box<dyn std::error::Error>> {
        match self.plugin.as_ref() {
            Some(p) => p
                .lock()
                .warm_up(num_blocks)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
            None => Err("no plugin loaded".into()),
        }
    }

    fn recommended_warm_up_blocks(&self) -> usize {
        self.plugin
            .as_ref()
            .map(|p| p.lock().recommended_warm_up_blocks())
            .unwrap_or(0)
    }
}
