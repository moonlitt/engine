//! # moonlitt-mixer
//!
//! Audio mixing graph — tracks, send buses, groups, PDC, metering, dithering.
//!
//! Platform-agnostic: no cpal, no midir, no threads.
//! The audio thread drives `Mixer::render()` each callback.

mod channel;
pub mod dither;
mod meter;
pub mod mixer;
mod render;

#[cfg(test)]
mod mixer_tests;

// Re-export main types for convenience.
pub use mixer::{InsertEffect, LevelMeter, MasterBus, Mixer, OutputTarget, SendBus, Track};
