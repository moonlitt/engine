//! Mixer — audio processing component, NOT a runtime concern.
//!
//! The Mixer is a pure audio DSP graph: it combines multiple engine tracks
//! with send buses (effects) and a master bus (limiter). It lives in
//! moonlitt-runtime for convenience but is conceptually independent —
//! AudioThread owns and drives the Mixer, calling `render()` each audio
//! callback. The Mixer has no knowledge of threads, devices, or transport.
//!
//! If the crate dependency graph ever needs it, Mixer can be extracted to
//! its own `moonlitt-mixer` crate with zero API changes.
//!
//! All rendering happens in the audio thread. No locks, no allocations.

use moonlitt_engine::engine::Engine;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Thread-safe stereo level meter (peak + RMS).
/// Written by audio thread, read by main thread via atomic f32-as-u32.
#[derive(Clone)]
pub struct LevelMeter {
    peak_left: Arc<AtomicU32>,
    peak_right: Arc<AtomicU32>,
    rms_left: Arc<AtomicU32>,
    rms_right: Arc<AtomicU32>,
}

impl LevelMeter {
    fn new() -> Self {
        Self {
            peak_left: Arc::new(AtomicU32::new(0)),
            peak_right: Arc::new(AtomicU32::new(0)),
            rms_left: Arc::new(AtomicU32::new(0)),
            rms_right: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Update meter from a rendered buffer. Called on audio thread.
    fn update(&self, left: &[f32], right: &[f32]) {
        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;
        let mut sum_sq_l: f32 = 0.0;
        let mut sum_sq_r: f32 = 0.0;
        for i in 0..left.len() {
            let al = left[i].abs();
            let ar = right[i].abs();
            if al > peak_l { peak_l = al; }
            if ar > peak_r { peak_r = ar; }
            sum_sq_l += left[i] * left[i];
            sum_sq_r += right[i] * right[i];
        }
        let n = left.len().max(1) as f32;
        let rms_l = (sum_sq_l / n).sqrt();
        let rms_r = (sum_sq_r / n).sqrt();

        self.peak_left.store(peak_l.to_bits(), Ordering::Relaxed);
        self.peak_right.store(peak_r.to_bits(), Ordering::Relaxed);
        self.rms_left.store(rms_l.to_bits(), Ordering::Relaxed);
        self.rms_right.store(rms_r.to_bits(), Ordering::Relaxed);
    }

    /// Read peak level (L, R). Called from main thread.
    pub fn peak(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_left.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_right.load(Ordering::Relaxed)),
        )
    }

    /// Read RMS level (L, R). Called from main thread.
    pub fn rms(&self) -> (f32, f32) {
        (
            f32::from_bits(self.rms_left.load(Ordering::Relaxed)),
            f32::from_bits(self.rms_right.load(Ordering::Relaxed)),
        )
    }
}

/// Ring buffer delay line for Plugin Delay Compensation (PDC).
///
/// When tracks have different insert chain latencies, the mixer delays
/// faster tracks so all audio arrives at the master bus in phase.
struct DelayLine {
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    write_pos: usize,
    delay: usize,
}

impl DelayLine {
    fn new() -> Self {
        Self {
            buffer_left: Vec::new(),
            buffer_right: Vec::new(),
            write_pos: 0,
            delay: 0,
        }
    }

