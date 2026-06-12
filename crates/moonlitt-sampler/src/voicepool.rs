//! VoicePool — manages multiple voices for polyphonic playback.
//!
//! Fixed-size pool with voice stealing (oldest voice) when full.
//! Each voice independently renders with Sinc72 interpolation.
//!
//! Per-channel MIDI expressiveness lives here: CC7 (volume), CC11
//! (expression), CC64 (sustain pedal) and pitch bend are tracked for
//! all 16 channels and applied to that channel's voices.

use crate::sample::SamplePool;
use crate::voice::Voice;

/// Pitch-bend range in semitones at full deflection (GM default).
const BEND_RANGE_SEMITONES: f64 = 2.0;

/// Per-channel controller state.
///
/// CC7/CC11 default to full scale (127) so an untouched channel renders
/// at unity — the GM "reset to 100" convention is deliberately not
/// applied to keep default loudness identical to the pre-CC behaviour.
#[derive(Clone, Copy)]
struct ChannelState {
    cc_volume: u8,
    cc_expression: u8,
    sustain: bool,
    bend: i16,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            cc_volume: 127,
            cc_expression: 127,
            sustain: false,
            bend: 0,
        }
    }
}

impl ChannelState {
    /// Combined CC7 × CC11 gain, square-law (GM convention).
    fn gain(&self) -> f32 {
        let v = self.cc_volume as f32 / 127.0;
        let e = self.cc_expression as f32 / 127.0;
        (v * v) * (e * e)
    }

    /// Playback-speed multiplier for the current bend
    /// (±`BEND_RANGE_SEMITONES` at full deflection).
    fn bend_ratio(&self) -> f64 {
        let normalized = self.bend as f64 / 8192.0; // -1.0..=~1.0
        2.0f64.powf(normalized * BEND_RANGE_SEMITONES / 12.0)
    }
}

/// A slot in the voice pool.
struct VoiceSlot {
    voice: Voice,
    /// Which note this voice is playing (for note-off matching).
    note: u8,
    /// Which channel.
    channel: u8,
    /// Note-off arrived while the sustain pedal was down — release when
    /// the pedal comes up.
    sustained: bool,
    /// Age counter (increments each note-on, oldest = lowest).
    age: u64,
    /// Whether this slot is in use.
    active: bool,
}

/// Fixed-size voice pool with polyphony and voice stealing.
pub struct VoicePool {
    slots: Vec<VoiceSlot>,
    /// Global age counter — incremented on each note-on.
    age_counter: u64,
    channels: [ChannelState; 16],
    /// Scratch mono buffer reused across renders (grows once to the
    /// host's block size, then never reallocates on the audio thread).
    scratch: Vec<f32>,
}

impl VoicePool {
    /// Create a pool with `max_voices` slots.
    pub fn new(max_voices: usize, sample_rate: u32) -> Self {
        let slots = (0..max_voices)
            .map(|_| VoiceSlot {
                voice: Voice::new_standalone(sample_rate),
                note: 0,
                channel: 0,
                sustained: false,
                age: 0,
                active: false,
            })
            .collect();

        Self {
            slots,
            age_counter: 0,
            channels: [ChannelState::default(); 16],
            scratch: Vec::new(),
        }
    }

    /// Start a note on `channel` using the pool's sample for
    /// (bank, program, note, velocity).
    pub fn note_on(
        &mut self,
        pool: &SamplePool,
        channel: u8,
        bank: u16,
        program: u8,
        note: u8,
        velocity: u8,
    ) {
        // Find a sample for this note
        let sample = match pool.find_sample(bank, program, note, velocity) {
            Some(s) => s,
            None => return, // no sample found for this note
        };

        self.age_counter += 1;

        // Find a free slot, or steal the oldest
        let slot_idx = self
            .find_free_slot()
            .unwrap_or_else(|| self.find_oldest_slot());

        let channel = channel.min(15);
        let bend = self.channels[channel as usize].bend_ratio();

        let slot = &mut self.slots[slot_idx];
        slot.voice.note_on(sample, note, velocity);
        slot.voice.set_bend_ratio(bend);
        slot.note = note;
        slot.channel = channel;
        slot.sustained = false;
        slot.age = self.age_counter;
        slot.active = true;
    }

