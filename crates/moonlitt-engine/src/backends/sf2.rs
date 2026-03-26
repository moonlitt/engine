//! SF2 backend — wraps fluidlite behind AudioBackend.

use crate::backend::{AudioBackend, BackendInfo, BackendType, PresetInfo};
use fluidlite::{IsSettings, Settings, Synth};

pub struct Sf2Backend {
    synth: Synth,
    sample_rate: u32,
    volume: f32,
    font_id: Option<u32>,
}

impl Sf2Backend {
    pub fn new(sample_rate: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let settings =
            Settings::new().map_err(|e| format!("failed to create fluidlite settings: {e}"))?;

        if let Some(sr) = settings.num("synth.sample-rate") {
            sr.set(sample_rate as f64);
        }

        let synth =
            Synth::new(settings).map_err(|e| format!("failed to create fluidlite synth: {e}"))?;

        Ok(Self {
            synth,
            sample_rate,
            volume: 1.0,
            font_id: None,
        })
    }
}

impl AudioBackend for Sf2Backend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "FluidLite",
            backend_type: BackendType::Sampler,
            extensions: &["sf2"],
        }
    }

    fn load(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.unload();

        let id = self
            .synth
            .sfload(path, true)
            .map_err(|e| format!("failed to load sf2: {e}"))?;
        self.font_id = Some(id);
        self.synth.set_gain(self.volume);

        // Enable built-in reverb and chorus with default parameters
        self.synth.set_reverb_on(true);
        self.synth.set_chorus_on(true);

        Ok(())
    }

    fn unload(&mut self) {
        if let Some(id) = self.font_id.take() {
            let _ = self.synth.sfunload(id, true);
        }
    }

    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let _ = self
            .synth
            .note_on(channel as u32, note as u32, velocity as u32);
    }

    fn note_off(&mut self, channel: u8, note: u8) {
        let _ = self.synth.note_off(channel as u32, note as u32);
    }

    fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        let _ = self.synth.cc(channel as u32, cc as u32, value as u32);
    }

    fn pitch_bend(&mut self, channel: u8, value: i16) {
        let unsigned = (value as i32 + 8192).clamp(0, 16383) as u32;
        let _ = self.synth.pitch_bend(channel as u32, unsigned);
    }

    fn program_change(&mut self, channel: u8, program: u8) {
        let _ = self
            .synth
            .program_change(channel as u32, program as u32);
    }

    fn all_notes_off(&mut self) {
        let _ = self.synth.system_reset();
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let _ = self.synth.write((left, right));
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.synth.set_gain(volume);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn presets(&self) -> Vec<PresetInfo> {
        let mut presets = Vec::new();
        // Iterate over banks 0..128, programs 0..128
        // FluidLite doesn't have a direct "list presets" API,
        // but we can query via sfont iteration.
        // A simpler approach: iterate bank/program combos that GM defines.
        // For GeneralUser_GS, bank 0 has 128 programs.
        if self.font_id.is_some() {
            for program in 0..128 {
                // Use bank 0 (GM)
                let name = format!("Bank 0, Program {program}");
                presets.push(PresetInfo {
                    id: program,
                    name,
                });
            }
        }
        presets
    }

    fn load_preset(&mut self, id: i32) -> Result<(), Box<dyn std::error::Error>> {
        // Load preset on channel 0, bank 0
        self.synth
            .program_change(0, id as u32)
            .map_err(|e| format!("failed to change program: {e}"))?;
        Ok(())
    }
}
