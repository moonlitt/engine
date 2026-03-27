//! # moonlitt-resampler
//!
//! High-quality sinc interpolation for audio resampling.
//! Supports up to 72-point windowed sinc (matching sfizz's maximum quality).
//!
//! Pure Rust, zero dependencies, no unsafe code.
//!
//! ```
//! use moonlitt_resampler::{SincInterpolator, Quality};
//!
//! let interpolator = SincInterpolator::new(Quality::Sinc72);
//! let samples = [0.0f32; 256];
//! let value = interpolator.interpolate(&samples, 100, 0.5); // fractional position
//! ```

mod sinc;
pub mod window;

pub use sinc::{Quality, SincInterpolator};

#[cfg(test)]
mod tests;
