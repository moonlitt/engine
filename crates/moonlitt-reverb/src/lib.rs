//! # moonlitt-reverb
//!
//! Stereo reverb based on the Freeverb algorithm (8 comb + 4 allpass per channel).
//!
//! Implements `AudioBackend` from `moonlitt-core` as a send effect with
//! pre-delay, damping, stereo width, wet EQ, and dry/wet mix.

mod allpass;
mod comb;
mod reverb;

pub use reverb::Reverb;
