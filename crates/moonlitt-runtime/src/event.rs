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
