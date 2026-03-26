//! SF2 backend — wraps fluidlite behind AudioBackend.

use crate::backend::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo, PresetInfo};
use fluidlite::{IsSettings, Settings, Synth};

// SF2 parameter IDs
const PARAM_REVERB_ON: u32 = 0;
const PARAM_REVERB_ROOMSIZE: u32 = 1;
const PARAM_REVERB_DAMP: u32 = 2;
const PARAM_REVERB_WIDTH: u32 = 3;
const PARAM_REVERB_LEVEL: u32 = 4;
const PARAM_CHORUS_ON: u32 = 10;
const PARAM_CHORUS_VOICES: u32 = 11;
const PARAM_CHORUS_LEVEL: u32 = 12;
const PARAM_CHORUS_SPEED: u32 = 13;
const PARAM_CHORUS_DEPTH: u32 = 14;
const PARAM_GAIN: u32 = 20;

const SF2_PARAMS: &[(u32, &str, &str, f64, f64, f64, u32)] = &[
    // (id, name, group, min, max, default, step_count)
    (PARAM_REVERB_ON, "Reverb On", "Reverb", 0.0, 1.0, 1.0, 1),
    (PARAM_REVERB_ROOMSIZE, "Room Size", "Reverb", 0.0, 1.2, 0.2, 0),
    (PARAM_REVERB_DAMP, "Damping", "Reverb", 0.0, 1.0, 0.0, 0),
    (PARAM_REVERB_WIDTH, "Width", "Reverb", 0.0, 100.0, 0.5, 0),
    (PARAM_REVERB_LEVEL, "Level", "Reverb", 0.0, 1.0, 0.9, 0),
    (PARAM_CHORUS_ON, "Chorus On", "Chorus", 0.0, 1.0, 1.0, 1),
    (PARAM_CHORUS_VOICES, "Voices", "Chorus", 0.0, 99.0, 3.0, 99),
    (PARAM_CHORUS_LEVEL, "Level", "Chorus", 0.0, 10.0, 2.0, 0),
    (PARAM_CHORUS_SPEED, "Speed", "Chorus", 0.1, 5.0, 0.3, 0),
    (PARAM_CHORUS_DEPTH, "Depth", "Chorus", 0.0, 256.0, 8.0, 0),
    (PARAM_GAIN, "Gain", "Master", 0.0, 5.0, 1.0, 0),
];

pub struct Sf2Backend {
    synth: Synth,
    sample_rate: u32,
    volume: f32,
    font_id: Option<u32>,
    reverb_on: bool,
    chorus_on: bool,
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
            reverb_on: true,
            chorus_on: true,
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

    fn param_count(&self) -> u32 {
        SF2_PARAMS.len() as u32
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        let p = SF2_PARAMS.get(index as usize)?;
        Some(ParamInfo {
            id: p.0,
            name: p.1.to_string(),
            group: p.2.to_string(),
            min: p.3,
            max: p.4,
            default: p.5,
            step_count: p.6,
            flags: if p.6 > 0 {
                ParamFlags::STEPPED
            } else {
                ParamFlags::empty()
            },
        })
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            PARAM_REVERB_ON => Some(if self.reverb_on { 1.0 } else { 0.0 }),
            PARAM_REVERB_ROOMSIZE => Some(self.synth.get_reverb_roomsize()),
            PARAM_REVERB_DAMP => Some(self.synth.get_reverb_damp()),
            PARAM_REVERB_WIDTH => Some(self.synth.get_reverb_width()),
            PARAM_REVERB_LEVEL => Some(self.synth.get_reverb_level()),
            PARAM_CHORUS_ON => Some(if self.chorus_on { 1.0 } else { 0.0 }),
            PARAM_CHORUS_VOICES => Some(self.synth.get_chorus_nr() as f64),
            PARAM_CHORUS_LEVEL => Some(self.synth.get_chorus_level()),
            PARAM_CHORUS_SPEED => Some(self.synth.get_chorus_speed()),
            PARAM_CHORUS_DEPTH => Some(self.synth.get_chorus_depth()),
            PARAM_GAIN => Some(self.volume as f64),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PARAM_REVERB_ON => {
                self.reverb_on = value > 0.5;
                self.synth.set_reverb_on(self.reverb_on);
            }
            PARAM_REVERB_ROOMSIZE | PARAM_REVERB_DAMP | PARAM_REVERB_WIDTH | PARAM_REVERB_LEVEL => {
                // Read current, update one, write all
                let mut rs = self.synth.get_reverb_roomsize();
                let mut damp = self.synth.get_reverb_damp();
                let mut width = self.synth.get_reverb_width();
                let mut level = self.synth.get_reverb_level();
                match id {
                    PARAM_REVERB_ROOMSIZE => rs = value,
                    PARAM_REVERB_DAMP => damp = value,
                    PARAM_REVERB_WIDTH => width = value,
                    PARAM_REVERB_LEVEL => level = value,
                    _ => {}
                }
                self.synth.set_reverb_params(rs, damp, width, level);
            }
            PARAM_CHORUS_ON => {
                self.chorus_on = value > 0.5;
                self.synth.set_chorus_on(self.chorus_on);
            }
            PARAM_CHORUS_VOICES | PARAM_CHORUS_LEVEL | PARAM_CHORUS_SPEED | PARAM_CHORUS_DEPTH => {
                let mut nr = self.synth.get_chorus_nr();
                let mut level = self.synth.get_chorus_level();
                let mut speed = self.synth.get_chorus_speed();
                let mut depth = self.synth.get_chorus_depth();
                match id {
                    PARAM_CHORUS_VOICES => nr = value as u32,
                    PARAM_CHORUS_LEVEL => level = value,
                    PARAM_CHORUS_SPEED => speed = value,
                    PARAM_CHORUS_DEPTH => depth = value,
                    _ => {}
                }
                self.synth.set_chorus_params(nr, level, speed, depth,
                    Default::default());
            }
            PARAM_GAIN => {
                self.volume = value as f32;
                self.synth.set_gain(self.volume);
            }
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            PARAM_REVERB_ON | PARAM_CHORUS_ON => {
                Some(if value > 0.5 { "On" } else { "Off" }.to_string())
            }
            PARAM_CHORUS_VOICES => Some(format!("{}", value as u32)),
            PARAM_GAIN => Some(format!("{:.1}", value)),
            _ => Some(format!("{:.2}", value)),
        }
    }

    fn load_preset(&mut self, id: i32) -> Result<(), Box<dyn std::error::Error>> {
        // Load preset on channel 0, bank 0
        self.synth
            .program_change(0, id as u32)
            .map_err(|e| format!("failed to change program: {e}"))?;
        Ok(())
    }
}
