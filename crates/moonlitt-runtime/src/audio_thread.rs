use crate::event::{AudioEvent, TimedEvent};
use crate::sequencer::Sequencer;
use crate::transport::Transport;
use moonlitt_engine::engine::Engine;
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
    pub engine: Engine,
    pub consumer: Consumer<TimedEvent>,
    pub sequencer: Option<Sequencer>,
    pub transport: Arc<Transport>,
    /// Pre-allocated render buffers
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Pre-allocated sequencer event buffer
    pub seq_events: Vec<AudioEvent>,
    /// Delayed events waiting for their sample offset
    pending: Vec<PendingEvent>,
}

impl AudioThread {
    pub fn new(
        engine: Engine,
        consumer: Consumer<TimedEvent>,
        sequencer: Option<Sequencer>,
        transport: Arc<Transport>,
        buffer_size: usize,
    ) -> Self {
        Self {
            engine,
            consumer,
            sequencer,
            transport,
            left: vec![0.0; buffer_size],
            right: vec![0.0; buffer_size],
            seq_events: Vec::with_capacity(64),
            pending: Vec::with_capacity(128),
        }
    }

    pub fn process(&mut self, output: &mut [f32]) {
        let buffer_size = self.left.len();
        let frames_needed = output.len() / 2; // interleaved stereo

        // Process in chunks of buffer_size
        let mut offset = 0;
        while offset < frames_needed {
            let chunk = (frames_needed - offset).min(buffer_size);

            // 1. Drain event queue — separate immediate and delayed
            while let Ok(timed) = self.consumer.pop() {
                if timed.delay_samples == 0 {
                    dispatch_to_engine(&mut self.engine, timed.event);
                } else {
                    self.pending.push(PendingEvent {
                        event: timed.event,
                        remaining: timed.delay_samples as i32,
                    });
                }
            }

            // 2. Advance sequencer (at chunk boundaries)
            if let Some(ref mut seq) = self.sequencer {
                if self.transport.is_playing() {
                    self.seq_events.clear();
                    seq.advance(
                        chunk,
                        self.engine.sample_rate(),
                        &mut self.seq_events,
                        self.transport.tempo(),
                        self.transport.looping(),
                    );
                    for i in 0..self.seq_events.len() {
                        let event = self.seq_events[i];
                        dispatch_to_engine(&mut self.engine, event);
                    }
                }
            }

            // 3. Render with sample-accurate event insertion
            self.left[..chunk].fill(0.0);
            self.right[..chunk].fill(0.0);

            if self.pending.is_empty() {
                // Fast path: no delayed events, render whole chunk
                self.engine
                    .render(&mut self.left[..chunk], &mut self.right[..chunk]);
            } else {
                self.render_with_splits(chunk);
            }

            // 4. Interleave into output
            for i in 0..chunk {
                output[(offset + i) * 2] = self.left[i];
                output[(offset + i) * 2 + 1] = self.right[i];
            }

            offset += chunk;
        }
    }

    /// Render a chunk with sample-accurate event insertion.
    /// Splits the chunk at each pending event's sample position,
    /// dispatching the event between sub-renders.
    fn render_with_splits(&mut self, chunk: usize) {
        // Sort pending by remaining samples (ascending)
        self.pending.sort_by_key(|e| e.remaining);

        let mut rendered = 0usize;
        let mut dispatched = 0usize;

        while dispatched < self.pending.len() {
            let sample_pos = self.pending[dispatched].remaining.max(0) as usize;
            if sample_pos >= chunk {
                break; // no more events in this chunk
            }

            // Render up to this event's position
            if sample_pos > rendered {
                self.engine.render(
                    &mut self.left[rendered..sample_pos],
                    &mut self.right[rendered..sample_pos],
                );
                rendered = sample_pos;
            }

            // Dispatch all events at this same sample position
            while dispatched < self.pending.len()
                && (self.pending[dispatched].remaining.max(0) as usize) == sample_pos
            {
                dispatch_to_engine(&mut self.engine, self.pending[dispatched].event);
                dispatched += 1;
            }
        }

        // Remove dispatched events
        if dispatched > 0 {
            self.pending.drain(0..dispatched);
        }

        // Render remaining samples
        if rendered < chunk {
            self.engine.render(
                &mut self.left[rendered..chunk],
                &mut self.right[rendered..chunk],
            );
        }

        // Decrement remaining for events not yet dispatched (carry over to next chunk)
        for pe in &mut self.pending {
            pe.remaining -= chunk as i32;
        }
    }
}

fn dispatch_to_engine(engine: &mut Engine, event: AudioEvent) {
    match event {
        AudioEvent::NoteOn {
            channel,
            note,
            velocity,
        } => engine.note_on(channel, note, velocity),
        AudioEvent::NoteOff { channel, note, .. } => engine.note_off(channel, note),
        AudioEvent::CC { channel, cc, value } => engine.cc(channel, cc, value),
        AudioEvent::PitchBend { channel, value } => engine.pitch_bend(channel, value),
        AudioEvent::ProgramChange { channel, program } => engine.program_change(channel, program),
        AudioEvent::AllNotesOff => engine.all_notes_off(),
        AudioEvent::SetVolume(v) => engine.set_volume(v),
        AudioEvent::Stop => engine.all_notes_off(),
    }
}
