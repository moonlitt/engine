//! Backend implementations.

#[cfg(feature = "sf2")]
pub mod oxisynth;

#[cfg(feature = "sf2-legacy")]
pub mod sf2;

#[cfg(feature = "vst3")]
pub mod vst3;

#[cfg(feature = "clap")]
pub mod clap;
