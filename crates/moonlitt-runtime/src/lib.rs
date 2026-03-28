//! # moonlitt-runtime
//!
//! Real-time audio runtime for moonlitt.
//! Connects Engine to audio hardware, MIDI keyboards, and MIDI file playback.
//!
//! All input sources feed a single lock-free event queue.
//! The audio thread drains events and renders audio.

mod audio_output;
mod audio_thread;
pub mod dither;
mod event;
mod midi_input;
pub mod mixer;
mod runtime;
pub mod sequencer;
pub mod session;
pub mod transport;

pub use event::{AudioEvent, TimedEvent};
pub use midi_input::MidiDeviceInfo;
pub use runtime::Runtime;
pub use session::Session;
