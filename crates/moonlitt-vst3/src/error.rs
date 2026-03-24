use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Plugin bundle not found or failed to load
    LoadFailed(String),
    /// Plugin does not implement required interface
    InterfaceNotFound(&'static str),
    /// Plugin returned an error code
    PluginError(i32),
    /// Operation not supported by this plugin
    NotSupported,
    /// Generic error
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::LoadFailed(msg) => write!(f, "load failed: {msg}"),
            Error::InterfaceNotFound(iface) => write!(f, "interface not found: {iface}"),
            Error::PluginError(code) => write!(f, "plugin error (code {code})"),
            Error::NotSupported => write!(f, "not supported"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