    fn set_delay(&mut self, delay: usize) {
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
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
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

/// A single insert effect slot on a track.
/// Processed pre-fader in series: Engine → Insert[0] → Insert[1] → ... → Fader.
pub struct InsertEffect {
    pub id: u32,
    pub engine: Engine,
    pub bypass: bool,
}

/// A single track: one engine + channel strip.
pub struct Track {
    pub id: u32,
    pub engine: Engine,
    /// Bitmask: which MIDI channels route to this track (bit N = channel N).
    pub channel_mask: u16,
    pub volume: f32,
    /// -1.0 (full left) to 1.0 (full right), 0.0 = center.
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    /// Send levels: one per send bus.
    pub send_levels: Vec<f32>,
    /// Insert effect chain (pre-fader, processed in order).
    pub inserts: Vec<InsertEffect>,
    // Pre-allocated render buffers
    left: Vec<f32>,
    right: Vec<f32>,
    // Scratch buffers for insert chain ping-pong processing
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
    /// PDC delay line — compensates for insert chain latency differences.
    delay_line: DelayLine,
    /// Level meter (peak + RMS), readable from main thread.
    pub meter: LevelMeter,
}

/// A send bus: accumulates audio from tracks, processes through an effect engine.
pub struct SendBus {
    pub id: u32,
    pub engine: Engine, // loaded with a VST3/CLAP effect plugin
    pub level: f32,     // return level to master
    // Accumulation + output buffers
    acc_left: Vec<f32>,
    acc_right: Vec<f32>,
    out_left: Vec<f32>,
    out_right: Vec<f32>,
}

/// Master bus: final volume + limiter.
pub struct MasterBus {
    pub volume: f32,
    pub limiter_threshold: f32,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Level meter (peak + RMS), readable from main thread.
    pub meter: LevelMeter,
}

/// The mixer: owns tracks, send buses, and master.
pub struct Mixer {
    tracks: Vec<Track>,
    send_buses: Vec<SendBus>,
    master: MasterBus,
    buffer_size: usize,
    sample_rate: u32,
    next_track_id: u32,
    next_bus_id: u32,
    next_insert_id: u32,
}

impl Mixer {
    /// Create a new mixer with the given sample rate and buffer size.
    ///
    /// `buffer_size` is `usize` since it's used for buffer allocation. Callers
    /// using `u32` (e.g., Runtime) cast via `as usize`. The inconsistency is
    /// cosmetic — both types are adequate for buffer sizes in practice.
    pub fn new(sample_rate: u32, buffer_size: usize) -> Self {
        Self {
            tracks: Vec::new(),
            send_buses: Vec::new(),
            master: MasterBus {
                volume: 1.0,
                limiter_threshold: 0.95,
                left: vec![0.0; buffer_size],
                right: vec![0.0; buffer_size],
                meter: LevelMeter::new(),
            },
            buffer_size,
            sample_rate,
            next_track_id: 0,
            next_bus_id: 0,
            next_insert_id: 0,
        }
    }

    // --- ID counter accessors (for Runtime pre-assignment) ---

    pub fn next_track_id(&self) -> u32 {
        self.next_track_id
    }

    pub fn next_bus_id(&self) -> u32 {
        self.next_bus_id
    }

    pub fn next_insert_id(&self) -> u32 {
        self.next_insert_id
    }

    /// Add a track with an engine and a channel mask. Returns the track ID.
    pub fn add_track(&mut self, engine: Engine, channel_mask: u16) -> u32 {
        let id = self.next_track_id;
        self.next_track_id += 1;
        self.add_track_inner(id, engine, channel_mask);
        id
    }

    /// Add a track with a pre-assigned ID (for Runtime command channel).
    pub fn add_track_with_id(&mut self, id: u32, engine: Engine, channel_mask: u16) {
        if id >= self.next_track_id {
            self.next_track_id = id + 1;
        }
        self.add_track_inner(id, engine, channel_mask);
    }

    fn add_track_inner(&mut self, id: u32, engine: Engine, channel_mask: u16) {
        self.tracks.push(Track {
            id,
            engine,
            channel_mask,
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            send_levels: vec![0.0; self.send_buses.len()],
            inserts: Vec::new(),
            left: vec![0.0; self.buffer_size],
            right: vec![0.0; self.buffer_size],
            scratch_left: vec![0.0; self.buffer_size],
            scratch_right: vec![0.0; self.buffer_size],
            delay_line: DelayLine::new(),
            meter: LevelMeter::new(),
        });
    }

    /// Remove a track by ID. Returns the engine if found.
    pub fn remove_track(&mut self, id: u32) -> Option<Engine> {
        let pos = self.tracks.iter().position(|t| t.id == id)?;
        let track = self.tracks.remove(pos);
        Some(track.engine)
    }

    /// Add a send bus with an effect engine. Returns the bus ID.
    pub fn add_send_bus(&mut self, engine: Engine) -> u32 {
        let id = self.next_bus_id;
        self.next_bus_id += 1;
        self.add_send_bus_inner(id, engine);
        id
    }

    /// Add a send bus with a pre-assigned ID (for Runtime command channel).
    pub fn add_send_bus_with_id(&mut self, id: u32, engine: Engine) {
        if id >= self.next_bus_id {
            self.next_bus_id = id + 1;
        }
        self.add_send_bus_inner(id, engine);
    }

    fn add_send_bus_inner(&mut self, id: u32, engine: Engine) {
        let bs = self.buffer_size;
        self.send_buses.push(SendBus {
            id,
            engine,
            level: 1.0,
            acc_left: vec![0.0; bs],
            acc_right: vec![0.0; bs],
            out_left: vec![0.0; bs],
            out_right: vec![0.0; bs],
        });
        // Extend all tracks' send_levels
        for track in &mut self.tracks {
            track.send_levels.push(0.0);
        }
    }

    // --- Accessors ---

    pub fn track(&self, id: u32) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: u32) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn send_buses(&self) -> &[SendBus] {
        &self.send_buses
    }

    pub fn send_bus_mut(&mut self, id: u32) -> Option<&mut SendBus> {
        self.send_buses.iter_mut().find(|b| b.id == id)
    }

    pub fn master(&self) -> &MasterBus {
        &self.master
    }

    pub fn master_mut(&mut self) -> &mut MasterBus {
        &mut self.master
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master.volume = volume;
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    // --- Insert effect management ---

    /// Add an insert effect to a track. Returns the insert ID, or None if track not found.
    pub fn add_insert(&mut self, track_id: u32, engine: Engine) -> Option<u32> {
        let track = self.tracks.iter_mut().find(|t| t.id == track_id)?;
        let id = self.next_insert_id;
        self.next_insert_id += 1;
        track.inserts.push(InsertEffect {
            id,
            engine,
            bypass: false,
        });
        self.recalculate_pdc();
        Some(id)
    }

    /// Add an insert with a pre-assigned ID (for Runtime command channel).
    pub fn add_insert_with_id(&mut self, track_id: u32, insert_id: u32, engine: Engine) {
        if insert_id >= self.next_insert_id {
            self.next_insert_id = insert_id + 1;
        }
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            track.inserts.push(InsertEffect {
                id: insert_id,
                engine,
                bypass: false,
            });
        }
        self.recalculate_pdc();
    }

    /// Remove an insert effect from a track. Returns the engine if found.
    pub fn remove_insert(&mut self, track_id: u32, insert_id: u32) -> Option<Engine> {
        let track = self.tracks.iter_mut().find(|t| t.id == track_id)?;
        let pos = track.inserts.iter().position(|i| i.id == insert_id)?;
        let insert = track.inserts.remove(pos);
        self.recalculate_pdc();
        Some(insert.engine)
    }

    /// Set bypass state for an insert effect.
    pub fn set_insert_bypass(&mut self, track_id: u32, insert_id: u32, bypass: bool) {
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            if let Some(insert) = track.inserts.iter_mut().find(|i| i.id == insert_id) {
                insert.bypass = bypass;
            }
        }
        self.recalculate_pdc();
    }

