//! # moonlitt-eq
//!
//! 8-band parametric EQ built on Audio EQ Cookbook biquad filters.
//! Implements `AudioBackend` from `moonlitt-core` for integration
//! into the moonlitt mixer as an insert/send effect.

pub mod biquad;
pub mod eq;

pub use biquad::{Biquad, BiquadCoeffs, FilterType};
pub use eq::{Band, ParametricEq};