    /// Release a note on `channel` (starts envelope release on the
    /// matching voice, or defers it while the sustain pedal is down).
    pub fn note_off(&mut self, channel: u8, note: u8) {
        let channel = channel.min(15);
        let sustain = self.channels[channel as usize].sustain;
        // Newest first, so overlapping re-strikes release in LIFO order.
        if let Some(slot) = self
            .slots
            .iter_mut()
            .filter(|s| s.active && s.channel == channel && s.note == note && !s.sustained)
            .max_by_key(|s| s.age)
        {
            if sustain {
                slot.sustained = true;
            } else {
                slot.voice.note_off();
            }
        }
    }

    /// Handle a control change on `channel`. Supported: CC7 (volume),
    /// CC11 (expression), CC64 (sustain pedal). Others are ignored.
    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        let ch = channel.min(15) as usize;
        match cc {
            7 => self.channels[ch].cc_volume = value.min(127),
            11 => self.channels[ch].cc_expression = value.min(127),
            64 => {
                let down = value >= 64;
                self.channels[ch].sustain = down;
                if !down {
                    // Pedal up: release every note held by the pedal.
                    for slot in self
                        .slots
                        .iter_mut()
                        .filter(|s| s.active && s.channel == ch as u8 && s.sustained)
                    {
                        slot.sustained = false;
                        slot.voice.note_off();
                    }
                }
            }
            _ => {}
        }
    }

    /// Set pitch bend for `channel` (-8192..=8191, 0 = centre). Applies
    /// to already-sounding voices immediately.
    pub fn pitch_bend(&mut self, channel: u8, value: i16) {
        let ch = channel.min(15) as usize;
        self.channels[ch].bend = value.clamp(-8192, 8191);
        let ratio = self.channels[ch].bend_ratio();
        for slot in self
            .slots
            .iter_mut()
            .filter(|s| s.active && s.channel == ch as u8)
        {
            slot.voice.set_bend_ratio(ratio);
        }
    }

    /// Immediately silence all voices.
    pub fn all_notes_off(&mut self) {
        for slot in &mut self.slots {
            slot.active = false;
            slot.sustained = false;
            slot.voice.silence();
        }
    }

    /// Number of currently active voices.
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.active).count()
    }

    /// Number of active voices on `channel` (diagnostics/tests).
    pub fn voices_on_channel(&self, channel: u8) -> usize {
        self.slots
            .iter()
            .filter(|s| s.active && s.channel == channel)
            .count()
    }

    /// Number of voices on `channel` currently fading out after release
    /// (diagnostics/tests).
    pub fn releasing_on_channel(&self, channel: u8) -> usize {
        self.slots
            .iter()
            .filter(|s| s.active && s.channel == channel && s.voice.is_releasing())
            .count()
    }

    /// Render all active voices into stereo buffers (summed).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);

        let n = left.len();
        // Grows on the first render (or a block-size increase) only;
        // steady-state renders never allocate.
        if self.scratch.len() < n {
            self.scratch.resize(n, 0.0);
        }

        for slot in &mut self.slots {
            if !slot.active {
                continue;
            }

            self.scratch[..n].fill(0.0);
            slot.voice.render(&mut self.scratch[..n]);

            // Check if voice finished (sample ended)
            if !slot.voice.is_active() {
                slot.active = false;
                continue;
            }

            let gain = self.channels[slot.channel as usize].gain();

            // Sum into stereo (center pan for now)
            for i in 0..n {
                left[i] += self.scratch[i] * gain;
                right[i] += self.scratch[i] * gain;
            }
        }
    }

    fn find_free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.active)
    }

    fn find_oldest_slot(&self) -> usize {
        self.slots
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| s.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bend_ratio_math() {
        let mut ch = ChannelState::default();
        assert!((ch.bend_ratio() - 1.0).abs() < 1e-12, "centre = unity");

        ch.bend = 8191; // ≈ +2 semitones
        let up = ch.bend_ratio();
        assert!(
            (up - 2.0f64.powf(2.0 / 12.0)).abs() < 1e-3,
            "full up ≈ +2 semitones (got {up})"
        );

        ch.bend = -8192; // exactly -2 semitones
        let down = ch.bend_ratio();
        assert!(
            (down - 2.0f64.powf(-2.0 / 12.0)).abs() < 1e-12,
            "full down = -2 semitones (got {down})"
        );
    }

    #[test]
    fn gain_is_square_law() {
        let mut ch = ChannelState::default();
        assert!((ch.gain() - 1.0).abs() < 1e-6, "defaults are unity");
        ch.cc_volume = 64;
        let expected = (64.0f32 / 127.0).powi(2);
        assert!((ch.gain() - expected).abs() < 1e-6);
    }
}
