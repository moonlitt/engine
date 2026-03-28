//! # moonlitt-sampler
//!
//! Pure Rust SF2 synthesizer with Sinc 72 interpolation.
//! World's first Sinc 72 SF2 sampler.
//!
//! Replaces OxiSynth with a from-scratch implementation
//! where every line of DSP code is ours.
//!
//! ## Public API
//!
//! Use `backend::SamplerBackend` as the primary entry point.
//! The remaining modules are implementation details exposed
//! for integration tests but should not be relied upon externally.

pub mod backend;

#[doc(hidden)]
pub mod envelope;
#[doc(hidden)]
pub mod filter;
#[doc(hidden)]
pub mod voicepool;

mod sample;
mod voice;

pub use sample::SamplePool;
pub use voice::Voice;
