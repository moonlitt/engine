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
    /// The host-side wrapper code panicked while talking to the plug-in.
    /// The plug-in instance should be considered tainted — drop it and
    /// reload if you want to keep going. NOTE: this only catches Rust
    /// panics. C++ segfaults inside the plug-in itself still crash the
    /// process; true crash isolation requires hosting the plug-in in a
    /// separate process (planned for v2).
    PluginPanicked(String),
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
            Error::PluginPanicked(msg) => write!(f, "plug-in wrapper panicked: {msg}"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Run `f` and convert any panic into [`Error::PluginPanicked`] so a host
/// audio thread can keep going. ONLY catches Rust panics — C++ exceptions
/// and segfaults inside the loaded plug-in are not caught (they require
/// subprocess isolation).
///
/// Use this at any FFI-adjacent boundary that touches plug-in code:
/// render, set_state, load_preset, process_effect. Inside the audio
/// thread, the cost of `catch_unwind` is negligible compared to the
/// FFI call itself.
pub(crate) fn catch_plugin_panic<R>(label: &str, f: impl FnOnce() -> Result<R>) -> Result<R> {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&'static str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            Err(Error::PluginPanicked(format!("{label}: {msg}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_plugin_panic_converts_static_str_panic() {
        let r: Result<()> = catch_plugin_panic("render", || panic!("synthetic boom"));
        match r {
            Err(Error::PluginPanicked(msg)) => assert!(
                msg.contains("synthetic boom") && msg.contains("render"),
                "missing context in error message: {msg}"
            ),
            other => panic!("expected PluginPanicked, got {other:?}"),
        }
    }

    #[test]
    fn catch_plugin_panic_converts_string_panic() {
        let r: Result<()> =
            catch_plugin_panic("set_state", || panic!("{}", String::from("dynamic boom")));
        assert!(matches!(r, Err(Error::PluginPanicked(_))));
    }

    #[test]
    fn catch_plugin_panic_passes_ok_through() {
        let r: Result<i32> = catch_plugin_panic("ok", || Ok(42));
        assert!(matches!(r, Ok(42)));
    }

    #[test]
    fn catch_plugin_panic_preserves_inner_err() {
        let r: Result<()> = catch_plugin_panic("err", || Err(Error::NotSupported));
        assert!(matches!(r, Err(Error::NotSupported)));
    }
}