    // --- Plugin Delay Compensation ---

    /// Recalculate PDC delay lines for all tracks.
    ///
    /// Computes the total insert chain latency for each track, finds the
    /// maximum, and sets delay lines on faster tracks to compensate.
    /// Called automatically after insert add/remove/bypass changes.
    pub fn recalculate_pdc(&mut self) {
        // Compute per-track latency
        let latencies: Vec<u32> = self
            .tracks
            .iter()
            .map(|t| {
                t.inserts
                    .iter()
                    .filter(|i| !i.bypass)
                    .map(|i| i.engine.latency())
                    .sum()
            })
            .collect();

        let max_latency = latencies.iter().copied().max().unwrap_or(0) as usize;

        for (track, &latency) in self.tracks.iter_mut().zip(latencies.iter()) {
            let compensation = max_latency - latency as usize;
            track.delay_line.set_delay(compensation);
        }
    }

    /// Get the insert chain for a track (for inspection/parameter control).
    pub fn track_insert(&self, track_id: u32, insert_id: u32) -> Option<&InsertEffect> {
        let track = self.tracks.iter().find(|t| t.id == track_id)?;
        track.inserts.iter().find(|i| i.id == insert_id)
    }

    /// Get a mutable reference to an insert effect (for parameter control).
    pub fn track_insert_mut(&mut self, track_id: u32, insert_id: u32) -> Option<&mut InsertEffect> {
        let track = self.tracks.iter_mut().find(|t| t.id == track_id)?;
        track.inserts.iter_mut().find(|i| i.id == insert_id)
    }

