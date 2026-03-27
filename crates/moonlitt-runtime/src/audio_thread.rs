use crate::event::{AudioEvent, TimedEvent};
use crate::mixer::Mixer;
use crate::sequencer::Sequencer;
use crate::transport::Transport;
use rtrb::Consumer;
use std::sync::Arc;

/// A delayed event waiting to be dispatched at the right sample.
#[derive(Clone, Copy)]
struct PendingEvent {
    event: AudioEvent,
    /// Samples remaining until dispatch. Negative = overdue (fire immediately).
    remaining: i32,
}

/// Holds everything that lives on the audio thread.
/// This struct is moved into the cpal callback closure.
pub(crate) struct AudioThread {
    pub mixer: Mixer,
    pub consumer: Consumer<TimedEvent>,
    pub sequencer: Option<Sequencer>,
    pub transport: Arc<Transport>,
    /// Pre-allocated sequencer event buffer
    pub seq_events: Vec<AudioEvent>,
    /// Delayed events waiting for their sample offset
    pending: Vec<PendingEvent>,
    /// Pre-allocated render buffers (for split rendering)
    render_left: Vec<f32>,
    render_right: Vec<f32>,
}

impl AudioThread {
    pub fn new(
        mixer: Mixer,
        consumer: Consumer<TimedEvent>,
        sequencer: Option<Sequencer>,
        transport: Arc<Transport>,
        buffer_size: usize,
    ) -> Self {
        Self {
            mixer,
            consumer,
            sequencer,
            transport,
            seq_events: Vec::with_capacity(64),
            pending: Vec::with_capacity(1024),
            render_left: vec![0.0; buffer_size],
            render_right: vec![0.0; buffer_size],
        }
    }

    pub fn process(&mut self, output: &mut [f32]) {
        let buffer_size = self.render_left.len();
        let frames_needed = output.len() / 2; // interleaved stereo

        // Process in chunks of buffer_size
        let mut offset = 0;
        while offset < frames_needed {
            let chunk = (frames_needed - offset).min(buffer_size);

            // 1. Drain event queue — separate immediate and delayed
            while let Ok(timed) = self.consumer.pop() {
                if timed.delay_samples == 0 {
                    dispatch_to_mixer(&mut self.mixer, timed.event);
                } else if self.pending.len() < self.pending.capacity() {
                    self.pending.push(PendingEvent {
                        event: timed.event,
                        remaining: timed.delay_samples as i32,
                    });
                }
                // else: drop the delayed event — better than allocating on audio thread
            }

            // 2. Advance sequencer (at chunk boundaries)
            if let Some(ref mut seq) = self.sequencer {
                if self.transport.is_playing() {
                    self.seq_events.clear();
                    seq.advance(
                        chunk,
                        self.mixer.sample_rate(),
                        &mut self.seq_events,
                        self.transport.tempo(),
                        self.transport.looping(),
                    );
                    for i in 0..self.seq_events.len() {
                        let event = self.seq_events[i];
                        dispatch_to_mixer(&mut self.mixer, event);
                    }
                }
            }

            // 3. Render with sample-accurate event insertion
            if self.pending.is_empty() {
                // Fast path: no delayed events
                self.mixer
                    .render(&mut self.render_left[..chunk], &mut self.render_right[..chunk]);
            } else {
                self.render_with_splits(chunk);
            }

            // 4. Interleave into output
            for i in 0..chunk {
                output[(offset + i) * 2] = self.render_left[i];
                output[(offset + i) * 2 + 1] = self.render_right[i];
            }

            offset += chunk;
        }
    }

    /// Render a chunk with sample-accurate event insertion.
    fn render_with_splits(&mut self, chunk: usize) {
        self.pending.sort_by_key(|e| e.remaining);

        let mut rendered = 0usize;
        let mut dispatched = 0usize;

        while dispatched < self.pending.len() {
            let sample_pos = self.pending[dispatched].remaining.max(0) as usize;
            if sample_pos >= chunk {
                break;
            }

            // Render up to this event's position
            if sample_pos > rendered {
                self.mixer.render(
                    &mut self.render_left[rendered..sample_pos],
                    &mut self.render_right[rendered..sample_pos],
                );
                rendered = sample_pos;
            }

            // Dispatch all events at this same sample position
            while dispatched < self.pending.len()
                && (self.pending[dispatched].remaining.max(0) as usize) == sample_pos
            {
                dispatch_to_mixer(&mut self.mixer, self.pending[dispatched].event);
                dispatched += 1;
            }
        }

        // Remove dispatched events
        if dispatched > 0 {
            self.pending.drain(0..dispatched);
        }

        // Render remaining samples
        if rendered < chunk {
            self.mixer.render(
                &mut self.render_left[rendered..chunk],
                &mut self.render_right[rendered..chunk],
            );
        }

        // Decrement remaining for events not yet dispatched
        for pe in &mut self.pending {
            pe.remaining -= chunk as i32;
        }
    }
}

fn dispatch_to_mixer(mixer: &mut Mixer, event: AudioEvent) {
    match event {
        AudioEvent::NoteOn {
            channel,
            note,
            velocity,
        } => mixer.note_on(channel, note, velocity),
        AudioEvent::NoteOff { channel, note, .. } => mixer.note_off(channel, note),
        AudioEvent::CC { channel, cc, value } => mixer.cc(channel, cc, value),
        AudioEvent::PitchBend { channel, value } => mixer.pitch_bend(channel, value),
        AudioEvent::ProgramChange { channel, program } => mixer.program_change(channel, program),
        AudioEvent::AllNotesOff => mixer.all_notes_off(),
        AudioEvent::SetVolume(v) => mixer.set_volume(v),
        AudioEvent::SetParam { id, value } => mixer.set_param(id, value as f64),
        AudioEvent::MixerTrackVolume { track_id, volume } => {
            if let Some(t) = mixer.track_mut(track_id as u32) { t.volume = volume; }
        }
        AudioEvent::MixerTrackPan { track_id, pan } => {
            if let Some(t) = mixer.track_mut(track_id as u32) { t.pan = pan; }
        }
        AudioEvent::MixerTrackMute { track_id, mute } => {
            if let Some(t) = mixer.track_mut(track_id as u32) { t.mute = mute; }
        }
        AudioEvent::MixerTrackSolo { track_id, solo } => {
            if let Some(t) = mixer.track_mut(track_id as u32) { t.solo = solo; }
        }
        AudioEvent::MixerTrackSend { track_id, bus_id, level } => {
            if let Some(t) = mixer.track_mut(track_id as u32) {
                if (bus_id as usize) < t.send_levels.len() {
                    t.send_levels[bus_id as usize] = level;
                }
            }
        }
        AudioEvent::MixerMasterVolume(v) => mixer.set_master_volume(v),
        AudioEvent::Stop => mixer.all_notes_off(),
    }
}
