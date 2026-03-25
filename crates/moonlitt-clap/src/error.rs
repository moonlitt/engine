//! Error types for CLAP plugin hosting.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Plugin bundle not found or failed to load.
    LoadFailed(String),
    /// Plugin returned an error during lifecycle call.
    PluginError(&'static str),
    /// Operation not supported by this plugin.
    NotSupported,
    /// Generic error.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::LoadFailed(msg) => write!(f, "load failed: {msg}"),
            Error::PluginError(op) => write!(f, "plugin error: {op}"),
            Error::NotSupported => write!(f, "not supported"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
