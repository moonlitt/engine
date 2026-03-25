use crate::event::AudioEvent;
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};

/// A stored event: absolute tick position + AudioEvent.
#[derive(Debug, Clone, Copy)]
struct TimedEvent {
    tick: u64,
    event: AudioEvent,
}

/// Sequencer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeqState {
    Stopped,
    Playing,
    Paused,
}

/// Sample-accurate MIDI sequencer.
///
/// Loads a MIDI file, stores events as a sorted (tick, AudioEvent) list,
/// and emits events at the correct sample positions via `advance()`.
pub struct Sequencer {
    events: Vec<TimedEvent>,
    /// Tempo changes: (tick, microseconds_per_beat)
    tempo_map: Vec<(u64, u32)>,
    ticks_per_beat: u16,
    /// Current position in fractional ticks
    current_tick: f64,
    /// Index of next event to emit
    cursor: usize,
    state: SeqState,
    /// Total ticks in the sequence (for looping)
    total_ticks: u64,
}

impl Sequencer {
    /// Parse MIDI from bytes (for testing and in-memory use).
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let smf = Smf::parse(data).map_err(|e| e.to_string())?;
        Self::from_smf(&smf)
    }

    /// Parse MIDI from a file path.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        Self::from_bytes(&data)
    }

    fn from_smf(smf: &Smf) -> Result<Self, String> {
        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(tpb) => tpb.as_int(),
            midly::Timing::Timecode(_, _) => {
                return Err("SMPTE timecode not supported".into());
            }
        };

        let mut events = Vec::new();
        let mut tempo_map = Vec::new();
        let mut total_ticks: u64 = 0;

        // Default tempo: 120 BPM = 500000 us/beat
        tempo_map.push((0, 500_000));

        for track in &smf.tracks {
            let mut abs_tick: u64 = 0;

            for event in track {
                abs_tick += event.delta.as_int() as u64;

                match event.kind {
                    TrackEventKind::Midi { channel, message } => {
                        let ch = channel.as_int();
                        let audio_event = match message {
                            MidiMessage::NoteOn { key, vel } => {
                                let v = vel.as_int();
                                if v == 0 {
                                    AudioEvent::NoteOff {
                                        channel: ch,
                                        note: key.as_int(),
                                        velocity: 0,
                                    }
                                } else {
                                    AudioEvent::NoteOn {
                                        channel: ch,
                                        note: key.as_int(),
                                        velocity: v,
                                    }
                                }
                            }
                            MidiMessage::NoteOff { key, vel } => AudioEvent::NoteOff {
                                channel: ch,
                                note: key.as_int(),
                                velocity: vel.as_int(),
                            },
                            MidiMessage::Controller { controller, value } => AudioEvent::CC {
                                channel: ch,
                                cc: controller.as_int(),
                                value: value.as_int(),
                            },
                            MidiMessage::PitchBend { bend } => AudioEvent::PitchBend {
                                channel: ch,
                                value: bend.as_int(),
                            },
                            MidiMessage::ProgramChange { program } => AudioEvent::ProgramChange {
                                channel: ch,
                                program: program.as_int(),
                            },
                            // Ignore aftertouch, channel aftertouch
                            _ => continue,
                        };

                        events.push(TimedEvent {
                            tick: abs_tick,
                            event: audio_event,
                        });
                    }
                    TrackEventKind::Meta(MetaMessage::Tempo(us_per_beat)) => {
                        tempo_map.push((abs_tick, us_per_beat.as_int()));
                    }
                    _ => {}
                }
            }

            total_ticks = total_ticks.max(abs_tick);
        }

        // Sort events by tick (stable sort preserves order for same-tick events)
        events.sort_by_key(|e| e.tick);

        // Sort tempo map by tick
        tempo_map.sort_by_key(|&(tick, _)| tick);
        // Deduplicate: keep last tempo for each tick
        tempo_map.dedup_by_key(|entry| entry.0);

        Ok(Self {
            events,
            tempo_map,
            ticks_per_beat,
            current_tick: 0.0,
            cursor: 0,
            state: SeqState::Stopped,
            total_ticks,
        })
    }

    /// Start or resume playback.
    pub fn play(&mut self) {
        self.state = SeqState::Playing;
    }

    /// Pause playback (position preserved).
    pub fn pause(&mut self) {
        self.state = SeqState::Paused;
    }

    /// Stop playback and reset to beginning.
    pub fn stop(&mut self) {
        self.state = SeqState::Stopped;
        self.current_tick = 0.0;
        self.cursor = 0;
    }

    /// Get current tempo in microseconds per beat at the given tick.
    fn us_per_beat_at(&self, tick: f64) -> u32 {
        let tick_u64 = tick as u64;
        let mut us = self.tempo_map[0].1;
        for &(t, tempo) in &self.tempo_map {
            if t <= tick_u64 {
                us = tempo;
            } else {
                break;
            }
        }
        us
    }

    /// Advance the sequencer by `samples` samples at `sample_rate`.
    /// Emits due events into `output`.
    pub fn advance(&mut self, samples: usize, sample_rate: u32, output: &mut Vec<AudioEvent>) {
        if self.state != SeqState::Playing {
            return;
        }

        // Calculate ticks to advance:
        // ticks = samples * ticks_per_beat * 1_000_000 / (sample_rate * us_per_beat)
        let us_per_beat = self.us_per_beat_at(self.current_tick) as f64;
        let ticks_per_sample =
            (self.ticks_per_beat as f64) * 1_000_000.0 / (sample_rate as f64 * us_per_beat);
        let ticks_elapsed = samples as f64 * ticks_per_sample;

        let new_tick = self.current_tick + ticks_elapsed;

        // Emit all events in [current_tick, new_tick)
        while self.cursor < self.events.len() {
            let ev = &self.events[self.cursor];
            if (ev.tick as f64) < new_tick {
                output.push(ev.event);
                self.cursor += 1;
            } else {
                break;
            }
        }

        self.current_tick = new_tick;
    }
}
