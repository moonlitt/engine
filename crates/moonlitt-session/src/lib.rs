//! # moonlitt-session
//!
//! Platform-agnostic audio session — transport, sequencer, event dispatch, persistence.
//!
//! No cpal, no midir — pure scheduling and rendering logic.
//! The runtime crate provides platform I/O and re-exports these modules.

pub mod persistence;
pub mod processor;
pub mod sequencer;
pub mod transport;

pub use persistence::Session;
pub use processor::{AudioThread, MixerCommand};
pub use sequencer::Sequencer;
pub use transport::{Transport, TransportState};
