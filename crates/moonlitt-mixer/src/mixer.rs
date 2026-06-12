//! Mixer — pure audio DSP graph.
//!
//! Combines multiple engine tracks with send buses (effects) and a master
//! bus (limiter). Platform-agnostic: no cpal, no midir, no threads.
//! AudioThread owns and drives the Mixer, calling `render()` each audio
//! callback. The Mixer has no knowledge of threads, devices, or transport.
//!
//! All rendering happens in the audio thread. No locks, no allocations.

use crate::channel::DelayLine;
use crate::dither::StereoDither;
use moonlitt_core::AudioBackend;

pub use crate::channel::{InsertEffect, MasterBus, OutputTarget, SendBus, Track};
pub use crate::meter::LevelMeter;

/// The mixer: owns tracks, send buses, and master.
pub struct Mixer {
    pub(crate) tracks: Vec<Track>,
    pub(crate) send_buses: Vec<SendBus>,
    pub(crate) master: MasterBus,
    pub(crate) buffer_size: usize,
    sample_rate: u32,
    next_track_id: u32,
    next_bus_id: u32,
    next_insert_id: u32,
    /// Pre-computed render order: source tracks first, then group tracks.
    /// Rebuilt when routing changes. Zero allocation during render().
    pub(crate) render_order: Vec<usize>,
    /// TPDF dither applied at output stage.
    pub(crate) dither: StereoDither,
    /// Whether dithering is enabled.
    pub(crate) dither_enabled: bool,
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
        self.add_track_inner(id, backend, None, channel_mask, LevelMeter::new());
        id
    }

    /// Add a track with a source path recorded for session persistence.
    pub fn add_track_with_source(
        &mut self,
        backend: Box<dyn AudioBackend>,
        source_path: Option<String>,
        channel_mask: u16,
    ) -> u32 {
        let id = self.next_track_id;
        self.next_track_id += 1;
        self.add_track_inner(id, backend, source_path, channel_mask, LevelMeter::new());
        id
    }

    /// Add a track with a pre-assigned ID (for Runtime command channel).
    pub fn add_track_with_id(
        &mut self,
        id: u32,
        backend: Box<dyn AudioBackend>,
        source_path: Option<String>,
        channel_mask: u16,
    ) {
        if id >= self.next_track_id {
            self.next_track_id = id + 1;
        }
        self.add_track_inner(id, backend, source_path, channel_mask, LevelMeter::new());
    }

    /// Add a track with a pre-built meter (for sharing with the main thread via Arc).
    pub fn add_track_with_meter(
        &mut self,
        id: u32,
        backend: Box<dyn AudioBackend>,
        channel_mask: u16,
        meter: LevelMeter,
    ) {
        if id >= self.next_track_id {
            self.next_track_id = id + 1;
        }
        self.add_track_inner(id, backend, None, channel_mask, meter);
    }

    fn add_track_inner(
        &mut self,
        id: u32,
        backend: Box<dyn AudioBackend>,
        source_path: Option<String>,
        channel_mask: u16,
        meter: LevelMeter,
    ) {
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
            meter,
        });
        self.rebuild_render_order();
    }

    /// Update which MIDI channels a track listens to. Bit N set ⇒ channel N
    /// will reach this track's backend. Used by the multi-track MIDI import
    /// flow to route each channel to its own DAW track.
    pub fn set_track_channel_mask(&mut self, track_id: u32, channel_mask: u16) {
        if let Some(t) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            t.channel_mask = channel_mask;
        }
    }

    /// Replace a track's audio backend, returning the old one. Inserts,
    /// volume, pan, sends, routing, and meter all stay attached — only the
    /// instrument is swapped. Active notes on the old backend are silenced
    /// before the swap.
    pub fn replace_track_backend(
        &mut self,
        track_id: u32,
        new_backend: Box<dyn AudioBackend>,
        new_source_path: Option<String>,
    ) -> Option<Box<dyn AudioBackend>> {
        let track = self.tracks.iter_mut().find(|t| t.id == track_id)?;
        track.backend.all_notes_off();
        let old = std::mem::replace(&mut track.backend, new_backend);
        track.source_path = new_source_path;
        Some(old)
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
                let next = self
                    .tracks
                    .iter()
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
    pub fn add_send_bus_with_id(
        &mut self,
        id: u32,
        backend: Box<dyn AudioBackend>,
        source_path: Option<String>,
    ) {
        if id >= self.next_bus_id {
            self.next_bus_id = id + 1;
        }
        self.add_send_bus_inner(id, backend, source_path);
    }

    fn add_send_bus_inner(
        &mut self,
        id: u32,
        backend: Box<dyn AudioBackend>,
        source_path: Option<String>,
    ) {
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
    pub fn add_insert_with_id(
        &mut self,
        track_id: u32,
        insert_id: u32,
        backend: Box<dyn AudioBackend>,
        source_path: Option<String>,
    ) {
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
    pub fn remove_insert(
        &mut self,
        track_id: u32,
        insert_id: u32,
    ) -> Option<Box<dyn AudioBackend>> {
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

    /// Get the master bus meter (for main thread level reading).
    pub fn master_meter(&self) -> &LevelMeter {
        &self.master.meter
    }

    /// Get a track's meter by ID.
    pub fn track_meter(&self, track_id: u32) -> Option<&LevelMeter> {
        self.tracks
            .iter()
            .find(|t| t.id == track_id)
            .map(|t| &t.meter)
    }

    /// Clone the master meter handle (Arc-backed, for cross-thread sharing).
    pub fn clone_master_meter(&self) -> LevelMeter {
        self.master.meter.clone()
    }

    /// Clone all track meter handles as (track_id, meter) pairs.
    pub fn clone_track_meters(&self) -> Vec<(u32, LevelMeter)> {
        self.tracks
            .iter()
            .map(|t| (t.id, t.meter.clone()))
            .collect()
    }

    /// Number of tracks currently in the mixer.
    pub fn track_count(&self) -> u32 {
        self.tracks.len() as u32
    }

    /// Enable or disable dithering.
    pub fn set_dither_enabled(&mut self, enabled: bool) {
        self.dither_enabled = enabled;
    }
}
