//! # moonlitt-mixer
//!
//! Audio mixing graph — tracks, send buses, groups, PDC, metering, dithering.
//!
//! Platform-agnostic: no cpal, no midir, no threads.
//! The audio thread drives `Mixer::render()` each callback.

pub mod dither;
pub mod mixer;

// Re-export main types for convenience.
pub use mixer::{InsertEffect, LevelMeter, MasterBus, Mixer, OutputTarget, SendBus, Track};
