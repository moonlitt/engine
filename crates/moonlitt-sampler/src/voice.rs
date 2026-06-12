//! Voice — single note playback with Sinc 72 interpolation + DAHDSR envelope.
//!
//! Playback rate = (sample_rate / output_rate) × 2^((note - root_key) / 12)
//! Each output sample is: interpolated_sample × envelope × amplitude

use crate::envelope::{Envelope, EnvelopeParams};
use crate::sample::{SampleInfo, SamplePool};
use moonlitt_resampler::{Quality, SincInterpolator};

pub struct Voice {
    interp: SincInterpolator,
    sample: Option<SampleInfo>,
    envelope: Envelope,
    position: f64,
    speed: f64,
    /// Live pitch-bend multiplier on top of `speed` (1.0 = no bend).
    bend: f64,
    output_rate: u32,
    amplitude: f32,
    active: bool,
}

impl Voice {
    pub fn new(pool: &SamplePool, output_rate: u32) -> Self {
        let _ = pool;
        Self::new_standalone(output_rate)
    }

    pub fn new_standalone(output_rate: u32) -> Self {
        Self {
            interp: SincInterpolator::new(Quality::Sinc72),
            sample: None,
            envelope: Envelope::new(EnvelopeParams::default(), output_rate),
            position: 0.0,
            speed: 1.0,
            bend: 1.0,
            output_rate,
            amplitude: 1.0,
            active: false,
        }
    }

    pub fn interpolation_quality(&self) -> Quality {
        self.interp.quality()
    }

    /// Activate voice with a sample and note parameters.
    pub fn note_on(&mut self, sample: SampleInfo, note: u8, velocity: u8) {
        let semitone_diff =
            note as f64 - sample.root_key as f64 + sample.pitch_correction as f64 / 100.0;
        let pitch_ratio = 2.0f64.powf(semitone_diff / 12.0);
        let rate_ratio = sample.sample_rate as f64 / self.output_rate as f64;

        self.speed = rate_ratio * pitch_ratio;
        self.amplitude = velocity as f32 / 127.0;
        self.position = 0.0;
        self.sample = Some(sample);
        self.active = true;

        // Start envelope: short attack to avoid click, natural decay
        self.envelope = Envelope::new(
            EnvelopeParams {
                delay: -12000,  // instant
                attack: -4800,  // ~60ms attack (smooth onset, no click)
                hold: -12000,   // instant
                decay: 1200,    // 2s decay
                sustain: 0.5,   // sustain at 50%
                release: -2400, // ~250ms release (smooth tail)
            },
            self.output_rate,
        );
        self.envelope.note_on();
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Whether the voice is fading out after note-off / pedal release.
    pub fn is_releasing(&self) -> bool {
        self.envelope.is_releasing()
    }

    /// Set the live pitch-bend multiplier (1.0 = centre). Applied on top
    /// of the note's base playback speed; takes effect mid-note.
    pub fn set_bend_ratio(&mut self, ratio: f64) {
        self.bend = ratio;
    }

    /// Begin envelope release (smooth fade out instead of abrupt cutoff).
    pub fn note_off(&mut self) {
        self.envelope.note_off();
    }

    /// Immediately silence.
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
            // Check envelope finished
            if self.envelope.is_finished() {
                *out = 0.0;
                self.active = false;
                continue;
            }

            let int_pos = self.position as usize;

            if int_pos >= data_len.saturating_sub(1) {
                *out = 0.0;
                self.active = false;
                continue;
            }

            let frac = (self.position - int_pos as f64) as f32;
            let raw = self.interp.interpolate_safe(data, int_pos, frac);
            let env = self.envelope.process();

            *out = raw * env * self.amplitude;
            self.position += self.speed * self.bend;
        }
    }
}
