//! VoicePool — manages multiple voices for polyphonic playback.
//!
//! Fixed-size pool with voice stealing (oldest voice) when full.
//! Each voice independently renders with Sinc72 interpolation.

use crate::sample::SamplePool;
use crate::voice::Voice;

/// A slot in the voice pool.
struct VoiceSlot {
    voice: Voice,
    /// Which note this voice is playing (for note-off matching).
    note: u8,
    /// Which channel.
    channel: u8,
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
}

impl VoicePool {
    /// Create a pool with `max_voices` slots.
    pub fn new(max_voices: usize, sample_rate: u32) -> Self {
        let slots = (0..max_voices)
            .map(|_| VoiceSlot {
                voice: Voice::new_standalone(sample_rate),
                note: 0,
                channel: 0,
                age: 0,
                active: false,
            })
            .collect();

        Self {
            slots,
            age_counter: 0,
        }
    }

    /// Start playing a note. Steals the oldest voice if pool is full.
    pub fn note_on(&mut self, pool: &SamplePool, bank: u16, program: u8, note: u8, velocity: u8) {
        // Find a sample for this note
        let sample = match pool.find_sample(bank, program, note, velocity) {
            Some(s) => s,
            None => return, // no sample found for this note
        };

        self.age_counter += 1;

        // Find a free slot, or steal the oldest
        let slot_idx = self.find_free_slot()
            .unwrap_or_else(|| self.find_oldest_slot());

        let slot = &mut self.slots[slot_idx];
        slot.voice.note_on(sample, note, velocity);
        slot.note = note;
        slot.channel = 0; // TODO: proper channel tracking
        slot.age = self.age_counter;
        slot.active = true;
    }

    /// Release a note (starts envelope release on matching voice).
    pub fn note_off(&mut self, _channel: u8, note: u8) {
        // Find the voice playing this note (newest first for overlapping notes)
        if let Some(slot) = self.slots.iter_mut()
            .filter(|s| s.active && s.note == note)
            .max_by_key(|s| s.age)
        {
            slot.voice.note_off();
        }
    }

    /// Immediately silence all voices.
    pub fn all_notes_off(&mut self) {
        for slot in &mut self.slots {
            slot.active = false;
            slot.voice.silence();
        }
    }

    /// Number of currently active voices.
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.active).count()
    }

    /// Render all active voices into stereo buffers (summed).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);

        let n = left.len();
        let mut mono_buf = vec![0.0f32; n]; // TODO: pre-allocate

        for slot in &mut self.slots {
            if !slot.active {
                continue;
            }

            mono_buf.fill(0.0);
            slot.voice.render(&mut mono_buf);

            // Check if voice finished (sample ended)
            if !slot.voice.is_active() {
                slot.active = false;
                continue;
            }

            // Sum into stereo (center pan for now)
            for i in 0..n {
                left[i] += mono_buf[i];
                right[i] += mono_buf[i];
            }
        }
    }

    fn find_free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.active)
    }

    fn find_oldest_slot(&self) -> usize {
        self.slots.iter()
            .enumerate()
            .min_by_key(|(_, s)| s.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}
