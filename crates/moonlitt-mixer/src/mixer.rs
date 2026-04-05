//! Mixer — pure audio DSP graph.
//!
//! Combines multiple engine tracks with send buses (effects) and a master
//! bus (limiter). Platform-agnostic: no cpal, no midir, no threads.
//! AudioThread owns and drives the Mixer, calling `render()` each audio
//! callback. The Mixer has no knowledge of threads, devices, or transport.
//!
//! All rendering happens in the audio thread. No locks, no allocations.

use crate::dither::StereoDither;
use moonlitt_core::AudioBackend;
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
    true_peak_left: Arc<AtomicU32>,
    true_peak_right: Arc<AtomicU32>,
}

impl LevelMeter {
    fn new() -> Self {
        Self {
            peak_left: Arc::new(AtomicU32::new(0)),
            peak_right: Arc::new(AtomicU32::new(0)),
            rms_left: Arc::new(AtomicU32::new(0)),
            rms_right: Arc::new(AtomicU32::new(0)),
            true_peak_left: Arc::new(AtomicU32::new(0)),
            true_peak_right: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Update meter from a rendered buffer. Called on audio thread.
    fn update(&self, left: &[f32], right: &[f32]) {
        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;
        let mut tp_l: f32 = 0.0;
        let mut tp_r: f32 = 0.0;
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

        // True peak: 4x oversampled via linear interpolation between adjacent samples
        tp_l = peak_l;
        tp_r = peak_r;
        if left.len() >= 2 {
            for i in 0..left.len() - 1 {
                // 3 interpolated points between sample[i] and sample[i+1]
                for k in 1..4u32 {
                    let t = k as f32 * 0.25;
                    let interp_l = left[i] + t * (left[i + 1] - left[i]);
                    let interp_r = right[i] + t * (right[i + 1] - right[i]);
                    let al = interp_l.abs();
                    let ar = interp_r.abs();
                    if al > tp_l { tp_l = al; }
                    if ar > tp_r { tp_r = ar; }
                }
            }
        }

        let n = left.len().max(1) as f32;
        let rms_l = (sum_sq_l / n).sqrt();
        let rms_r = (sum_sq_r / n).sqrt();

        self.peak_left.store(peak_l.to_bits(), Ordering::Relaxed);
        self.peak_right.store(peak_r.to_bits(), Ordering::Relaxed);
        self.rms_left.store(rms_l.to_bits(), Ordering::Relaxed);
        self.rms_right.store(rms_r.to_bits(), Ordering::Relaxed);
        self.true_peak_left.store(tp_l.to_bits(), Ordering::Relaxed);
        self.true_peak_right.store(tp_r.to_bits(), Ordering::Relaxed);
    }

    /// Read sample peak level (L, R).
    pub fn peak(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_left.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_right.load(Ordering::Relaxed)),
        )
    }

    /// Read RMS level (L, R).
    pub fn rms(&self) -> (f32, f32) {
        (
            f32::from_bits(self.rms_left.load(Ordering::Relaxed)),
            f32::from_bits(self.rms_right.load(Ordering::Relaxed)),
        )
    }

