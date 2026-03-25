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
    tempo: AtomicU64, // f64 bits stored as u64
    looping: AtomicBool,
}

impl Transport {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(TransportState::Stopped as u8),
            tempo: AtomicU64::new(120.0f64.to_bits()),
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

    pub fn tempo(&self) -> f64 {
        f64::from_bits(self.tempo.load(Ordering::Relaxed))
    }

    pub fn set_tempo(&self, bpm: f64) {
        self.tempo.store(bpm.to_bits(), Ordering::Relaxed);
    }

    pub fn looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }

    pub fn set_loop(&self, v: bool) {
        self.looping.store(v, Ordering::Relaxed);
    }
}
