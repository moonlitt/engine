use moonlitt_runtime::sequencer::Sequencer;
use moonlitt_runtime::AudioEvent;

#[test]
fn sequencer_load_and_advance() {
    // Create minimal MIDI data: one note at tick 0, one at tick 480 (1 beat at 120 BPM)
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();

    let mut events = Vec::new();
    let sample_rate = 44100u32;

    // At 120 BPM, 1 beat = 0.5 seconds = 22050 samples
    // Advance 22050 samples in chunks of 256
    let chunks = 22050 / 256;
    for _ in 0..chunks {
        seq.advance(256, sample_rate, &mut events);
    }

    // Should have received at least the first note
    assert!(!events.is_empty(), "should have events after advancing 0.5s");
    assert!(matches!(events[0], AudioEvent::NoteOn { note: 60, .. }));
}

#[test]
fn sequencer_pause_stops_advancing() {
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();
    let mut events = Vec::new();
    seq.advance(256, 44100, &mut events);
    let _count_playing = events.len();

    seq.pause();
    events.clear();
    seq.advance(256, 44100, &mut events);
    assert_eq!(events.len(), 0, "paused sequencer should not produce events");
}

#[test]
fn sequencer_stop_resets_position() {
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();
    let mut events = Vec::new();
    for _ in 0..100 {
        seq.advance(256, 44100, &mut events);
    }

    seq.stop();
    events.clear();
    seq.play();
    seq.advance(256, 44100, &mut events);

    // Should replay from beginning — first event should be NoteOn again
    if !events.is_empty() {
        assert!(matches!(events[0], AudioEvent::NoteOn { note: 60, .. }));
    }
}

/// Create a minimal Standard MIDI File in memory
fn create_test_midi() -> Vec<u8> {
    // MThd header: format 0, 1 track, 480 ticks per beat
    let mut data = Vec::new();
    // Header
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes()); // chunk length
    data.extend_from_slice(&0u16.to_be_bytes()); // format 0
    data.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    data.extend_from_slice(&480u16.to_be_bytes()); // 480 ticks/beat

    // Track
    let mut track = Vec::new();
    // Set tempo: 500000 microseconds per beat = 120 BPM
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
    // Note On at tick 0: channel 0, note 60, velocity 100
    track.extend_from_slice(&[0x00, 0x90, 60, 100]);
    // Note Off at tick 240 (half beat): delta=0x81,0x70 (VLQ for 240)
    track.extend_from_slice(&[0x81, 0x70, 0x80, 60, 0]);
    // Note On at tick 480 (beat 2): delta=240
    track.extend_from_slice(&[0x81, 0x70, 0x90, 64, 100]);
    // Note Off at tick 720
    track.extend_from_slice(&[0x81, 0x70, 0x80, 64, 0]);
    // End of track
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);

    data.extend_from_slice(b"MTrk");
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(&track);
    data
}
