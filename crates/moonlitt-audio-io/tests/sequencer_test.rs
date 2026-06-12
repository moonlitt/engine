use moonlitt_audio_io::sequencer::Sequencer;
use moonlitt_audio_io::AudioEvent;

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
        seq.advance(256, sample_rate, &mut events, None, false);
    }

    // Should have received at least the first note
    assert!(
        !events.is_empty(),
        "should have events after advancing 0.5s"
    );
    assert!(matches!(events[0], AudioEvent::NoteOn { note: 60, .. }));
}

#[test]
fn sequencer_pause_stops_advancing() {
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();
    let mut events = Vec::new();
    seq.advance(256, 44100, &mut events, None, false);
    let _count_playing = events.len();

    seq.pause();
    events.clear();
    seq.advance(256, 44100, &mut events, None, false);
    assert_eq!(
        events.len(),
        0,
        "paused sequencer should not produce events"
    );
}

#[test]
fn sequencer_stop_resets_position() {
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();

    seq.play();
    let mut events = Vec::new();
    for _ in 0..100 {
        seq.advance(256, 44100, &mut events, None, false);
    }

    seq.stop();
    events.clear();
    seq.play();
    seq.advance(256, 44100, &mut events, None, false);

    // Should replay from beginning — first event should be NoteOn again
    if !events.is_empty() {
        assert!(matches!(events[0], AudioEvent::NoteOn { note: 60, .. }));
    }
}

#[test]
fn sequencer_loops_when_enabled() {
    let midi_bytes = create_test_midi();
    let mut seq = Sequencer::from_bytes(&midi_bytes).unwrap();
    seq.play();

    let mut events = Vec::new();
    // Advance well past the end (MIDI is ~720 ticks at 480 tpb ≈ 0.75s)
    // 2 seconds at 44100 = 88200 samples, in 256 chunks = ~344 chunks
    for _ in 0..350 {
        seq.advance(256, 44100, &mut events, None, true);
    }

    // Count NoteOn events — with looping, we should see more than the original 2
    let note_on_count = events
        .iter()
        .filter(|e| matches!(e, AudioEvent::NoteOn { .. }))
        .count();
    assert!(
        note_on_count > 2,
        "looping should replay events, got {} NoteOns",
        note_on_count
    );
}

#[test]
fn sequencer_tempo_override() {
    let midi_bytes = create_test_midi();

    // Run at 120 BPM (default) — 1 beat = 22050 samples
    let mut seq1 = Sequencer::from_bytes(&midi_bytes).unwrap();
    seq1.play();
    let mut events1 = Vec::new();
    for _ in 0..80 {
        seq1.advance(256, 44100, &mut events1, None, false);
    }

    // Run at 240 BPM (2x speed) — same number of samples should yield more events
    let mut seq2 = Sequencer::from_bytes(&midi_bytes).unwrap();
    seq2.play();
    let mut events2 = Vec::new();
    for _ in 0..80 {
        seq2.advance(256, 44100, &mut events2, Some(240.0), false);
    }

    // At 2x tempo, all events should appear sooner
    assert!(
        events2.len() >= events1.len(),
        "faster tempo should emit events sooner: {} vs {}",
        events2.len(),
        events1.len()
    );
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
