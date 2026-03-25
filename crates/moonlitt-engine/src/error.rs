//! Engine error types.

use std::fmt;

#[derive(Debug)]
pub enum EngineError {
    /// File format not supported (unknown extension).
    UnsupportedFormat(String),
    /// Error from a backend (SF2, VST3, CLAP).
    BackendError(String),
    /// No backend is currently loaded.
    NoBackendLoaded,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(ext) => write!(f, "unsupported format: {ext}"),
            Self::BackendError(msg) => write!(f, "backend error: {msg}"),
            Self::NoBackendLoaded => write!(f, "no backend loaded"),
        }
    }
}

impl std::error::Error for EngineError {}
