//! Mixer — combines multiple engine tracks with send buses and master output.
//!
//! All rendering happens in the audio thread. No locks, no allocations.

use moonlitt_engine::engine::Engine;

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
    // Pre-allocated render buffers
    left: Vec<f32>,
    right: Vec<f32>,
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
}

impl Mixer {
    pub fn new(sample_rate: u32, buffer_size: usize) -> Self {
        Self {
            tracks: Vec::new(),
            send_buses: Vec::new(),
            master: MasterBus {
                volume: 1.0,
                limiter_threshold: 0.95,
                left: vec![0.0; buffer_size],
                right: vec![0.0; buffer_size],
            },
            buffer_size,
            sample_rate,
            next_track_id: 0,
            next_bus_id: 0,
        }
    }

    /// Add a track with an engine and a channel mask. Returns the track ID.
    pub fn add_track(&mut self, engine: Engine, channel_mask: u16) -> u32 {
        let id = self.next_track_id;
        self.next_track_id += 1;
        self.tracks.push(Track {
            id,
            engine,
            channel_mask,
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            send_levels: vec![0.0; self.send_buses.len()],
            left: vec![0.0; self.buffer_size],
            right: vec![0.0; self.buffer_size],
        });
        id
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
        id
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

    pub fn master(&self) -> &MasterBus {
        &self.master
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master.volume = volume;
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
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

    pub fn set_param(&mut self, id: u32, value: f64) {
        // Route to all tracks (params are engine-specific, caller should target)
        for track in &mut self.tracks {
            track.engine.set_param(id, value);
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
    }
}

/// Constant-power pan law.
fn apply_pan(left: &mut [f32], right: &mut [f32], pan: f32) {
    if pan == 0.0 {
        return; // center, no change
    }
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
fn soft_limit(sample: f32, threshold: f32) -> f32 {
    let abs = sample.abs();
    if abs <= threshold {
        sample
    } else {
        let sign = sample.signum();
        let excess = (abs - threshold) / (1.0 - threshold);
        sign * (threshold + (1.0 - threshold) * excess.tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pan_center_is_identity() {
        let mut l = vec![1.0, 0.5, -0.3];
        let mut r = vec![1.0, 0.5, -0.3];
        let l_orig = l.clone();
        let r_orig = r.clone();
        apply_pan(&mut l, &mut r, 0.0);
        assert_eq!(l, l_orig);
        assert_eq!(r, r_orig);
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
}