    // --- Event routing ---

    /// Dispatch a MIDI event to the correct track(s) by channel.
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.engine.note_on(channel, note, velocity);
            }
        }
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.engine.note_off(channel, note);
            }
        }
    }

    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.engine.cc(channel, cc, value);
            }
        }
    }

    pub fn pitch_bend(&mut self, channel: u8, value: i16) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.engine.pitch_bend(channel, value);
            }
        }
    }

    pub fn program_change(&mut self, channel: u8, program: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.engine.program_change(channel, program);
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for track in &mut self.tracks {
            track.engine.all_notes_off();
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.master.volume = volume;
    }

    /// Broadcast a parameter change to all tracks.
    pub fn set_param(&mut self, id: u32, value: f64) {
        for track in &mut self.tracks {
            track.engine.set_param(id, value);
        }
    }

    /// Set a parameter on a specific track's engine.
    pub fn set_param_for_track(&mut self, track_id: u32, id: u32, value: f64) {
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            track.engine.set_param(id, value);
        }
    }

    /// Set a parameter on a specific insert effect.
    pub fn set_insert_param(&mut self, track_id: u32, insert_id: u32, id: u32, value: f64) {
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            if let Some(insert) = track.inserts.iter_mut().find(|i| i.id == insert_id) {
                insert.engine.set_param(id, value);
            }
        }
    }

    // --- Rendering ---

    /// Render one chunk of audio into an interleaved stereo output buffer.
    pub fn render(&mut self, output_left: &mut [f32], output_right: &mut [f32]) {
        let chunk = output_left.len().min(output_right.len()).min(self.buffer_size);

        // Clear master
        self.master.left[..chunk].fill(0.0);
        self.master.right[..chunk].fill(0.0);

        // Clear send bus accumulators
        for bus in &mut self.send_buses {
            bus.acc_left[..chunk].fill(0.0);
            bus.acc_right[..chunk].fill(0.0);
        }

        // Check if any track has solo enabled
        let any_solo = self.tracks.iter().any(|t| t.solo);

        // Render each track
        for track in &mut self.tracks {
            // Skip if muted or not soloed
            let audible = !track.mute && (!any_solo || track.solo);

            // Always render (to keep engine state consistent), but zero if inaudible
            track.left[..chunk].fill(0.0);
            track.right[..chunk].fill(0.0);
            track.engine.render(&mut track.left[..chunk], &mut track.right[..chunk]);

            // Process insert chain (pre-fader)
            if !track.inserts.is_empty() {
                process_insert_chain(
                    &mut track.inserts,
                    &mut track.left,
                    &mut track.right,
                    &mut track.scratch_left,
                    &mut track.scratch_right,
                    chunk,
                );
            }

            // Apply PDC delay (compensate for insert chain latency differences)
            track.delay_line.process(&mut track.left[..chunk], &mut track.right[..chunk]);

            if !audible {
                continue;
            }

            // Apply volume
            let vol = track.volume;
            for s in &mut track.left[..chunk] {
                *s *= vol;
            }
            for s in &mut track.right[..chunk] {
                *s *= vol;
            }

            // Apply pan (constant-power)
            apply_pan(&mut track.left[..chunk], &mut track.right[..chunk], track.pan);

            // Update track meter (post-fader)
            track.meter.update(&track.left[..chunk], &track.right[..chunk]);

            // Sum into master
            for i in 0..chunk {
                self.master.left[i] += track.left[i];
                self.master.right[i] += track.right[i];
            }

            // Accumulate into send buses
            for (bus_idx, bus) in self.send_buses.iter_mut().enumerate() {
                let send = if bus_idx < track.send_levels.len() {
                    track.send_levels[bus_idx]
                } else {
                    0.0
                };
                if send > 0.0 {
                    for i in 0..chunk {
                        bus.acc_left[i] += track.left[i] * send;
                        bus.acc_right[i] += track.right[i] * send;
                    }
                }
            }
        }

        // Process send buses (effect mode: feed accumulated audio through effect engine)
        for bus in &mut self.send_buses {
            bus.out_left[..chunk].fill(0.0);
            bus.out_right[..chunk].fill(0.0);
            bus.engine.process_effect(
                &bus.acc_left[..chunk],
                &bus.acc_right[..chunk],
                &mut bus.out_left[..chunk],
                &mut bus.out_right[..chunk],
            );

            // Mix effect output into master
            let level = bus.level;
            for i in 0..chunk {
                self.master.left[i] += bus.out_left[i] * level;
                self.master.right[i] += bus.out_right[i] * level;
            }
        }

        // Apply master volume + limiter
        let mvol = self.master.volume;
        let threshold = self.master.limiter_threshold;
        for i in 0..chunk {
            output_left[i] = soft_limit(self.master.left[i] * mvol, threshold);
            output_right[i] = soft_limit(self.master.right[i] * mvol, threshold);
        }

        // Update master meter (post-limiter)
        self.master.meter.update(&output_left[..chunk], &output_right[..chunk]);
    }

    /// Get the master bus meter (for main thread level reading).
    pub fn master_meter(&self) -> &LevelMeter {
        &self.master.meter
    }

    /// Get a track's meter by ID.
    pub fn track_meter(&self, track_id: u32) -> Option<&LevelMeter> {
        self.tracks.iter().find(|t| t.id == track_id).map(|t| &t.meter)
    }
}

