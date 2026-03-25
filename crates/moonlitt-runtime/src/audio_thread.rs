use crate::event::AudioEvent;
use crate::sequencer::Sequencer;
use crate::transport::Transport;
use moonlitt_engine::engine::Engine;
use rtrb::Consumer;
use std::sync::Arc;

/// Holds everything that lives on the audio thread.
/// This struct is moved into the cpal callback closure.
pub(crate) struct AudioThread {
    pub engine: Engine,
    pub consumer: Consumer<AudioEvent>,
    pub sequencer: Option<Sequencer>,
    pub transport: Arc<Transport>,
    /// Pre-allocated render buffers
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Pre-allocated sequencer event buffer
    pub seq_events: Vec<AudioEvent>,
}

impl AudioThread {
    pub fn process(&mut self, output: &mut [f32]) {
        let buffer_size = self.left.len();
        let frames_needed = output.len() / 2; // interleaved stereo

        // Process in chunks of buffer_size
        let mut offset = 0;
        while offset < frames_needed {
            let chunk = (frames_needed - offset).min(buffer_size);

            // 1. Drain event queue
            while let Ok(event) = self.consumer.pop() {
                dispatch_to_engine(&mut self.engine, event);
            }

            // 2. Advance sequencer
            if let Some(ref mut seq) = self.sequencer {
                if self.transport.is_playing() {
                    self.seq_events.clear();
                    seq.advance(chunk, self.engine.sample_rate(), &mut self.seq_events);
                    for i in 0..self.seq_events.len() {
                        let event = self.seq_events[i];
                        dispatch_to_engine(&mut self.engine, event);
                    }
                }
            }

            // 3. Render
            self.left[..chunk].fill(0.0);
            self.right[..chunk].fill(0.0);
            self.engine.render(&mut self.left[..chunk], &mut self.right[..chunk]);

            // 4. Interleave into output
            for i in 0..chunk {
                output[(offset + i) * 2] = self.left[i];
                output[(offset + i) * 2 + 1] = self.right[i];
            }

            offset += chunk;
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
