use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TransportState {
    Stopped = 0,
    Playing = 1,
    Paused = 2,
}

pub struct Transport {
    state: AtomicU8,
    /// Tempo override: 0 = use MIDI file's embedded tempo map, else f64 BPM bits.
    tempo_override: AtomicU64,
    looping: AtomicBool,
}

impl Default for Transport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(TransportState::Stopped as u8),
            tempo_override: AtomicU64::new(0), // 0 = no override
            looping: AtomicBool::new(false),
        }
    }

    pub fn state(&self) -> TransportState {
        match self.state.load(Ordering::Relaxed) {
            1 => TransportState::Playing,
            2 => TransportState::Paused,
            _ => TransportState::Stopped,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.state() == TransportState::Playing
    }

    pub fn play(&self) {
        self.state
            .store(TransportState::Playing as u8, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.state
            .store(TransportState::Paused as u8, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.state
            .store(TransportState::Stopped as u8, Ordering::Relaxed);
    }

    /// Returns the tempo override, or `None` if using the MIDI file's embedded tempo.
    pub fn tempo(&self) -> Option<f64> {
        let bits = self.tempo_override.load(Ordering::Relaxed);
        if bits == 0 {
            None
        } else {
            Some(f64::from_bits(bits))
        }
    }

    /// Override the tempo (in BPM). This takes precedence over the MIDI file's tempo map.
    pub fn set_tempo(&self, bpm: f64) {
        self.tempo_override.store(bpm.to_bits(), Ordering::Relaxed);
    }

    /// Clear the tempo override — revert to the MIDI file's embedded tempo.
    pub fn clear_tempo(&self) {
        self.tempo_override.store(0, Ordering::Relaxed);
    }

    pub fn looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }

    pub fn set_loop(&self, v: bool) {
        self.looping.store(v, Ordering::Relaxed);
    }
}
