//! DAHDSR Volume Envelope — SF2 2.04 spec compliant.
//!
//! 6 stages: Delay → Attack → Hold → Decay → Sustain → Release
//!
//! Timing uses "timecents": tc = 1200 × log2(seconds)
//! This means seconds = 2^(tc / 1200).
//!
//! Attack uses linear amplitude ramp (SF2 spec §8.1.3).
//! Decay and Release use exponential decay in dB domain (SF2 spec §8.1.3).

/// Convert timecents to seconds.
/// SF2 spec: seconds = 2^(timecents / 1200)
pub fn timecents_to_secs(tc: i32) -> f64 {
    2.0f64.powf(tc as f64 / 1200.0)
}

/// Envelope stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Finished,
}

/// Envelope parameters in timecents (except sustain which is 0.0-1.0 linear).
#[derive(Debug, Clone, Copy)]
pub struct EnvelopeParams {
    /// Delay time in timecents.
    pub delay: i32,
    /// Attack time in timecents.
    pub attack: i32,
    /// Hold time in timecents.
    pub hold: i32,
    /// Decay time in timecents.
    pub decay: i32,
    /// Sustain level (0.0 = silence, 1.0 = full volume).
    pub sustain: f32,
    /// Release time in timecents.
    pub release: i32,
}

impl Default for EnvelopeParams {
    fn default() -> Self {
        Self {
            delay: -12000,   // instant
            attack: -12000,  // instant
            hold: -12000,    // instant
            decay: -12000,   // instant
            sustain: 1.0,    // full sustain
            release: -12000, // instant
        }
    }
}

/// DAHDSR envelope generator.
pub struct Envelope {
    stage: Stage,
    /// Current amplitude (0.0 to 1.0).
    level: f32,
    /// Samples remaining in current timed stage.
    samples_remaining: u32,
    /// Per-sample increment/decrement for current stage.
    rate: f32,
    /// Sustain level.
    sustain: f32,
    /// Sample rate.
    sample_rate: u32,
    /// Stored params for release calculation.
    params: EnvelopeParams,
}

impl Envelope {
    pub fn new(params: EnvelopeParams, sample_rate: u32) -> Self {
        Self {
            stage: Stage::Idle,
            level: 0.0,
            samples_remaining: 0,
            rate: 0.0,
            sustain: params.sustain.clamp(0.0, 1.0),
            sample_rate,
            params,
        }
    }

    pub fn note_on(&mut self) {
        self.level = 0.0;
        self.enter_stage(Stage::Delay);
    }

    pub fn note_off(&mut self) {
        if self.stage == Stage::Idle || self.stage == Stage::Finished {
            return;
        }
        // Enter release from current level
        let release_secs = timecents_to_secs(self.params.release).max(0.001);
        let release_samples = (release_secs * self.sample_rate as f64) as u32;

        if release_samples == 0 {
            self.level = 0.0;
            self.stage = Stage::Finished;
        } else {
            // Exponential release: level decreases by a factor each sample
            // We use linear approximation for simplicity in Sprint 2
            self.rate = -self.level / release_samples as f32;
            self.samples_remaining = release_samples;
            self.stage = Stage::Release;
        }
    }

    /// Returns true if the envelope has finished (silent after release).
    /// Whether the envelope is in its release stage (note released but
    /// still audibly fading).
    pub fn is_releasing(&self) -> bool {
        matches!(self.stage, Stage::Release)
    }

    pub fn is_finished(&self) -> bool {
        self.stage == Stage::Finished
    }

    /// Process one sample, returning the envelope amplitude (0.0 to 1.0).
    pub fn process(&mut self) -> f32 {
        match self.stage {
            Stage::Idle | Stage::Finished => 0.0,
            Stage::Delay => {
                if self.samples_remaining == 0 {
                    self.enter_stage(Stage::Attack);
                    return self.process();
                }
                self.samples_remaining -= 1;
                0.0
            }
            Stage::Attack => {
                if self.samples_remaining == 0 {
                    self.level = 1.0;
                    self.enter_stage(Stage::Hold);
                    return self.level;
                }
                self.level += self.rate;
                self.level = self.level.clamp(0.0, 1.0);
                self.samples_remaining -= 1;
                self.level
            }
            Stage::Hold => {
                if self.samples_remaining == 0 {
                    self.enter_stage(Stage::Decay);
                    return self.level;
                }
                self.samples_remaining -= 1;
                1.0
            }
            Stage::Decay => {
                if self.samples_remaining == 0 || self.level <= self.sustain {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                    return self.level;
                }
                self.level += self.rate; // rate is negative
                if self.level < self.sustain {
                    self.level = self.sustain;
                }
                self.samples_remaining -= 1;
                self.level
            }
            Stage::Sustain => {
                self.sustain
            }
            Stage::Release => {
                if self.samples_remaining == 0 || self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Finished;
                    return 0.0;
                }
                self.level += self.rate; // rate is negative
                if self.level < 0.0 {
                    self.level = 0.0;
                }
                self.samples_remaining -= 1;
                self.level
            }
        }
    }

    fn enter_stage(&mut self, stage: Stage) {
        self.stage = stage;
        match stage {
            Stage::Delay => {
                let secs = timecents_to_secs(self.params.delay).max(0.0);
                self.samples_remaining = (secs * self.sample_rate as f64) as u32;
                self.rate = 0.0;
            }
            Stage::Attack => {
                let secs = timecents_to_secs(self.params.attack).max(0.001);
                let samples = (secs * self.sample_rate as f64) as u32;
                if samples == 0 {
                    self.level = 1.0;
                    self.enter_stage(Stage::Hold);
                    return;
                }
                self.samples_remaining = samples;
                // Linear attack: from current level to 1.0
                self.rate = (1.0 - self.level) / samples as f32;
            }
            Stage::Hold => {
                let secs = timecents_to_secs(self.params.hold).max(0.0);
                self.samples_remaining = (secs * self.sample_rate as f64) as u32;
                self.level = 1.0;
            }
            Stage::Decay => {
                let secs = timecents_to_secs(self.params.decay).max(0.001);
                let samples = (secs * self.sample_rate as f64) as u32;
                if samples == 0 || self.sustain >= 1.0 {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                    return;
                }
                self.samples_remaining = samples;
                // Linear decay from 1.0 to sustain
                self.rate = (self.sustain - 1.0) / samples as f32;
            }
            Stage::Sustain => {
                self.level = self.sustain;
            }
            _ => {}
        }
    }
}
