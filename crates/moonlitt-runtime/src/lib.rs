//! # moonlitt-runtime
//!
//! Real-time audio runtime for moonlitt.
//! Connects Engine to audio hardware, MIDI keyboards, and MIDI file playback.
//!
//! All input sources feed a single lock-free event queue.
//! The audio thread drains events and renders audio.

mod audio_output;
mod audio_thread;
mod event;
mod midi_input;
mod runtime;
pub mod sequencer;
pub mod transport;

pub use event::AudioEvent;
pub use midi_input::MidiDeviceInfo;
pub use runtime::Runtime;
