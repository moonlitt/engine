//! # moonlitt-sampler
//!
//! Pure Rust SF2 synthesizer with Sinc 72 interpolation.
//! World's first Sinc 72 SF2 sampler.
//!
//! Replaces OxiSynth with a from-scratch implementation
//! where every line of DSP code is ours.

mod sample;
mod voice;

pub use sample::SamplePool;
pub use voice::Voice;
