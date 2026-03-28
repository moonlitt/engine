//! AudioBackend implementation for moonlitt-sampler.
//!
//! Wraps SamplePool + VoicePool behind the standard AudioBackend trait,
//! making moonlitt-sampler a drop-in replacement for OxiSynth.

use crate::sample::SamplePool;
use crate::voicepool::VoicePool;
use moonlitt_engine::backend::{AudioBackend, BackendInfo, BackendType, PresetInfo};

pub struct SamplerBackend {
    pool: Option<SamplePool>,
    voices: VoicePool,
    sample_rate: u32,
    volume: f32,
    /// Per-channel program (GM preset number).
    channel_programs: [u8; 16],
}

impl SamplerBackend {
    pub fn new(sample_rate: u32) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            pool: None,
            voices: VoicePool::new(256, sample_rate),
            sample_rate,
            volume: 1.0,
            channel_programs: [0; 16],
        })
    }
}

impl AudioBackend for SamplerBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "moonlitt-sampler",
            backend_type: BackendType::Sampler,
            extensions: &["sf2"],
        }
    }

    fn load(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.unload();
        let pool = SamplePool::from_file(path)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        self.pool = Some(pool);
        self.channel_programs = [0; 16];
        Ok(())
    }

    fn unload(&mut self) {
        self.voices.all_notes_off();
        self.pool = None;
    }

    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        if let Some(pool) = &self.pool {
            let ch = (channel as usize).min(15);
            let program = self.channel_programs[ch];
            self.voices.note_on(pool, 0, program, note, velocity);
        }
    }

    fn note_off(&mut self, channel: u8, note: u8) {
        self.voices.note_off(channel, note);
    }

    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {
        // TODO Sprint 6: CC handling (volume, expression, sustain)
    }

    fn pitch_bend(&mut self, _channel: u8, _value: i16) {
        // TODO Sprint 6: pitch bend
    }

    fn program_change(&mut self, channel: u8, program: u8) {
        let ch = (channel as usize).min(15);
        self.channel_programs[ch] = program;
    }

    fn all_notes_off(&mut self) {
        self.voices.all_notes_off();
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.voices.render(left, right);

        // Apply master volume
        if (self.volume - 1.0).abs() > 0.001 {
            for s in left.iter_mut() {
                *s *= self.volume;
            }
            for s in right.iter_mut() {
                *s *= self.volume;
            }
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn presets(&self) -> Vec<PresetInfo> {
        if self.pool.is_some() {
            (0..128)
                .map(|i| PresetInfo {
                    id: i,
                    name: format!("Program {i}"),
                })
                .collect()
        } else {
            vec![]
        }
    }

    fn load_preset(&mut self, id: i32) -> Result<(), Box<dyn std::error::Error>> {
        self.channel_programs[0] = id as u8;
        Ok(())
    }
}
