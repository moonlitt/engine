//! # moonlitt-sampler
//!
//! Pure Rust SF2 synthesizer with Sinc 72 interpolation.
//! World's first Sinc 72 SF2 sampler.
//!
//! Replaces OxiSynth with a from-scratch implementation
//! where every line of DSP code is ours.

pub mod envelope;
pub mod filter;
mod sample;
mod voice;
pub mod voicepool;

pub use sample::SamplePool;
pub use voice::Voice;
