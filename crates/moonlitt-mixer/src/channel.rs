//! Channel-strip building blocks: PDC delay line, output routing, insert
//! effects, tracks, send buses, and the master bus.

use crate::meter::LevelMeter;
use moonlitt_core::AudioBackend;

/// Ring buffer delay line for Plugin Delay Compensation (PDC).
///
/// When tracks have different insert chain latencies, the mixer delays
/// faster tracks so all audio arrives at the master bus in phase.
pub(crate) struct DelayLine {
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    write_pos: usize,
    pub(crate) delay: usize,
}

impl DelayLine {
    pub(crate) fn new() -> Self {
        Self {
            buffer_left: Vec::new(),
            buffer_right: Vec::new(),
            write_pos: 0,
            delay: 0,
        }
    }

    pub(crate) fn set_delay(&mut self, delay: usize) {
        if delay == self.delay {
            return;
        }
        self.delay = delay;
        if delay == 0 {
            self.buffer_left.clear();
            self.buffer_right.clear();
        } else {
            self.buffer_left = vec![0.0; delay];
            self.buffer_right = vec![0.0; delay];
        }
        self.write_pos = 0;
    }

    /// Process audio through the delay line.
    /// No-op when delay is 0 (fast path).
    pub(crate) fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.delay == 0 {
            return;
        }
        for i in 0..left.len() {
            let delayed_l = self.buffer_left[self.write_pos];
            let delayed_r = self.buffer_right[self.write_pos];
            self.buffer_left[self.write_pos] = left[i];
            self.buffer_right[self.write_pos] = right[i];
            left[i] = delayed_l;
            right[i] = delayed_r;
            self.write_pos = (self.write_pos + 1) % self.delay;
        }
    }
}

/// Where a track routes its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTarget {
    /// Route to master bus (default).
    Master,
    /// Route to a group track (submix).
    Group(u32),
}

/// A single insert effect slot on a track.
/// Processed pre-fader in series: `Backend → Insert[0] → Insert[1] → … → Fader`.
pub struct InsertEffect {
    pub id: u32,
    pub backend: Box<dyn AudioBackend>,
    pub bypass: bool,
    /// Path of the loaded file (for session persistence).
    pub source_path: Option<String>,
    /// External sidechain source track ID. None = internal sidechain.
    pub sidechain_source: Option<u32>,
}

/// A single track: one audio backend + channel strip.
pub struct Track {
    pub id: u32,
    pub backend: Box<dyn AudioBackend>,
    /// Bitmask: which MIDI channels route to this track (bit N = channel N).
    pub channel_mask: u16,
    /// Path of the loaded file (for session persistence).
    pub source_path: Option<String>,
    pub volume: f32,
    /// Pre-insert gain trim in dB. Range: -24.0 to +24.0, default 0.0.
    pub trim_db: f32,
    /// -1.0 (full left) to 1.0 (full right), 0.0 = center.
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    /// Send levels: one per send bus.
    pub send_levels: Vec<f32>,
    /// Insert effect chain (pre-fader, processed in order).
    pub inserts: Vec<InsertEffect>,
    /// Output routing: Master (default) or Group(track_id) for submixing.
    pub output_target: OutputTarget,
    // Pre-allocated render buffers
    pub(crate) left: Vec<f32>,
    pub(crate) right: Vec<f32>,
    // Group input accumulators (used when this track is a submix target)
    pub(crate) group_in_left: Vec<f32>,
    pub(crate) group_in_right: Vec<f32>,
    // Scratch buffers for insert chain ping-pong processing
    pub(crate) scratch_left: Vec<f32>,
    pub(crate) scratch_right: Vec<f32>,
    // Temporary buffers for external sidechain signal
    pub(crate) sidechain_buf_l: Vec<f32>,
    pub(crate) sidechain_buf_r: Vec<f32>,
    /// PDC delay line — compensates for insert chain latency differences.
    pub(crate) delay_line: DelayLine,
    /// Level meter (peak + RMS), readable from main thread.
    pub meter: LevelMeter,
}

/// A send bus: accumulates audio from tracks, processes through an effect backend.
pub struct SendBus {
    pub id: u32,
    pub backend: Box<dyn AudioBackend>,
    pub level: f32, // return level to master
    /// Path of the loaded file (for session persistence).
    pub source_path: Option<String>,
    // Accumulation + output buffers
    pub(crate) acc_left: Vec<f32>,
    pub(crate) acc_right: Vec<f32>,
    pub(crate) out_left: Vec<f32>,
    pub(crate) out_right: Vec<f32>,
}

/// Master bus: final volume + limiter.
pub struct MasterBus {
    pub volume: f32,
    pub limiter_threshold: f32,
    pub(crate) left: Vec<f32>,
    pub(crate) right: Vec<f32>,
    /// Level meter (peak + RMS), readable from main thread.
    pub meter: LevelMeter,
}
