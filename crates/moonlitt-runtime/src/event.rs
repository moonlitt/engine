/// Placeholder — implemented in Task 2
#[derive(Debug, Clone, Copy)]
pub enum AudioEvent {
    NoteOn { channel: u8, note: u8, velocity: u8 },
}
