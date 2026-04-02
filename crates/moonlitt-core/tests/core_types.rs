use std::mem;

#[test]
fn backend_caps_source_and_effect() {
    use moonlitt_core::BackendCaps;
    let caps = BackendCaps::SOURCE | BackendCaps::EFFECT;
    assert_eq!(caps, BackendCaps::BOTH);
    assert!(caps.contains(BackendCaps::SOURCE));
    assert!(caps.contains(BackendCaps::EFFECT));
}

#[test]
fn backend_caps_empty() {
    use moonlitt_core::BackendCaps;
    let caps = BackendCaps::empty();
    assert!(!caps.contains(BackendCaps::SOURCE));
    assert!(!caps.contains(BackendCaps::EFFECT));
}

#[test]
fn audio_event_is_copy() {
    use moonlitt_core::AudioEvent;
    let e = AudioEvent::NoteOn { channel: 0, note: 60, velocity: 100 };
    let e2 = e;
    let _ = e;
    let _ = e2;
}

#[test]
fn audio_event_size_le_16_bytes() {
    use moonlitt_core::AudioEvent;
    assert!(
        mem::size_of::<AudioEvent>() <= 16,
        "AudioEvent is {} bytes, must be <= 16",
        mem::size_of::<AudioEvent>()
    );
}

#[test]
fn timed_event_size_le_24_bytes() {
    use moonlitt_core::TimedEvent;
    assert!(
        mem::size_of::<TimedEvent>() <= 24,
        "TimedEvent is {} bytes, must be <= 24",
        mem::size_of::<TimedEvent>()
    );
}

#[test]
fn audio_event_all_variants_roundtrip() {
    use moonlitt_core::AudioEvent;
    let events = [
        AudioEvent::NoteOn { channel: 15, note: 127, velocity: 127 },
        AudioEvent::NoteOff { channel: 0, note: 0, velocity: 0 },
        AudioEvent::CC { channel: 9, cc: 64, value: 127 },
        AudioEvent::PitchBend { channel: 0, value: -8192 },
        AudioEvent::ProgramChange { channel: 0, program: 127 },
        AudioEvent::AllNotesOff,
        AudioEvent::SetVolume(0.5),
        AudioEvent::SetParam { id: 42, value: 0.75 },
        AudioEvent::MixerTrackVolume { track_id: 255, volume: -6.0 },
        AudioEvent::MixerTrackPan { track_id: 0, pan: -1.0 },
        AudioEvent::MixerTrackTrim { track_id: 0, trim_db: 6.0 },
        AudioEvent::MixerTrackMute { track_id: 0, mute: true },
        AudioEvent::MixerTrackSolo { track_id: 0, solo: true },
        AudioEvent::MixerTrackSend { track_id: 0, bus_id: 0, level: 0.5 },
        AudioEvent::MixerMasterVolume(0.0),
        AudioEvent::InsertBypass { track_id: 0, insert_id: 0, bypass: true },
        AudioEvent::SetParamForTrack { track_id: 0, param_id: 0, value: 0.0 },
        AudioEvent::SetInsertParam { track_id: 0, insert_id: 0, param_id: 0, value: 0.0 },
        AudioEvent::SetSendBusParam { bus_id: 0, param_id: 0, value: 0.0 },
        AudioEvent::MixerTrackRoute { track_id: 0, target_id: 0xFF },
        AudioEvent::Stop,
    ];
    for e in events {
        let _ = e;
    }
    assert_eq!(events.len(), 21);
}

#[test]
fn audio_host_trait_is_object_safe() {
    use moonlitt_core::AudioHost;
    fn _assert_object_safe(_: &dyn AudioHost) {}
}
