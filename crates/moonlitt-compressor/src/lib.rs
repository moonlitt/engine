//! # moonlitt-compressor
//!
//! Dynamics compressor with log-domain gain computation, soft knee,
//! sidechain HPF, and configurable detection mode (Peak / RMS).
//!
//! Implements the `AudioBackend` trait from `moonlitt-core`.

pub mod compressor;
pub mod envelope;

pub use compressor::{Compressor, DetectionMode};
pub use envelope::EnvelopeFollower;
