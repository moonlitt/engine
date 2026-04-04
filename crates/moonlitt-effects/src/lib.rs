//! # moonlitt-effects
//!
//! Built-in audio effects — dynamics, EQ, spatial, modulation, utility.
//!
//! ## Feature flags
//!
//! - `dynamics` — all dynamics processors
//!   - `compressor` — log-domain compressor with soft knee
//!   - `limiter` — brickwall limiter with lookahead
//!   - `gate` — noise gate / expander with hysteresis
//!   - `deesser` — split-band sibilance reduction
//! - `eq` / `parametric-eq` — 8-band parametric EQ (biquad cascade)
//! - `spatial` — reverb + convolution reverb
//!   - `reverb` — Freeverb + Dattorro plate reverb
//!   - `convolver` — FFT partitioned convolution (requires `rustfft`)
//! - `modulation` — time-based modulation effects
//!   - `delay` — stereo delay with tempo sync and ping-pong
//!   - `chorus` — 4-voice chorus with sinc-interpolated delay
//!   - `flanger` — through-zero flanger with soft saturation
//!   - `phaser` — N-stage allpass phaser with LFO sweep
//!   - `tremolo` — tremolo with tempo sync and stereo auto-pan
//! - `utility` — mix helpers
//!   - `gain` — gain, polarity invert, mono sum
//!   - `stereo-width` — mid/side stereo width control

pub mod common;

#[cfg(any(feature = "compressor", feature = "limiter", feature = "gate", feature = "deesser"))]
pub mod dynamics;

// eq::biquad is also used by gate and deesser for sidechain filters
#[cfg(any(feature = "parametric-eq", feature = "gate", feature = "deesser"))]
pub mod eq;

#[cfg(any(feature = "reverb", feature = "convolver"))]
pub mod spatial;

#[cfg(any(
    feature = "delay",
    feature = "chorus",
    feature = "flanger",
    feature = "phaser",
    feature = "tremolo"
))]
pub mod modulation;

#[cfg(any(feature = "gain", feature = "stereo-width"))]
pub mod utility;

// ---- Convenience re-exports ----

// dynamics
#[cfg(feature = "compressor")]
pub use dynamics::compressor::{Compressor, DetectionMode};

#[cfg(feature = "compressor")]
pub use dynamics::envelope::EnvelopeFollower;

#[cfg(feature = "limiter")]
pub use dynamics::limiter::Limiter;

#[cfg(feature = "gate")]
pub use dynamics::gate::Gate;

#[cfg(feature = "deesser")]
pub use dynamics::deesser::DeEsser;

// eq
#[cfg(feature = "parametric-eq")]
pub use eq::parametric::ParametricEq;

#[cfg(feature = "parametric-eq")]
pub use eq::biquad::{Biquad, BiquadCoeffs, FilterType};

#[cfg(feature = "parametric-eq")]
pub use eq::parametric::Band;

// spatial
#[cfg(feature = "reverb")]
pub use spatial::reverb::Reverb;

#[cfg(feature = "reverb")]
pub use spatial::dattorro::DattorroReverb;

#[cfg(feature = "convolver")]
pub use spatial::convolver::Convolver;

// modulation
#[cfg(feature = "delay")]
pub use modulation::delay::StereoDelay;

#[cfg(feature = "chorus")]
pub use modulation::chorus::Chorus;

#[cfg(feature = "flanger")]
pub use modulation::flanger::Flanger;

#[cfg(feature = "phaser")]
pub use modulation::phaser::Phaser;

#[cfg(feature = "tremolo")]
pub use modulation::tremolo::Tremolo;

// utility
#[cfg(feature = "gain")]
pub use utility::gain::Gain;

#[cfg(feature = "stereo-width")]
pub use utility::stereo_width::StereoWidth;
