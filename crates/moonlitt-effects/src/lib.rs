//! # moonlitt-effects
//!
//! Built-in audio effects — dynamics, EQ, spatial processing.
//!
//! ## Feature flags
//!
//! - `dynamics` / `compressor` — dynamics compressor with log-domain gain
//! - `eq` / `parametric-eq` — 8-band parametric EQ (biquad cascade)
//! - `spatial` — reverb + convolution reverb
//!   - `reverb` — Freeverb + Dattorro plate reverb
//!   - `convolver` — FFT partitioned convolution (requires `rustfft`)

#[cfg(feature = "compressor")]
pub mod dynamics;

#[cfg(feature = "parametric-eq")]
pub mod eq;

#[cfg(any(feature = "reverb", feature = "convolver"))]
pub mod spatial;

// Convenience re-exports
#[cfg(feature = "compressor")]
pub use dynamics::compressor::{Compressor, DetectionMode};

#[cfg(feature = "compressor")]
pub use dynamics::envelope::EnvelopeFollower;

#[cfg(feature = "parametric-eq")]
pub use eq::parametric::ParametricEq;

#[cfg(feature = "parametric-eq")]
pub use eq::biquad::{Biquad, BiquadCoeffs, FilterType};

#[cfg(feature = "parametric-eq")]
pub use eq::parametric::Band;

#[cfg(feature = "reverb")]
pub use spatial::reverb::Reverb;

#[cfg(feature = "reverb")]
pub use spatial::dattorro::DattorroReverb;

#[cfg(feature = "convolver")]
pub use spatial::convolver::Convolver;