/// Process insert effect chain using ping-pong buffers.
///
/// Alternates between track buffers (left/right) and scratch buffers to avoid
/// allocation. If the result ends up in scratch, copies back to track buffers.
///
/// Split borrows: `inserts`, `left/right`, and `scratch_left/scratch_right` are
/// disjoint fields of Track, passed separately to satisfy the borrow checker.
fn process_insert_chain(
    inserts: &mut [InsertEffect],
    left: &mut [f32],
    right: &mut [f32],
    scratch_left: &mut [f32],
    scratch_right: &mut [f32],
    chunk: usize,
) {
    let mut in_scratch = false;
    for insert in inserts.iter_mut() {
        if insert.bypass {
            continue;
        }
        if !in_scratch {
            // Read from left/right, write to scratch
            insert.engine.process_effect(
                &left[..chunk],
                &right[..chunk],
                &mut scratch_left[..chunk],
                &mut scratch_right[..chunk],
            );
            in_scratch = true;
        } else {
            // Read from scratch, write to left/right
            insert.engine.process_effect(
                &scratch_left[..chunk],
                &scratch_right[..chunk],
                &mut left[..chunk],
                &mut right[..chunk],
            );
            in_scratch = false;
        }
    }
    // If final result is in scratch, copy back to track buffers
    if in_scratch {
        left[..chunk].copy_from_slice(&scratch_left[..chunk]);
        right[..chunk].copy_from_slice(&scratch_right[..chunk]);
    }
}

/// Constant-power pan law.
/// Center (pan=0): L=R=cos(π/4)≈0.707 (−3dB each, total power preserved).
/// Hard left (pan=−1): L=1.0, R=0.0. Hard right (pan=+1): L=0.0, R=1.0.
fn apply_pan(left: &mut [f32], right: &mut [f32], pan: f32) {
    let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
    let gain_l = angle.cos();
    let gain_r = angle.sin();
    for s in left.iter_mut() {
        *s *= gain_l;
    }
    for s in right.iter_mut() {
        *s *= gain_r;
    }
}