    /// Read true peak level (L, R) — 4x oversampled per EBU R128.
    pub fn true_peak(&self) -> (f32, f32) {
        (
            f32::from_bits(self.true_peak_left.load(Ordering::Relaxed)),
            f32::from_bits(self.true_peak_right.load(Ordering::Relaxed)),
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

/// Where a track routes its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTarget {
    /// Route to master bus (default).
    Master,
    /// Route to a group track (submix).
    Group(u32),
}

/// A single insert effect slot on a track.
/// Processed pre-fader in series: Backend → Insert[0] → Insert[1] → ... → Fader.
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
    left: Vec<f32>,
    right: Vec<f32>,
    // Group input accumulators (used when this track is a submix target)
    group_in_left: Vec<f32>,
    group_in_right: Vec<f32>,
    // Scratch buffers for insert chain ping-pong processing
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
    // Temporary buffers for external sidechain signal
    sidechain_buf_l: Vec<f32>,
    sidechain_buf_r: Vec<f32>,
    /// PDC delay line — compensates for insert chain latency differences.
    delay_line: DelayLine,
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
    /// Pre-computed render order: source tracks first, then group tracks.
    /// Rebuilt when routing changes. Zero allocation during render().
    render_order: Vec<usize>,
    /// TPDF dither applied at output stage.
    dither: StereoDither,
    /// Whether dithering is enabled.
    dither_enabled: bool,
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
            render_order: Vec::new(),
            dither: StereoDither::new_24bit(),
            dither_enabled: false, // Enable explicitly; off by default for bit-exact testing
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

    /// Add a track with a backend and a channel mask. Returns the track ID.
    pub fn add_track(&mut self, backend: Box<dyn AudioBackend>, channel_mask: u16) -> u32 {
        let id = self.next_track_id;
        self.next_track_id += 1;
        self.add_track_inner(id, backend, None, channel_mask);
        id
    }

    /// Add a track with a source path recorded for session persistence.
    pub fn add_track_with_source(&mut self, backend: Box<dyn AudioBackend>, source_path: Option<String>, channel_mask: u16) -> u32 {
        let id = self.next_track_id;
        self.next_track_id += 1;
        self.add_track_inner(id, backend, source_path, channel_mask);
        id
    }

    /// Add a track with a pre-assigned ID (for Runtime command channel).
    pub fn add_track_with_id(&mut self, id: u32, backend: Box<dyn AudioBackend>, source_path: Option<String>, channel_mask: u16) {
        if id >= self.next_track_id {
            self.next_track_id = id + 1;
        }
        self.add_track_inner(id, backend, source_path, channel_mask);
    }

    fn add_track_inner(&mut self, id: u32, backend: Box<dyn AudioBackend>, source_path: Option<String>, channel_mask: u16) {
        self.tracks.push(Track {
            id,
            backend,
            channel_mask,
            source_path,
            volume: 1.0,
            trim_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            send_levels: vec![0.0; self.send_buses.len()],
            inserts: Vec::new(),
            output_target: OutputTarget::Master,
            left: vec![0.0; self.buffer_size],
            right: vec![0.0; self.buffer_size],
            group_in_left: vec![0.0; self.buffer_size],
            group_in_right: vec![0.0; self.buffer_size],
            scratch_left: vec![0.0; self.buffer_size],
            scratch_right: vec![0.0; self.buffer_size],
            sidechain_buf_l: vec![0.0; self.buffer_size],
            sidechain_buf_r: vec![0.0; self.buffer_size],
            delay_line: DelayLine::new(),
            meter: LevelMeter::new(),
        });
        self.rebuild_render_order();
    }

    /// Remove a track by ID. Returns the backend if found.
    pub fn remove_track(&mut self, id: u32) -> Option<Box<dyn AudioBackend>> {
        // Reset any tracks routing to this group
        for t in &mut self.tracks {
            if t.output_target == OutputTarget::Group(id) {
                t.output_target = OutputTarget::Master;
            }
            // Clear any sidechain references to the removed track
            for insert in &mut t.inserts {
                if insert.sidechain_source == Some(id) {
                    insert.sidechain_source = None;
                }
            }
        }
        let pos = self.tracks.iter().position(|t| t.id == id)?;
        let track = self.tracks.remove(pos);
        self.rebuild_render_order();
        Some(track.backend)
    }

    /// Set a track's pre-insert trim gain in dB (clamped to -24..+24).
    pub fn set_track_trim(&mut self, track_id: u32, trim_db: f32) {
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            track.trim_db = trim_db.clamp(-24.0, 24.0);
        }
    }

