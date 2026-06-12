use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use moonlitt_core::AudioEvent;

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
///
/// # Note on dual-gating
///
/// When used inside a `Runtime`, the `AudioThread` gates `advance()` calls
/// behind `Transport::is_playing()`. The sequencer's internal `SeqState` is
/// therefore redundant in that context. However, the internal state is
/// retained for standalone usage (e.g., CLI or tests) where no external
/// transport controls advancement. Future cleanup: unify gating so there
/// is a single source of truth.
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
    /// Total ticks in the sequence (used for looping).
    total_ticks: u64,
    /// Optional practice-loop region `[start, end)` in ticks. When set
    /// AND looping is on, playback wraps inside the region instead of
    /// the whole clip.
    loop_region: Option<(f64, f64)>,
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
            loop_region: None,
        })
    }

    /// Set (or clear) the practice-loop region, in ticks. Input is
    /// sanitised: clamped to the clip, rejected when inverted or
    /// degenerate after clamping.
    pub fn set_loop_region(&mut self, region: Option<(f64, f64)>) {
        self.loop_region = region.and_then(|(start, end)| {
            let start = start.clamp(0.0, self.total_ticks as f64);
            let end = end.clamp(0.0, self.total_ticks as f64);
            (start < end).then_some((start, end))
        });
    }

    /// The active practice-loop region, if any.
    pub fn loop_region(&self) -> Option<(f64, f64)> {
        self.loop_region
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

    /// Whether the playhead has passed the last event of the clip.
    /// (Only meaningful when not looping — looping wraps instead.)
    pub fn is_finished(&self) -> bool {
        self.current_tick >= self.total_ticks as f64
    }

    /// Jump the playhead to an absolute tick position (clamped to the
    /// clip length). The event cursor is re-aligned so events before
    /// the target never re-fire and the next event after it does.
    /// Playback state (playing/paused) is unchanged. Callers should
    /// silence sounding notes themselves — the sequencer doesn't know
    /// which notes are held.
    pub fn seek(&mut self, tick: f64) {
        let tick = tick.clamp(0.0, self.total_ticks as f64);
        self.current_tick = tick;
        self.cursor = self.events.partition_point(|e| (e.tick as f64) < tick);
    }

    /// Current playhead position in fractional ticks.
    pub fn position_ticks(&self) -> f64 {
        self.current_tick
    }

    /// Clip length in ticks (the last event's position).
    pub fn total_ticks(&self) -> u64 {
        self.total_ticks
    }

    /// MIDI resolution: ticks per quarter note.
    pub fn ticks_per_beat(&self) -> u16 {
        self.ticks_per_beat
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
    ///
    /// - `tempo_override`: if `Some(bpm)`, overrides the MIDI file's embedded tempo map.
    /// - `looping`: if `true`, loops back to the beginning when reaching the end.
    ///
    /// Emits due events into `output`.
    pub fn advance(
        &mut self,
        samples: usize,
        sample_rate: u32,
        output: &mut Vec<AudioEvent>,
        tempo_override: Option<f64>,
        looping: bool,
    ) {
        if self.state != SeqState::Playing {
            return;
        }

        // Calculate ticks to advance:
        // ticks = samples * ticks_per_beat * 1_000_000 / (sample_rate * us_per_beat)
        let us_per_beat = match tempo_override {
            Some(bpm) => 60_000_000.0 / bpm,
            None => self.us_per_beat_at(self.current_tick) as f64,
        };
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

        // Loop wrap: inside the practice region when one is set, else
        // around the whole clip. Hanging notes are flushed — a region
        // boundary can cut right through held notes.
        if looping {
            let (wrap_start, wrap_end) = self
                .loop_region
                .unwrap_or((0.0, self.total_ticks as f64));
            if self.current_tick >= wrap_end {
                output.push(AudioEvent::AllNotesOff);
                self.current_tick = wrap_start;
                self.cursor = self
                    .events
                    .partition_point(|e| (e.tick as f64) < wrap_start);
            }
        }
    }
}

#[cfg(test)]
mod seek_tests {
    use super::*;

    fn test_midi() -> Vec<u8> {
        // Two notes: tick 0 (note 60) and tick 480 (note 64), 120 BPM.
        let mut track: Vec<u8> = Vec::new();
        track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
        track.extend_from_slice(&[0x00, 0x90, 60, 100]);
        track.extend_from_slice(&[0x81, 0x70, 0x80, 60, 0]); // delta 240
        track.extend_from_slice(&[0x81, 0x70, 0x90, 64, 100]); // tick 480
        track.extend_from_slice(&[0x81, 0x70, 0x80, 64, 0]);
        track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&6u32.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&480u16.to_be_bytes());
        data.extend_from_slice(b"MTrk");
        data.extend_from_slice(&(track.len() as u32).to_be_bytes());
        data.extend_from_slice(&track);
        data
    }

    #[test]
    fn seek_repositions_playhead_and_cursor() {
        let mut seq = Sequencer::from_bytes(&test_midi()).unwrap();
        seq.play();

        // Play past the first note.
        let mut events = Vec::new();
        seq.advance(22050, 44100, &mut events, None, false); // 0.5 s = 1 beat
        assert!(
            events
                .iter()
                .any(|e| matches!(e, moonlitt_core::AudioEvent::NoteOn { note: 60, .. })),
            "first note plays before the seek"
        );

        // Seek back to the very start: the first note must play AGAIN.
        seq.seek(0.0);
        assert_eq!(seq.position_ticks(), 0.0);
        events.clear();
        seq.advance(11025, 44100, &mut events, None, false); // 0.25 s
        assert!(
            events
                .iter()
                .any(|e| matches!(e, moonlitt_core::AudioEvent::NoteOn { note: 60, .. })),
            "replay after seek(0) must re-emit the first note"
        );

        // Seek into the middle: only later events fire (no double-fire of
        // earlier ones, no skipping of the next one).
        seq.seek(400.0);
        events.clear();
        seq.advance(11025, 44100, &mut events, None, false); // 0.25 s = 240 ticks
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, moonlitt_core::AudioEvent::NoteOn { note: 60, .. })),
            "events before the seek target must not re-fire"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, moonlitt_core::AudioEvent::NoteOn { note: 64, .. })),
            "the next event after the seek target must fire"
        );

        // Seek past the end clamps and reports finished.
        seq.seek(1e12);
        assert!(seq.is_finished());

        // Introspection for progress UIs. Last event: note-off at tick
        // 0 + 240 + 240 + 240 = 720.
        assert_eq!(seq.total_ticks(), 720);
        assert_eq!(seq.ticks_per_beat(), 480);
    }

    /// A loop REGION wraps playback inside [start, end): crossing the
    /// end jumps back to the start, flushes hanging notes, and re-emits
    /// the region's events on the next pass. This is the practice-loop
    /// feature — without a region, looping wraps the whole clip.
    #[test]
    fn loop_region_wraps_to_region_start_and_replays() {
        let mut seq = Sequencer::from_bytes(&test_midi()).unwrap();
        seq.set_loop_region(Some((0.0, 480.0)));
        seq.play();

        // Advance 1.25 s = 600 ticks at 120 BPM — crosses the region
        // end (480) once.
        let mut events = Vec::new();
        seq.advance(55125, 44100, &mut events, None, true);
        assert!(
            seq.position_ticks() < 480.0,
            "position must wrap inside the region, got {}",
            seq.position_ticks()
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, moonlitt_core::AudioEvent::AllNotesOff)),
            "wrap must flush hanging notes"
        );

        // Next pass re-emits the region's first note.
        events.clear();
        seq.advance(11025, 44100, &mut events, None, true);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, moonlitt_core::AudioEvent::NoteOn { note: 60, .. })),
            "region restart must re-emit the first note"
        );
    }

    #[test]
    fn loop_without_region_still_wraps_whole_clip() {
        let mut seq = Sequencer::from_bytes(&test_midi()).unwrap();
        seq.play();
        let mut events = Vec::new();
        // 2 s = 960 ticks — crosses total (720).
        seq.advance(88200, 44100, &mut events, None, true);
        assert!(
            seq.position_ticks() < 720.0,
            "whole-clip loop must wrap, got {}",
            seq.position_ticks()
        );
    }

    #[test]
    fn loop_region_sanitises_bad_input() {
        let mut seq = Sequencer::from_bytes(&test_midi()).unwrap();
        // Inverted → rejected.
        seq.set_loop_region(Some((480.0, 240.0)));
        assert_eq!(seq.loop_region(), None);
        // Out of bounds → clamped to the clip.
        seq.set_loop_region(Some((-50.0, 10_000.0)));
        assert_eq!(seq.loop_region(), Some((0.0, 720.0)));
        // Degenerate (start == end after clamping) → rejected.
        seq.set_loop_region(Some((700.0, 700.0)));
        assert_eq!(seq.loop_region(), None);
    }
}
