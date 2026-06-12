//! Null backend — renders silence, accepts all MIDI. Used for testing
//! and as a placeholder when no real backend is loaded.

use crate::{AudioBackend, BackendInfo, BackendType};

/// A backend that renders silence and does nothing. Useful for testing
/// and as a placeholder in mixer tracks/inserts/send buses.
pub struct NullBackend {
    sample_rate: u32,
}

impl NullBackend {
    pub fn new(sample_rate: u32) -> Self {
        Self { sample_rate }
    }
}

impl AudioBackend for NullBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Null",
            backend_type: BackendType::Sampler,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Err("NullBackend does not support loading".into())
    }

    fn unload(&mut self) {}

    fn note_on(&mut self, _channel: u8, _note: u8, _velocity: u8) {}
    fn note_off(&mut self, _channel: u8, _note: u8) {}
    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: i16) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn all_notes_off(&mut self) {}

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);
    }

    fn process_effect(
        &mut self,
        _in_l: &[f32],
        _in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        out_l.fill(0.0);
        out_r.fill(0.0);
    }

    fn set_volume(&mut self, _volume: f32) {}

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
