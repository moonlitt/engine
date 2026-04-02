//! # moonlitt-runtime
//!
//! Real-time audio runtime for moonlitt.
//! Connects Engine to audio hardware, MIDI keyboards, and MIDI file playback.
//!
//! All input sources feed a single lock-free event queue.
//! The audio thread drains events and renders audio.

mod audio_output;
mod midi_input;
mod runtime;

// Re-export from moonlitt-mixer for backward compatibility.
pub use moonlitt_mixer::dither;
pub use moonlitt_mixer::mixer;

// Re-export from moonlitt-session for backward compatibility.
pub use moonlitt_session::persistence as session;
pub use moonlitt_session::sequencer;
pub use moonlitt_session::transport;

pub use moonlitt_core::{AudioEvent, TimedEvent};
pub use midi_input::MidiDeviceInfo;
pub use moonlitt_session::Session;
pub use runtime::Runtime;
