//! SF2 backend — pure Rust via OxiSynth.
//!
//! Replaces FluidLite with a pure Rust SoundFont synthesizer.
//! Uses Sinc72 interpolation (72-point windowed sinc, Kaiser beta=9.5).

use crate::backend::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo, PresetInfo};
use oxisynth::{InterpolationMethod, MidiEvent, SoundFont, Synth, SynthDescriptor};

// SF2 parameter IDs (same as FluidLite backend for API compatibility)
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

pub struct OxiSynthBackend {
    synth: Synth,
    sample_rate: u32,
    volume: f32,
    font_id: Option<oxisynth::SoundFontId>,
    reverb_on: bool,
    chorus_on: bool,
}

impl OxiSynthBackend {
    pub fn new(sample_rate: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let synth = Synth::new(SynthDescriptor {
            sample_rate: sample_rate as f32,
            gain: 1.0,
            polyphony: 256,
            midi_channels: 16,
            reverb_active: true,
            chorus_active: true,
            interpolation: InterpolationMethod::Sinc72,
            ..Default::default()
        })
        .map_err(|e| format!("failed to create OxiSynth: {e}"))?;

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

impl AudioBackend for OxiSynthBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "OxiSynth",
            backend_type: BackendType::Sampler,
            extensions: &["sf2"],
        }
    }

    fn load(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.unload();

        let mut file = std::fs::File::open(path)
            .map_err(|e| format!("failed to open sf2: {e}"))?;
        let font = SoundFont::load(&mut file)
            .map_err(|e| format!("failed to load sf2: {e}"))?;

        let id = self.synth.add_font(font, true);
        self.font_id = Some(id);
        self.synth.set_gain(self.volume);

        Ok(())
    }

    fn unload(&mut self) {
        if let Some(id) = self.font_id.take() {
            self.synth.remove_font(id, true);
        }
    }

    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let _ = self.synth.send_event(MidiEvent::NoteOn {
            channel,
            key: note,
            vel: velocity,
        });
    }

    fn note_off(&mut self, channel: u8, note: u8) {
        let _ = self.synth.send_event(MidiEvent::NoteOff {
            channel,
            key: note,
        });
    }

    fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        let _ = self.synth.send_event(MidiEvent::ControlChange {
            channel,
            ctrl: cc,
            value,
        });
    }

    fn pitch_bend(&mut self, channel: u8, value: i16) {
        let unsigned = (value as i32 + 8192).clamp(0, 16383) as u16;
        let _ = self.synth.send_event(MidiEvent::PitchBend {
            channel,
            value: unsigned,
        });
    }

    fn program_change(&mut self, channel: u8, program: u8) {
        let _ = self.synth.send_event(MidiEvent::ProgramChange {
            channel,
            program_id: program,
        });
    }

    fn all_notes_off(&mut self) {
        let _ = self.synth.send_event(MidiEvent::SystemReset);
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.synth.write_f32(
            left.len(),
            left, 0, 1,
            right, 0, 1,
        );
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.synth.set_gain(volume);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    // --- Parameters ---

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
            flags: if p.6 > 0 { ParamFlags::STEPPED } else { ParamFlags::empty() },
        })
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            PARAM_REVERB_ON => Some(if self.reverb_on { 1.0 } else { 0.0 }),
            PARAM_REVERB_ROOMSIZE => Some(self.synth.reverb_params().roomsize as f64),
            PARAM_REVERB_DAMP => Some(self.synth.reverb_params().damp as f64),
            PARAM_REVERB_WIDTH => Some(self.synth.reverb_params().width as f64),
            PARAM_REVERB_LEVEL => Some(self.synth.reverb_params().level as f64),
            PARAM_CHORUS_ON => Some(if self.chorus_on { 1.0 } else { 0.0 }),
            PARAM_CHORUS_VOICES => Some(self.synth.chorus_params().nr as f64),
            PARAM_CHORUS_LEVEL => Some(self.synth.chorus_params().level as f64),
            PARAM_CHORUS_SPEED => Some(self.synth.chorus_params().speed as f64),
            PARAM_CHORUS_DEPTH => Some(self.synth.chorus_params().depth as f64),
            PARAM_GAIN => Some(self.volume as f64),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PARAM_REVERB_ON => {
                self.reverb_on = value > 0.5;
                // OxiSynth doesn't have set_reverb_on — set level to 0 to disable
                if !self.reverb_on {
                    let mut p = self.synth.reverb_params();
                    p.level = 0.0f32;
                    self.synth.set_reverb_params(&p);
                }
            }
            PARAM_REVERB_ROOMSIZE | PARAM_REVERB_DAMP | PARAM_REVERB_WIDTH | PARAM_REVERB_LEVEL => {
                let mut p = self.synth.reverb_params();
                match id {
                    PARAM_REVERB_ROOMSIZE => p.roomsize = value as f32,
                    PARAM_REVERB_DAMP => p.damp = value as f32,
                    PARAM_REVERB_WIDTH => p.width = value as f32,
                    PARAM_REVERB_LEVEL => p.level = value as f32,
                    _ => {}
                }
                self.synth.set_reverb_params(&p);
            }
            PARAM_CHORUS_ON => {
                self.chorus_on = value > 0.5;
                if !self.chorus_on {
                    let mut p = self.synth.chorus_params();
                    p.level = 0.0f32;
                    self.synth.set_chorus_params(&p);
                }
            }
            PARAM_CHORUS_VOICES | PARAM_CHORUS_LEVEL | PARAM_CHORUS_SPEED | PARAM_CHORUS_DEPTH => {
                let mut p = self.synth.chorus_params();
                match id {
                    PARAM_CHORUS_VOICES => p.nr = value as u32,
                    PARAM_CHORUS_LEVEL => p.level = value as f32,
                    PARAM_CHORUS_SPEED => p.speed = value as f32,
                    PARAM_CHORUS_DEPTH => p.depth = value as f32,
                    _ => {}
                }
                self.synth.set_chorus_params(&p);
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
            _ => Some(format!("{:.2}", value)),
        }
    }

    // --- Presets ---

    fn presets(&self) -> Vec<PresetInfo> {
        if self.font_id.is_some() {
            (0..128)
                .map(|i| PresetInfo {
                    id: i,
                    name: format!("Bank 0, Program {i}"),
                })
                .collect()
        } else {
            vec![]
        }
    }

    fn load_preset(&mut self, id: i32) -> Result<(), Box<dyn std::error::Error>> {
        self.synth
            .send_event(MidiEvent::ProgramChange {
                channel: 0,
                program_id: id as u8,
            })
            .map_err(|e| format!("program change failed: {e}"))?;
        Ok(())
    }
}
