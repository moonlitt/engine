//! Voice — single note playback with Sinc 72 interpolation.
//!
//! A voice reads through a sample at a calculated playback rate,
//! using Sinc 72 interpolation for highest quality resampling.
//! Playback rate = (sample_rate / output_rate) × 2^((note - root_key) / 12)

use crate::sample::{SampleInfo, SamplePool};
use moonlitt_resampler::{Quality, SincInterpolator};

pub struct Voice {
    interp: SincInterpolator,
    sample: Option<SampleInfo>,
    /// Fractional position in sample (in sample's native sample indices).
    position: f64,
    /// Playback speed: samples consumed per output sample.
    speed: f64,
    output_rate: u32,
    amplitude: f32,
    active: bool,
}

impl Voice {
    pub fn new(pool: &SamplePool, output_rate: u32) -> Self {
        let _ = pool;
        Self::new_standalone(output_rate)
    }

    /// Create a voice without a pool reference (for VoicePool use).
    pub fn new_standalone(output_rate: u32) -> Self {
        Self {
            interp: SincInterpolator::new(Quality::Sinc72),
            sample: None,
            position: 0.0,
            speed: 1.0,
            output_rate,
            amplitude: 1.0,
            active: false,
        }
    }

    pub fn interpolation_quality(&self) -> Quality {
        self.interp.quality()
    }

    /// Activate voice with a sample and note parameters.
    /// Calculates playback speed from pitch difference.
    pub fn note_on(&mut self, sample: SampleInfo, note: u8, velocity: u8) {
        let semitone_diff = note as f64 - sample.root_key as f64
            + sample.pitch_correction as f64 / 100.0;
        let pitch_ratio = 2.0f64.powf(semitone_diff / 12.0);
        let rate_ratio = sample.sample_rate as f64 / self.output_rate as f64;

        self.speed = rate_ratio * pitch_ratio;
        self.amplitude = velocity as f32 / 127.0;
        self.position = 0.0;
        self.sample = Some(sample);
        self.active = true;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Begin release (for Sprint 4, immediately stops; Sprint 5 will use envelope).
    pub fn note_off(&mut self) {
        self.active = false;
    }

    /// Immediately silence (for all-notes-off).
    pub fn silence(&mut self) {
        self.active = false;
        self.sample = None;
        self.position = 0.0;
    }

    /// Render mono audio into the output buffer.
    pub fn render(&mut self, output: &mut [f32]) {
        let sample = match &self.sample {
            Some(s) if self.active => s,
            _ => {
                output.fill(0.0);
                return;
            }
        };

        let data = &sample.data;
        let data_len = data.len();

        for out in output.iter_mut() {
            let int_pos = self.position as usize;

            if int_pos >= data_len.saturating_sub(1) {
                *out = 0.0;
                self.active = false;
                continue;
            }

            let frac = (self.position - int_pos as f64) as f32;
            *out = self.amplitude * self.interp.interpolate_safe(data, int_pos, frac);
            self.position += self.speed;
        }
    }
}
