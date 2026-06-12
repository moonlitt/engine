//! Mixer render path — drives all tracks, send buses, and the master bus.
//!
//! Runs on the audio thread: no locks, no allocations.

use crate::channel::{InsertEffect, OutputTarget, Track};
use crate::mixer::Mixer;

impl Mixer {
    // --- Rendering ---

    /// Render one chunk of audio into an interleaved stereo output buffer.
    ///
    /// Two-phase rendering for group track support:
    /// 1. Source tracks render first, routing to master or group accumulators
    /// 2. Group tracks render after sources, consuming accumulated input
    pub fn render(&mut self, output_left: &mut [f32], output_right: &mut [f32]) {
        let chunk = output_left
            .len()
            .min(output_right.len())
            .min(self.buffer_size);

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
            if idx >= self.tracks.len() {
                continue;
            }

            let track = &mut self.tracks[idx];
            let audible = !track.mute && (!any_solo || track.solo);

            // Render engine output
            track.left[..chunk].fill(0.0);
            track.right[..chunk].fill(0.0);
            track
                .backend
                .render(&mut track.left[..chunk], &mut track.right[..chunk]);

            // Add accumulated group input (for group tracks)
            for k in 0..chunk {
                track.left[k] += track.group_in_left[k];
                track.right[k] += track.group_in_right[k];
            }

            // Trim (pre-insert gain staging)
            if track.trim_db != 0.0 {
                let trim_gain = 10f32.powf(track.trim_db / 20.0);
                for s in &mut track.left[..chunk] {
                    *s *= trim_gain;
                }
                for s in &mut track.right[..chunk] {
                    *s *= trim_gain;
                }
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
            track
                .delay_line
                .process(&mut track.left[..chunk], &mut track.right[..chunk]);

            if !audible {
                continue;
            }

            // Volume (fader)
            let vol = track.volume;
            for s in &mut track.left[..chunk] {
                *s *= vol;
            }
            for s in &mut track.right[..chunk] {
                *s *= vol;
            }

            // Pan (constant-power)
            apply_pan(
                &mut track.left[..chunk],
                &mut track.right[..chunk],
                track.pan,
            );

            // Meter (post-fader)
            track
                .meter
                .update(&track.left[..chunk], &track.right[..chunk]);

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
            self.dither
                .process(&mut output_left[..chunk], &mut output_right[..chunk]);
        }

        // Update master meter (post-dither)
        self.master
            .meter
            .update(&output_left[..chunk], &output_right[..chunk]);
    }
}

/// Process insert effect chain using ping-pong buffers.
///
/// Alternates between track buffers (left/right) and scratch buffers to avoid
/// allocation. If the result ends up in scratch, copies back to track buffers.
///
/// Split borrows: `inserts`, `left/right`, and `scratch_left/scratch_right` are
/// disjoint fields of Track, passed separately to satisfy the borrow checker.
pub(crate) fn process_insert_chain(
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
pub(crate) fn apply_pan(left: &mut [f32], right: &mut [f32], pan: f32) {
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
pub(crate) fn soft_limit(sample: f32, threshold: f32) -> f32 {
    let abs = sample.abs();
    if abs <= threshold {
        sample
    } else {
        let sign = sample.signum();
        let excess = (abs - threshold) / (1.0 - threshold);
        (sign * (threshold + (1.0 - threshold) * excess.tanh())).clamp(-1.0, 1.0)
    }
}