/// Soft limiter: passes through below threshold, tanh compression above.
/// Output is clamped to [-1.0, 1.0] to guarantee no clipping past the DAC range.
fn soft_limit(sample: f32, threshold: f32) -> f32 {
    let abs = sample.abs();
    if abs <= threshold {
        sample
    } else {
        let sign = sample.signum();
        let excess = (abs - threshold) / (1.0 - threshold);
        (sign * (threshold + (1.0 - threshold) * excess.tanh())).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pan_center_is_minus_3db() {
        let mut l = vec![1.0];
        let mut r = vec![1.0];
        apply_pan(&mut l, &mut r, 0.0);
        // At center: gain = cos(π/4) ≈ 0.7071
        let expected = std::f32::consts::FRAC_1_SQRT_2;
        assert!((l[0] - expected).abs() < 0.001, "Center L should be ~0.707, got {}", l[0]);
        assert!((r[0] - expected).abs() < 0.001, "Center R should be ~0.707, got {}", r[0]);
    }

    #[test]
    fn test_pan_hard_left() {
        let mut l = vec![1.0];
        let mut r = vec![1.0];
        apply_pan(&mut l, &mut r, -1.0);
        assert!(l[0] > 0.99); // nearly full
        assert!(r[0] < 0.01); // nearly zero
    }

    #[test]
    fn test_pan_hard_right() {
        let mut l = vec![1.0];
        let mut r = vec![1.0];
        apply_pan(&mut l, &mut r, 1.0);
        assert!(l[0] < 0.01);
        assert!(r[0] > 0.99);
    }

    #[test]
    fn test_soft_limit_below_threshold() {
        assert_eq!(soft_limit(0.5, 0.95), 0.5);
        assert_eq!(soft_limit(-0.3, 0.95), -0.3);
    }

    #[test]
    fn test_soft_limit_above_threshold() {
        let limited = soft_limit(2.0, 0.95);
        assert!(limited > 0.95);
        // Output approaches 1.0 asymptotically but never exceeds it meaningfully
        assert!(limited <= 1.0 + f32::EPSILON);
        // Should be less than the input
        assert!(limited < 2.0);
    }

    #[test]
    fn test_soft_limit_preserves_sign() {
        let pos = soft_limit(2.0, 0.95);
        let neg = soft_limit(-2.0, 0.95);
        assert!(pos > 0.0);
        assert!(neg < 0.0);
        assert!((pos + neg).abs() < 0.001);
    }

    #[test]
    fn test_mixer_empty_renders_silence() {
        let mut mixer = Mixer::new(44100, 256);
        let mut left = vec![1.0; 256];
        let mut right = vec![1.0; 256];
        mixer.render(&mut left, &mut right);
        assert!(left.iter().all(|&s| s == 0.0));
        assert!(right.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_mixer_single_track() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        mixer.add_track(engine, 0xFFFF); // all channels
        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Engine with no backend renders silence — should pass without crash
    }

    #[test]
    fn test_mixer_mute() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let id = mixer.add_track(engine, 0xFFFF);
        mixer.track_mut(id).unwrap().mute = true;
        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Muted track contributes nothing
        assert!(left.iter().all(|&s| s == 0.0));
    }

    // --- Insert effect tests ---

    #[test]
    fn test_add_insert() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let effect = Engine::new(44100, 256);
        let insert_id = mixer.add_insert(track_id, effect);
        assert!(insert_id.is_some());
        assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 1);
    }

    #[test]
    fn test_add_insert_invalid_track() {
        let mut mixer = Mixer::new(44100, 256);
        let effect = Engine::new(44100, 256);
        assert!(mixer.add_insert(999, effect).is_none());
    }

    #[test]
    fn test_remove_insert() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let effect = Engine::new(44100, 256);
        let insert_id = mixer.add_insert(track_id, effect).unwrap();
        assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 1);

        let removed = mixer.remove_insert(track_id, insert_id);
        assert!(removed.is_some());
        assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 0);
    }

    #[test]
    fn test_remove_insert_invalid() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);
        assert!(mixer.remove_insert(track_id, 999).is_none());
        assert!(mixer.remove_insert(999, 0).is_none());
    }

    #[test]
    fn test_insert_bypass() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let effect = Engine::new(44100, 256);
        let insert_id = mixer.add_insert(track_id, effect).unwrap();

        // Default: not bypassed
        assert!(!mixer.track(track_id).unwrap().inserts[0].bypass);

        mixer.set_insert_bypass(track_id, insert_id, true);
        assert!(mixer.track(track_id).unwrap().inserts[0].bypass);

        mixer.set_insert_bypass(track_id, insert_id, false);
        assert!(!mixer.track(track_id).unwrap().inserts[0].bypass);
    }

    #[test]
    fn test_insert_ids_are_unique() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let id1 = mixer.add_insert(track_id, Engine::new(44100, 256)).unwrap();
        let id2 = mixer.add_insert(track_id, Engine::new(44100, 256)).unwrap();
        let id3 = mixer.add_insert(track_id, Engine::new(44100, 256)).unwrap();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
    }

    #[test]
    fn test_insert_chain_renders_without_crash() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);

        // Add 3 inserts (no-backend engines = they zero the output, simulating effects)
        mixer.add_insert(track_id, Engine::new(44100, 256));
        mixer.add_insert(track_id, Engine::new(44100, 256));
        mixer.add_insert(track_id, Engine::new(44100, 256));

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Should not crash
    }

    #[test]
    fn test_insert_chain_all_bypassed() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let id1 = mixer.add_insert(track_id, Engine::new(44100, 256)).unwrap();
        let id2 = mixer.add_insert(track_id, Engine::new(44100, 256)).unwrap();
        mixer.set_insert_bypass(track_id, id1, true);
        mixer.set_insert_bypass(track_id, id2, true);

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // All bypassed = same as no inserts
    }

    #[test]
    fn test_process_insert_chain_passthrough_when_empty() {
        // With no inserts, audio should be unmodified
        let mut left = vec![0.5; 64];
        let mut right = vec![0.3; 64];
        let mut scratch_l = vec![0.0; 64];
        let mut scratch_r = vec![0.0; 64];
        let mut inserts: Vec<InsertEffect> = vec![];

        process_insert_chain(&mut inserts, &mut left, &mut right, &mut scratch_l, &mut scratch_r, 64);

        assert!(left.iter().all(|&s| (s - 0.5).abs() < f32::EPSILON));
        assert!(right.iter().all(|&s| (s - 0.3).abs() < f32::EPSILON));
    }

    #[test]
    fn test_process_insert_chain_all_bypassed_passthrough() {
        let mut left = vec![0.5; 64];
        let mut right = vec![0.3; 64];
        let mut scratch_l = vec![0.0; 64];
        let mut scratch_r = vec![0.0; 64];
        let mut inserts = vec![
            InsertEffect { id: 0, engine: Engine::new(44100, 64), bypass: true },
            InsertEffect { id: 1, engine: Engine::new(44100, 64), bypass: true },
        ];

        process_insert_chain(&mut inserts, &mut left, &mut right, &mut scratch_l, &mut scratch_r, 64);

        // All bypassed = audio unchanged
        assert!(left.iter().all(|&s| (s - 0.5).abs() < f32::EPSILON));
        assert!(right.iter().all(|&s| (s - 0.3).abs() < f32::EPSILON));
    }

    #[test]
    fn test_track_insert_accessor() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = Engine::new(44100, 256);
        let track_id = mixer.add_track(engine, 0xFFFF);
        let insert_id = mixer.add_insert(track_id, Engine::new(44100, 256)).unwrap();

        assert!(mixer.track_insert(track_id, insert_id).is_some());
        assert!(mixer.track_insert(track_id, 999).is_none());
        assert!(mixer.track_insert(999, insert_id).is_none());
    }

    #[test]
    fn test_multiple_tracks_with_inserts() {
        let mut mixer = Mixer::new(44100, 256);
        let t1 = mixer.add_track(Engine::new(44100, 256), 0x0001);
        let t2 = mixer.add_track(Engine::new(44100, 256), 0x0002);

        mixer.add_insert(t1, Engine::new(44100, 256));
        mixer.add_insert(t1, Engine::new(44100, 256));
        mixer.add_insert(t2, Engine::new(44100, 256));

        assert_eq!(mixer.track(t1).unwrap().inserts.len(), 2);
        assert_eq!(mixer.track(t2).unwrap().inserts.len(), 1);

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Should not crash with multiple tracks each having inserts
    }

    // --- PDC tests ---

    #[test]
    fn test_delay_line_passthrough_when_zero() {
        let mut dl = DelayLine::new();
        let mut left = vec![1.0, 2.0, 3.0];
        let mut right = vec![4.0, 5.0, 6.0];
        dl.process(&mut left, &mut right);
        // Zero delay = passthrough
        assert_eq!(left, vec![1.0, 2.0, 3.0]);
        assert_eq!(right, vec![4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_delay_line_delays_by_n_samples() {
        let mut dl = DelayLine::new();
        dl.set_delay(2);

        // First block: input [1,2,3], output should be [0,0,1] (delayed by 2)
        let mut left = vec![1.0, 2.0, 3.0];
        let mut right = vec![0.0; 3];
        dl.process(&mut left, &mut right);
        assert_eq!(left, vec![0.0, 0.0, 1.0]);

        // Second block: input [4,5,6], output should be [2,3,4] (continuing)
        let mut left2 = vec![4.0, 5.0, 6.0];
        let mut right2 = vec![0.0; 3];
        dl.process(&mut left2, &mut right2);
        assert_eq!(left2, vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_delay_line_set_delay_clears_buffer() {
        let mut dl = DelayLine::new();
        dl.set_delay(4);
        let mut left = vec![1.0; 4];
        let mut right = vec![0.0; 4];
        dl.process(&mut left, &mut right);
        // Output is zeros (delay buffer was initialized to zero)
        assert!(left.iter().all(|&s| s == 0.0));

        // Changing delay clears buffer
        dl.set_delay(2);
        let mut left2 = vec![5.0; 2];
        let mut right2 = vec![0.0; 2];
        dl.process(&mut left2, &mut right2);
        assert!(left2.iter().all(|&s| s == 0.0)); // Fresh zero buffer
    }

    #[test]
    fn test_pdc_no_inserts_no_delay() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(Engine::new(44100, 256), 0xFFFF);
        mixer.add_track(Engine::new(44100, 256), 0xFFFF);
        mixer.recalculate_pdc();

        // No inserts → no latency → no delay
        assert_eq!(mixer.tracks()[0].delay_line.delay, 0);
        assert_eq!(mixer.tracks()[1].delay_line.delay, 0);
    }

    #[test]
    fn test_pdc_recalculate_on_insert_add() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(Engine::new(44100, 256), 0x0001);
        mixer.add_track(Engine::new(44100, 256), 0x0002);

        // Add insert to track 0 (Engine with no backend reports 0 latency)
        mixer.add_insert(0, Engine::new(44100, 256));

        // Both tracks have 0 latency (no backend) → no compensation
        assert_eq!(mixer.tracks()[0].delay_line.delay, 0);
        assert_eq!(mixer.tracks()[1].delay_line.delay, 0);
    }

    #[test]
    fn test_pdc_renders_without_crash() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(Engine::new(44100, 256), 0xFFFF);
        mixer.add_track(Engine::new(44100, 256), 0xFFFF);
        mixer.add_insert(0, Engine::new(44100, 256));
        mixer.recalculate_pdc();

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Should render without crash
    }

    #[test]
    fn test_pdc_bypass_recalculates() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(Engine::new(44100, 256), 0xFFFF);
        mixer.add_track(Engine::new(44100, 256), 0xFFFF);
        let i0 = mixer.add_insert(t0, Engine::new(44100, 256)).unwrap();

        // Bypass should trigger recalculation
        mixer.set_insert_bypass(t0, i0, true);
        // No crash, PDC updated
    }
}