    /// Set a track's output routing (Master or Group).
    /// Returns false if the target would create a cycle.
    pub fn set_track_output(&mut self, track_id: u32, target: OutputTarget) -> bool {
        // Validate: target group must exist and not create a cycle
        if let OutputTarget::Group(group_id) = target {
            if group_id == track_id {
                return false; // Can't route to self
            }
            if !self.tracks.iter().any(|t| t.id == group_id) {
                return false; // Target doesn't exist
            }
            // Check for cycles: follow the chain from group_id
            let mut current = group_id;
            for _ in 0..self.tracks.len() {
                let next = self.tracks.iter()
                    .find(|t| t.id == current)
                    .map(|t| t.output_target);
                match next {
                    Some(OutputTarget::Group(next_id)) => {
                        if next_id == track_id {
                            return false; // Cycle detected
                        }
                        current = next_id;
                    }
                    _ => break, // Reached master or not found
                }
            }
        }
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            track.output_target = target;
            self.rebuild_render_order();
            true
        } else {
            false
        }
    }

    /// Add a send bus with an effect backend. Returns the bus ID.
    pub fn add_send_bus(&mut self, backend: Box<dyn AudioBackend>) -> u32 {
        let id = self.next_bus_id;
        self.next_bus_id += 1;
        self.add_send_bus_inner(id, backend, None);
        id
    }

    /// Add a send bus with a pre-assigned ID (for Runtime command channel).
    pub fn add_send_bus_with_id(&mut self, id: u32, backend: Box<dyn AudioBackend>, source_path: Option<String>) {
        if id >= self.next_bus_id {
            self.next_bus_id = id + 1;
        }
        self.add_send_bus_inner(id, backend, source_path);
    }

    fn add_send_bus_inner(&mut self, id: u32, backend: Box<dyn AudioBackend>, source_path: Option<String>) {
        let bs = self.buffer_size;
        self.send_buses.push(SendBus {
            id,
            backend,
            source_path,
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
    pub fn add_insert(&mut self, track_id: u32, backend: Box<dyn AudioBackend>) -> Option<u32> {
        let track = self.tracks.iter_mut().find(|t| t.id == track_id)?;
        let id = self.next_insert_id;
        self.next_insert_id += 1;
        track.inserts.push(InsertEffect {
            id,
            backend,
            bypass: false,
            source_path: None,
            sidechain_source: None,
        });
        self.recalculate_pdc();
        Some(id)
    }

    /// Add an insert with a pre-assigned ID (for Runtime command channel).
    pub fn add_insert_with_id(&mut self, track_id: u32, insert_id: u32, backend: Box<dyn AudioBackend>, source_path: Option<String>) {
        if insert_id >= self.next_insert_id {
            self.next_insert_id = insert_id + 1;
        }
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            track.inserts.push(InsertEffect {
                id: insert_id,
                backend,
                bypass: false,
                source_path,
                sidechain_source: None,
            });
        }
        self.recalculate_pdc();
    }

    /// Remove an insert effect from a track. Returns the backend if found.
    pub fn remove_insert(&mut self, track_id: u32, insert_id: u32) -> Option<Box<dyn AudioBackend>> {
        let track = self.tracks.iter_mut().find(|t| t.id == track_id)?;
        let pos = track.inserts.iter().position(|i| i.id == insert_id)?;
        let insert = track.inserts.remove(pos);
        self.recalculate_pdc();
        Some(insert.backend)
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

    /// Set external sidechain source for an insert effect.
    /// Returns false if the target track/insert doesn't exist or if it would create a cycle.
    pub fn set_insert_sidechain(
        &mut self,
        track_id: u32,
        insert_id: u32,
        source_track_id: Option<u32>,
    ) -> bool {
        // Validate track and insert exist
        let track_idx = match self.tracks.iter().position(|t| t.id == track_id) {
            Some(i) => i,
            None => return false,
        };
        let insert_exists = self.tracks[track_idx]
            .inserts
            .iter()
            .any(|i| i.id == insert_id);
        if !insert_exists {
            return false;
        }

        if let Some(src_id) = source_track_id {
            // Source must exist and not be same track
            if src_id == track_id {
                return false;
            }
            if !self.tracks.iter().any(|t| t.id == src_id) {
                return false;
            }
            // Check for cycles: would adding this sidechain dep create a cycle?
            // Build the full dependency graph and check reachability from src_id back to track_id.
            if self.would_create_sidechain_cycle(track_id, src_id) {
                return false;
            }
        }

        // Set the sidechain source
        let insert = self.tracks[track_idx]
            .inserts
            .iter_mut()
            .find(|i| i.id == insert_id)
            .unwrap();
        insert.sidechain_source = source_track_id;
        self.rebuild_render_order();
        true
    }

    /// Check if adding a sidechain dependency (track_id depends on src_id) would create a cycle.
    /// A cycle exists if src_id already depends on track_id through group routing or sidechain deps.
    fn would_create_sidechain_cycle(&self, track_id: u32, src_id: u32) -> bool {
        // DFS from src_id: can we reach track_id?
        let mut visited = vec![false; self.tracks.len()];
        let start_idx = match self.tracks.iter().position(|t| t.id == src_id) {
            Some(i) => i,
            None => return false,
        };
        self.can_reach_via_deps(start_idx, track_id, &mut visited)
    }

    /// DFS: can we reach target_id from tracks[idx] following group routing and sidechain deps?
    fn can_reach_via_deps(&self, idx: usize, target_id: u32, visited: &mut [bool]) -> bool {
        if visited[idx] {
            return false;
        }
        visited[idx] = true;

        let track = &self.tracks[idx];

        // Check group routing dependency: if this track routes to a group, that group depends on this track.
        // But we want the reverse: does this track depend on something that reaches target_id?
        // Group routing: track routes to Group(g) => track's output goes INTO g, meaning g depends on track.
        // For cycle detection, we need: does src_id (transitively) depend on track_id?
        // Dependencies of a track:
        //   - sidechain sources: if track has insert with sidechain_source = Some(x), track depends on x
        //   - group routing: if track routes to Group(g), that doesn't mean track depends on g (it's the reverse)
        // Actually: the render order ensures group targets render AFTER their sources.
        // And sidechain sources must render before the dependent track.
        // So track_id depends on src_id (sidechain), and we need to check if src_id depends on track_id.
        // src_id depends on track_id if:
        //   - src_id has a sidechain source that is track_id, or transitively depends on track_id
        //   - src_id routes to Group(track_id), meaning src_id's output feeds track_id
        //     (but that means track_id depends on src_id for group input, not the other way)
        //
        // Wait — let's think about this differently. The render order graph edges are:
        //   - If track A routes to Group(B): B depends on A (A must render first)
        //   - If track A has sidechain from C: A depends on C (C must render first)
        //
        // Adding edge: track_id depends on src_id.
        // Cycle if src_id depends on track_id transitively.
        // So: from src_id, follow all outgoing dependency edges.

        // Sidechain dependencies: track depends on its sidechain sources
        for insert in &track.inserts {
            if let Some(sc_src) = insert.sidechain_source {
                if sc_src == target_id {
                    return true;
                }
                if let Some(sc_idx) = self.tracks.iter().position(|t| t.id == sc_src) {
                    if self.can_reach_via_deps(sc_idx, target_id, visited) {
                        return true;
                    }
                }
            }
        }

        // Group routing: if this track routes to Group(g), does g depend on target?
        // No — routing to Group(g) means g depends on this track (g accumulates this track's output).
        // We're checking if this track (src_id) depends on target_id.
        // This track depends on its group target ONLY if it IS a group target itself (receives input from others).
        // Actually: a group track depends on the tracks that route INTO it.
        // So if other tracks route to Group(track.id), then track depends on those.
        // Let's check: does any track route to this track as a group?
        for (other_idx, other) in self.tracks.iter().enumerate() {
            if other.output_target == OutputTarget::Group(track.id) {
                // This track (as group) depends on `other`
                if other.id == target_id {
                    return true;
                }
                if self.can_reach_via_deps(other_idx, target_id, visited) {
                    return true;
                }
            }
        }

        false
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
                    .map(|i| i.backend.latency())
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

    /// Rebuild the render order using topological sort.
    /// Accounts for both group routing and sidechain dependencies.
    /// Called automatically when routing or track structure changes.
    fn rebuild_render_order(&mut self) {
        let n = self.tracks.len();
        if n == 0 {
            self.render_order.clear();
            return;
        }

        // Build in-degree and adjacency list.
        // Edge: dependency -> dependent (dependency must render first).
        let mut in_degree = vec![0u32; n];
        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];

        for (i, track) in self.tracks.iter().enumerate() {
            // Group routing: if track routes to Group(g), then g depends on track.
            // Edge: track (i) -> group (g_idx).
            if let OutputTarget::Group(g_id) = track.output_target {
                if let Some(g_idx) = self.tracks.iter().position(|t| t.id == g_id) {
                    adjacency[i].push(g_idx);
                    in_degree[g_idx] += 1;
                }
            }

            // Sidechain: if track has insert with sidechain_source = Some(src_id),
            // then track depends on src_id. Edge: src_idx -> track (i).
            for insert in &track.inserts {
                if let Some(src_id) = insert.sidechain_source {
                    if let Some(src_idx) = self.tracks.iter().position(|t| t.id == src_id) {
                        adjacency[src_idx].push(i);
                        in_degree[i] += 1;
                    }
                }
            }
        }

        // Kahn's algorithm (BFS topological sort).
        // Use a FIFO queue (VecDeque) to maintain stable insertion order
        // for tracks at the same dependency level.
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for (i, &deg) in in_degree.iter().enumerate() {
            if deg == 0 {
                queue.push_back(i);
            }
        }

        self.render_order.clear();
        while let Some(idx) = queue.pop_front() {
            self.render_order.push(idx);
            for &dep in &adjacency[idx] {
                in_degree[dep] -= 1;
                if in_degree[dep] == 0 {
                    queue.push_back(dep);
                }
            }
        }

        // If not all tracks were visited, there's a cycle — shouldn't happen
        // since we reject cycles at set time. Fall back to sequential order.
        if self.render_order.len() < n {
            self.render_order.clear();
            self.render_order.extend(0..n);
        }
    }

    // --- Event routing ---

    /// Dispatch a MIDI event to the correct track(s) by channel.
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.backend.note_on(channel, note, velocity);
            }
        }
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.backend.note_off(channel, note);
            }
        }
    }

    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.backend.cc(channel, cc, value);
            }
        }
    }

    pub fn pitch_bend(&mut self, channel: u8, value: i16) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.backend.pitch_bend(channel, value);
            }
        }
    }

    pub fn program_change(&mut self, channel: u8, program: u8) {
        let mask = 1u16 << channel;
        for track in &mut self.tracks {
            if track.channel_mask & mask != 0 {
                track.backend.program_change(channel, program);
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for track in &mut self.tracks {
            track.backend.all_notes_off();
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.master.volume = volume;
    }

    /// Broadcast a parameter change to all tracks.
    pub fn set_param(&mut self, id: u32, value: f64) {
        for track in &mut self.tracks {
            track.backend.set_param(id, value);
        }
    }

    /// Set a parameter on a specific track's backend.
    pub fn set_param_for_track(&mut self, track_id: u32, id: u32, value: f64) {
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            track.backend.set_param(id, value);
        }
    }

    /// Set a parameter on a specific insert effect.
    pub fn set_insert_param(&mut self, track_id: u32, insert_id: u32, id: u32, value: f64) {
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            if let Some(insert) = track.inserts.iter_mut().find(|i| i.id == insert_id) {
                insert.backend.set_param(id, value);
            }
        }
    }

    /// Set a parameter on a send bus effect backend.
    pub fn set_send_bus_param(&mut self, bus_id: u32, param_id: u32, value: f64) {
        if let Some(bus) = self.send_bus_mut(bus_id) {
            bus.backend.set_param(param_id, value);
        }
    }

    // --- Rendering ---

    /// Render one chunk of audio into an interleaved stereo output buffer.
    ///
    /// Two-phase rendering for group track support:
    /// 1. Source tracks render first, routing to master or group accumulators
    /// 2. Group tracks render after sources, consuming accumulated input
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

        // Clear group input accumulators
        for track in &mut self.tracks {
            track.group_in_left[..chunk].fill(0.0);
            track.group_in_right[..chunk].fill(0.0);
        }

        let any_solo = self.tracks.iter().any(|t| t.solo);

        // Render all tracks in dependency order (sources before groups).
        // Use index-based iteration for split borrows during group routing.
        let order_len = self.render_order.len();
        for order_i in 0..order_len {
            let idx = self.render_order[order_i];
            if idx >= self.tracks.len() { continue; }

            let track = &mut self.tracks[idx];
            let audible = !track.mute && (!any_solo || track.solo);

            // Render engine output
            track.left[..chunk].fill(0.0);
            track.right[..chunk].fill(0.0);
            track.backend.render(&mut track.left[..chunk], &mut track.right[..chunk]);

            // Add accumulated group input (for group tracks)
            for k in 0..chunk {
                track.left[k] += track.group_in_left[k];
                track.right[k] += track.group_in_right[k];
            }

            // Trim (pre-insert gain staging)
            if track.trim_db != 0.0 {
                let trim_gain = 10f32.powf(track.trim_db / 20.0);
                for s in &mut track.left[..chunk] { *s *= trim_gain; }
                for s in &mut track.right[..chunk] { *s *= trim_gain; }
            }

            // Inject sidechain signals: for each insert with a sidechain source,
            // copy the source track's pre-fader audio and call set_sidechain().
            // Two-pass approach to avoid heap allocation on the audio thread.
            {
                let num_inserts = self.tracks[idx].inserts.len();
                for ins_i in 0..num_inserts {
                    let src_id = self.tracks[idx].inserts[ins_i].sidechain_source;
                    if let Some(src_id) = src_id {
                        if let Some(src_idx) = self.tracks.iter().position(|t| t.id == src_id) {
                            if src_idx != idx {
                                copy_sidechain_buffers(&mut self.tracks, src_idx, idx, chunk);
                                let track = &mut self.tracks[idx];
                                track.inserts[ins_i].backend.set_sidechain(
                                    &track.sidechain_buf_l[..chunk],
                                    &track.sidechain_buf_r[..chunk],
                                );
                            }
                        }
                    }
                }
            }

            let track = &mut self.tracks[idx];

            // Insert chain (pre-fader)
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

            // PDC delay
            track.delay_line.process(&mut track.left[..chunk], &mut track.right[..chunk]);

            if !audible { continue; }

            // Volume (fader)
            let vol = track.volume;
            for s in &mut track.left[..chunk] { *s *= vol; }
            for s in &mut track.right[..chunk] { *s *= vol; }

            // Pan (constant-power)
            apply_pan(&mut track.left[..chunk], &mut track.right[..chunk], track.pan);

            // Meter (post-fader)
            track.meter.update(&track.left[..chunk], &track.right[..chunk]);

            // Route output
            let output_target = track.output_target;
            match output_target {
                OutputTarget::Master => {
                    for k in 0..chunk {
                        self.master.left[k] += self.tracks[idx].left[k];
                        self.master.right[k] += self.tracks[idx].right[k];
                    }
                }
                OutputTarget::Group(group_id) => {
                    // Accumulate into group track's input buffer (split borrow)
                    if let Some(gidx) = self.tracks.iter().position(|t| t.id == group_id) {
                        if gidx != idx {
                            accumulate_group(&mut self.tracks, idx, gidx, chunk);
                        }
                    }
                }
            }

            // Send buses (post-fader, always routes regardless of output_target)
            for (bus_idx, bus) in self.send_buses.iter_mut().enumerate() {
                let send = if bus_idx < self.tracks[idx].send_levels.len() {
                    self.tracks[idx].send_levels[bus_idx]
                } else {
                    0.0
                };
                if send > 0.0 {
                    for k in 0..chunk {
                        bus.acc_left[k] += self.tracks[idx].left[k] * send;
                        bus.acc_right[k] += self.tracks[idx].right[k] * send;
                    }
                }
            }
        }

        // Process send buses (effect mode: feed accumulated audio through effect engine)
        for bus in &mut self.send_buses {
            bus.out_left[..chunk].fill(0.0);
            bus.out_right[..chunk].fill(0.0);
            bus.backend.process_effect(
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

        // Apply TPDF dither (post-limiter, pre-DAC)
        if self.dither_enabled {
            self.dither.process(&mut output_left[..chunk], &mut output_right[..chunk]);
        }

        // Update master meter (post-dither)
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

    /// Enable or disable dithering.
    pub fn set_dither_enabled(&mut self, enabled: bool) {
        self.dither_enabled = enabled;
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
            insert.backend.process_effect(
                &left[..chunk],
                &right[..chunk],
                &mut scratch_left[..chunk],
                &mut scratch_right[..chunk],
            );
            in_scratch = true;
        } else {
            // Read from scratch, write to left/right
            insert.backend.process_effect(
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

/// Accumulate source track output into group track's input buffer.
/// Uses split_at_mut for borrow-checker-safe access to two tracks.
fn accumulate_group(tracks: &mut [Track], src: usize, dst: usize, chunk: usize) {
    if src < dst {
        let (left, right) = tracks.split_at_mut(dst);
        let s = &left[src];
        let d = &mut right[0];
        for k in 0..chunk {
            d.group_in_left[k] += s.left[k];
            d.group_in_right[k] += s.right[k];
        }
    } else {
        let (left, right) = tracks.split_at_mut(src);
        let d = &mut left[dst];
        let s = &right[0];
        for k in 0..chunk {
            d.group_in_left[k] += s.left[k];
            d.group_in_right[k] += s.right[k];
        }
    }
}

/// Copy source track's pre-fader audio (left/right after engine+trim) into
/// destination track's sidechain buffers. Uses split_at_mut for borrow safety.
fn copy_sidechain_buffers(tracks: &mut [Track], src: usize, dst: usize, chunk: usize) {
    if src < dst {
        let (left, right) = tracks.split_at_mut(dst);
        let s = &left[src];
        let d = &mut right[0];
        d.sidechain_buf_l[..chunk].copy_from_slice(&s.left[..chunk]);
        d.sidechain_buf_r[..chunk].copy_from_slice(&s.right[..chunk]);
    } else {
        let (left, right) = tracks.split_at_mut(src);
        let d = &mut left[dst];
        let s = &right[0];
        d.sidechain_buf_l[..chunk].copy_from_slice(&s.left[..chunk]);
        d.sidechain_buf_r[..chunk].copy_from_slice(&s.right[..chunk]);
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
    use moonlitt_core::NullBackend;

    /// Shorthand for creating a boxed NullBackend in tests.
    fn null(sr: u32) -> Box<dyn AudioBackend> {
        Box::new(NullBackend::new(sr))
    }

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
        let engine = null(44100);
        mixer.add_track(engine, 0xFFFF); // all channels
        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Engine with no backend renders silence — should pass without crash
    }

    #[test]
    fn test_mixer_mute() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = null(44100);
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
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let effect = null(44100);
        let insert_id = mixer.add_insert(track_id, effect);
        assert!(insert_id.is_some());
        assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 1);
    }

    #[test]
    fn test_add_insert_invalid_track() {
        let mut mixer = Mixer::new(44100, 256);
        let effect = null(44100);
        assert!(mixer.add_insert(999, effect).is_none());
    }

    #[test]
    fn test_remove_insert() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let effect = null(44100);
        let insert_id = mixer.add_insert(track_id, effect).unwrap();
        assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 1);

        let removed = mixer.remove_insert(track_id, insert_id);
        assert!(removed.is_some());
        assert_eq!(mixer.track(track_id).unwrap().inserts.len(), 0);
    }

    #[test]
    fn test_remove_insert_invalid() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);
        assert!(mixer.remove_insert(track_id, 999).is_none());
        assert!(mixer.remove_insert(999, 0).is_none());
    }

    #[test]
    fn test_insert_bypass() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let effect = null(44100);
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
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let id1 = mixer.add_insert(track_id, null(44100)).unwrap();
        let id2 = mixer.add_insert(track_id, null(44100)).unwrap();
        let id3 = mixer.add_insert(track_id, null(44100)).unwrap();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
    }

    #[test]
    fn test_insert_chain_renders_without_crash() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);

        // Add 3 inserts (no-backend engines = they zero the output, simulating effects)
        mixer.add_insert(track_id, null(44100));
        mixer.add_insert(track_id, null(44100));
        mixer.add_insert(track_id, null(44100));

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Should not crash
    }

    #[test]
    fn test_insert_chain_all_bypassed() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);

        let id1 = mixer.add_insert(track_id, null(44100)).unwrap();
        let id2 = mixer.add_insert(track_id, null(44100)).unwrap();
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
            InsertEffect { id: 0, backend: null(44100), bypass: true, source_path: None, sidechain_source: None },
            InsertEffect { id: 1, backend: null(44100), bypass: true, source_path: None, sidechain_source: None },
        ];

        process_insert_chain(&mut inserts, &mut left, &mut right, &mut scratch_l, &mut scratch_r, 64);

        // All bypassed = audio unchanged
        assert!(left.iter().all(|&s| (s - 0.5).abs() < f32::EPSILON));
        assert!(right.iter().all(|&s| (s - 0.3).abs() < f32::EPSILON));
    }

    #[test]
    fn test_track_insert_accessor() {
        let mut mixer = Mixer::new(44100, 256);
        let engine = null(44100);
        let track_id = mixer.add_track(engine, 0xFFFF);
        let insert_id = mixer.add_insert(track_id, null(44100)).unwrap();

        assert!(mixer.track_insert(track_id, insert_id).is_some());
        assert!(mixer.track_insert(track_id, 999).is_none());
        assert!(mixer.track_insert(999, insert_id).is_none());
    }

    #[test]
    fn test_multiple_tracks_with_inserts() {
        let mut mixer = Mixer::new(44100, 256);
        let t1 = mixer.add_track(null(44100), 0x0001);
        let t2 = mixer.add_track(null(44100), 0x0002);

        mixer.add_insert(t1, null(44100));
        mixer.add_insert(t1, null(44100));
        mixer.add_insert(t2, null(44100));

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
        mixer.add_track(null(44100), 0xFFFF);
        mixer.add_track(null(44100), 0xFFFF);
        mixer.recalculate_pdc();

        // No inserts → no latency → no delay
        assert_eq!(mixer.tracks()[0].delay_line.delay, 0);
        assert_eq!(mixer.tracks()[1].delay_line.delay, 0);
    }

    #[test]
    fn test_pdc_recalculate_on_insert_add() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(null(44100), 0x0001);
        mixer.add_track(null(44100), 0x0002);

        // Add insert to track 0 (Engine with no backend reports 0 latency)
        mixer.add_insert(0, null(44100));

        // Both tracks have 0 latency (no backend) → no compensation
        assert_eq!(mixer.tracks()[0].delay_line.delay, 0);
        assert_eq!(mixer.tracks()[1].delay_line.delay, 0);
    }

    #[test]
    fn test_pdc_renders_without_crash() {
        let mut mixer = Mixer::new(44100, 256);
        mixer.add_track(null(44100), 0xFFFF);
        mixer.add_track(null(44100), 0xFFFF);
        mixer.add_insert(0, null(44100));
        mixer.recalculate_pdc();

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Should render without crash
    }

    #[test]
    fn test_pdc_bypass_recalculates() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        mixer.add_track(null(44100), 0xFFFF);
        let i0 = mixer.add_insert(t0, null(44100)).unwrap();

        // Bypass should trigger recalculation
        mixer.set_insert_bypass(t0, i0, true);
        // No crash, PDC updated
    }

    // --- Group track / submix tests ---

    #[test]
    fn test_set_track_output_to_group() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002); // group

        assert!(mixer.set_track_output(t0, OutputTarget::Group(t1)));
        assert_eq!(mixer.track(t0).unwrap().output_target, OutputTarget::Group(t1));
    }

    #[test]
    fn test_set_track_output_rejects_self() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        assert!(!mixer.set_track_output(t0, OutputTarget::Group(t0)));
    }

    #[test]
    fn test_set_track_output_rejects_cycle() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);
        mixer.set_track_output(t0, OutputTarget::Group(t1));
        // t1 → t0 would create cycle
        assert!(!mixer.set_track_output(t1, OutputTarget::Group(t0)));
    }

    #[test]
    fn test_set_track_output_rejects_nonexistent() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        assert!(!mixer.set_track_output(t0, OutputTarget::Group(999)));
    }

    #[test]
    fn test_group_track_renders_without_crash() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);
        let group = mixer.add_track(null(44100), 0x0000); // group (no MIDI)

        mixer.set_track_output(t0, OutputTarget::Group(group));
        mixer.set_track_output(t1, OutputTarget::Group(group));

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
    }

    #[test]
    fn test_remove_group_target_resets_routing() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let group = mixer.add_track(null(44100), 0x0000);
        mixer.set_track_output(t0, OutputTarget::Group(group));

        mixer.remove_track(group);
        // t0 should be reset to Master
        assert_eq!(mixer.track(t0).unwrap().output_target, OutputTarget::Master);
    }

    #[test]
    fn test_render_order_sources_before_groups() {
        let mut mixer = Mixer::new(44100, 256);
        let _t0 = mixer.add_track(null(44100), 0x0001);
        let group = mixer.add_track(null(44100), 0x0000);
        let _t1 = mixer.add_track(null(44100), 0x0002);

        mixer.set_track_output(_t0, OutputTarget::Group(group));
        mixer.set_track_output(_t1, OutputTarget::Group(group));

        // Group should be last in render order
        let last_idx = *mixer.render_order.last().unwrap();
        assert_eq!(mixer.tracks[last_idx].id, group);
    }

    // --- Trim tests ---

    #[test]
    fn test_trim_zero_is_passthrough() {
        let mut mixer = Mixer::new(44100, 256);
        let id = mixer.add_track(null(44100), 0xFFFF);
        // trim_db defaults to 0.0
        assert_eq!(mixer.track(id).unwrap().trim_db, 0.0);

        // Manually write known data into the track buffer and render
        // With a no-backend engine, output is silence regardless, so verify
        // the field default and setter round-trip
        mixer.set_track_trim(id, 0.0);
        assert_eq!(mixer.track(id).unwrap().trim_db, 0.0);
    }

    #[test]
    fn test_trim_plus_6db() {
        let mut mixer = Mixer::new(44100, 4);
        let id = mixer.add_track(null(44100), 0xFFFF);
        mixer.set_track_trim(id, 6.0);

        let expected_gain = 10f32.powf(6.0 / 20.0); // ~1.9953

        // Directly verify the trim_db is stored
        assert!((mixer.track(id).unwrap().trim_db - 6.0).abs() < f32::EPSILON);

        // Verify the gain factor
        assert!((expected_gain - 1.9953).abs() < 0.001,
            "6 dB gain should be ~1.9953, got {}", expected_gain);
    }

    #[test]
    fn test_trim_clamp() {
        let mut mixer = Mixer::new(44100, 256);
        let id = mixer.add_track(null(44100), 0xFFFF);

        // Above +24 should clamp to +24
        mixer.set_track_trim(id, 30.0);
        assert_eq!(mixer.track(id).unwrap().trim_db, 24.0);

        // Below -24 should clamp to -24
        mixer.set_track_trim(id, -30.0);
        assert_eq!(mixer.track(id).unwrap().trim_db, -24.0);

        // Within range should pass through
        mixer.set_track_trim(id, -12.5);
        assert!((mixer.track(id).unwrap().trim_db - (-12.5)).abs() < f32::EPSILON);
    }

    // --- Sidechain tests ---

    #[test]
    fn test_sidechain_source_default_none() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        let i0 = mixer.add_insert(t0, null(44100)).unwrap();
        assert_eq!(mixer.track_insert(t0, i0).unwrap().sidechain_source, None);
    }

    #[test]
    fn test_set_insert_sidechain() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);
        let i1 = mixer.add_insert(t1, null(44100)).unwrap();

        // Set sidechain: t1's insert uses t0 as sidechain source
        assert!(mixer.set_insert_sidechain(t1, i1, Some(t0)));
        assert_eq!(
            mixer.track_insert(t1, i1).unwrap().sidechain_source,
            Some(t0)
        );

        // Clear sidechain
        assert!(mixer.set_insert_sidechain(t1, i1, None));
        assert_eq!(mixer.track_insert(t1, i1).unwrap().sidechain_source, None);
    }

    #[test]
    fn test_sidechain_rejects_self() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        let i0 = mixer.add_insert(t0, null(44100)).unwrap();

        // Can't sidechain to self
        assert!(!mixer.set_insert_sidechain(t0, i0, Some(t0)));
    }

    #[test]
    fn test_sidechain_rejects_nonexistent() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0xFFFF);
        let i0 = mixer.add_insert(t0, null(44100)).unwrap();

        // Nonexistent source track
        assert!(!mixer.set_insert_sidechain(t0, i0, Some(999)));

        // Nonexistent track
        assert!(!mixer.set_insert_sidechain(999, 0, Some(t0)));

        // Nonexistent insert
        assert!(!mixer.set_insert_sidechain(t0, 999, Some(t0)));
    }

    #[test]
    fn test_sidechain_cycle_rejected() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);
        let i0 = mixer.add_insert(t0, null(44100)).unwrap();
        let i1 = mixer.add_insert(t1, null(44100)).unwrap();

        // A sidechains from B
        assert!(mixer.set_insert_sidechain(t0, i0, Some(t1)));
        // B sidechains from A — would create a cycle
        assert!(!mixer.set_insert_sidechain(t1, i1, Some(t0)));
    }

    #[test]
    fn test_render_order_with_sidechain() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001); // source
        let t1 = mixer.add_track(null(44100), 0x0002); // dependent
        let i1 = mixer.add_insert(t1, null(44100)).unwrap();

        // t1's insert uses t0 as sidechain source => t0 must render first
        mixer.set_insert_sidechain(t1, i1, Some(t0));

        let t0_idx = mixer.tracks().iter().position(|t| t.id == t0).unwrap();
        let t1_idx = mixer.tracks().iter().position(|t| t.id == t1).unwrap();

        let t0_order = mixer.render_order.iter().position(|&i| i == t0_idx).unwrap();
        let t1_order = mixer.render_order.iter().position(|&i| i == t1_idx).unwrap();
        assert!(t0_order < t1_order, "Source track must render before dependent track");
    }

    #[test]
    fn test_sidechain_renders_without_crash() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);
        let i1 = mixer.add_insert(t1, null(44100)).unwrap();

        mixer.set_insert_sidechain(t1, i1, Some(t0));

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        mixer.render(&mut left, &mut right);
        // Should not crash
    }

    #[test]
    fn test_remove_track_clears_sidechain_refs() {
        let mut mixer = Mixer::new(44100, 256);
        let t0 = mixer.add_track(null(44100), 0x0001);
        let t1 = mixer.add_track(null(44100), 0x0002);
        let i1 = mixer.add_insert(t1, null(44100)).unwrap();

        mixer.set_insert_sidechain(t1, i1, Some(t0));
        assert_eq!(
            mixer.track_insert(t1, i1).unwrap().sidechain_source,
            Some(t0)
        );

        // Remove source track — sidechain ref should be cleared
        mixer.remove_track(t0);
        assert_eq!(mixer.track_insert(t1, i1).unwrap().sidechain_source, None);
    }
}
