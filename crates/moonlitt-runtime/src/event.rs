/// Unified event type. All input sources produce the same event.
/// Must be Copy + small for efficient lock-free queue transfer.
#[derive(Debug, Clone, Copy)]
pub enum AudioEvent {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    CC { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: i16 },
    ProgramChange { channel: u8, program: u8 },
    AllNotesOff,
    SetVolume(f32),
    SetParam { id: u32, value: f32 },
    // Mixer control events
    MixerTrackVolume { track_id: u8, volume: f32 },
    MixerTrackPan { track_id: u8, pan: f32 },
    MixerTrackMute { track_id: u8, mute: bool },
    MixerTrackSolo { track_id: u8, solo: bool },
    MixerTrackSend { track_id: u8, bus_id: u8, level: f32 },
    MixerMasterVolume(f32),
    // Insert effect control
    InsertBypass { track_id: u8, insert_id: u8, bypass: bool },
    // Per-track parameter targeting
    SetParamForTrack { track_id: u8, param_id: u16, value: f32 },
    SetInsertParam { track_id: u8, insert_id: u8, param_id: u16, value: f32 },
    // Track routing (0xFF = master, else = group track ID)
    MixerTrackRoute { track_id: u8, target_id: u8 },
    Stop,
}

/// An event with a sample-accurate delay.
/// `delay_samples = 0` means immediate (same as legacy behavior).
/// `delay_samples > 0` means "trigger at this sample offset within the audio buffer."
#[derive(Debug, Clone, Copy)]
pub struct TimedEvent {
    pub event: AudioEvent,
    pub delay_samples: u32,
}

// Ensure TimedEvent stays small for efficient lock-free queue transfer.
// If this assertion fails, review AudioEvent variants for bloat.
const _: () = assert!(
    std::mem::size_of::<TimedEvent>() <= 16,
    "TimedEvent should be <= 16 bytes for cache-friendly ring buffer transfer"
);
