//! Error model shared by every moonlitt C API function.
//!
//! Two layers:
//!
//! * **Machine-readable** — every fallible function returns a
//!   [`MoonlittStatus`]: `MOONLITT_OK` (0) on success, a negative
//!   `MOONLITT_ERR_*` class on failure.
//! * **Human-readable** — the most recent *failure* on the calling
//!   thread stores a message, retrievable via
//!   [`moonlitt_last_error_message`]. The message is only meaningful
//!   immediately after a call returned non-OK; it is owned by the
//!   library and valid on the same thread until the next failing call.
//!
//! Every `extern "C"` body is wrapped in [`ffi_guard!`] so a Rust panic
//! can never unwind across the FFI boundary into the host process — it
//! is converted into `MOONLITT_ERR_PANIC` plus a last-error message.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};

/// Status code returned by fallible moonlitt functions.
/// `0` = success; negative values classify the failure.
pub type MoonlittStatus = i32;

/// Success.
pub const MOONLITT_OK: MoonlittStatus = 0;
/// An argument was null, out of range, or otherwise invalid.
pub const MOONLITT_ERR_INVALID_ARG: MoonlittStatus = -1;
/// The operation needs a loaded backend/resource that isn't there
/// (engine without a sound loaded, consumed engine handle, …).
pub const MOONLITT_ERR_NOT_LOADED: MoonlittStatus = -2;
/// The lock-free event queue to the audio thread is full; the event
/// was dropped. Retry on the next tick.
pub const MOONLITT_ERR_QUEUE_FULL: MoonlittStatus = -3;
/// Filesystem or device I/O failed.
pub const MOONLITT_ERR_IO: MoonlittStatus = -4;
/// The hosted plugin (VST3/CLAP) or soundfont rejected the operation.
pub const MOONLITT_ERR_PLUGIN: MoonlittStatus = -5;
/// State blob save/load failed (corrupt, wrong backend, version skew).
pub const MOONLITT_ERR_STATE: MoonlittStatus = -6;
/// An internal panic was caught at the FFI boundary.
pub const MOONLITT_ERR_PANIC: MoonlittStatus = -7;
/// The backend does not support this operation.
pub const MOONLITT_ERR_UNSUPPORTED: MoonlittStatus = -8;

/// ABI version, bumped by hand on every ABI-affecting change.
/// 1.0.0 is the frozen v1 surface: existing signatures and semantics
/// are stable; additions bump MINOR, breaking changes bump MAJOR.
pub const MOONLITT_ABI_MAJOR: u32 = 1;
pub const MOONLITT_ABI_MINOR: u32 = 0;
pub const MOONLITT_ABI_PATCH: u32 = 0;

enum LastError {
    Owned(CString),
    Static(&'static CStr),
}

thread_local! {
    static LAST_ERROR: RefCell<Option<LastError>> = const { RefCell::new(None) };
}

/// Record a failure message for the calling thread.
pub(crate) fn set_last_error(msg: impl std::fmt::Display) {
    let text = msg.to_string();
    let c = CString::new(text)
        .unwrap_or_else(|_| CString::new("error message contained NUL").expect("static"));
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(LastError::Owned(c)));
}

/// Allocation-free variant for hot paths (e.g. event-queue overflow).
pub(crate) fn set_last_error_static(msg: &'static CStr) {
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(LastError::Static(msg)));
}

/// Human-readable detail of the most recent failure on the calling
/// thread, or NULL if nothing failed yet.
///
/// Ownership: borrowed from the library — do NOT free. Valid on the
/// calling thread until the next failing moonlitt call. Only meaningful
/// immediately after a call returned non-OK / NULL.
#[no_mangle]
pub extern "C" fn moonlitt_last_error_message() -> *const c_char {
    LAST_ERROR.with(|e| match &*e.borrow() {
        Some(LastError::Owned(c)) => c.as_ptr(),
        Some(LastError::Static(c)) => c.as_ptr(),
        None => std::ptr::null(),
    })
}

/// Packed ABI version: `(major << 16) | (minor << 8) | patch`.
///
/// Bindings should check this at load time and refuse to run against an
/// incompatible major version.
#[no_mangle]
pub extern "C" fn moonlitt_abi_version() -> u32 {
    (MOONLITT_ABI_MAJOR << 16) | (MOONLITT_ABI_MINOR << 8) | MOONLITT_ABI_PATCH
}

/// Wrap an `extern "C"` body so a Rust panic cannot unwind into the
/// host process. On panic: record a last-error message and return the
/// given fallback value (typically `MOONLITT_ERR_PANIC`, NULL, 0.0, …).
macro_rules! ffi_guard {
    ($on_panic:expr, $body:expr) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body)) {
            Ok(v) => v,
            Err(payload) => {
                let detail: &str = if let Some(s) = payload.downcast_ref::<&str>() {
                    s
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.as_str()
                } else {
                    "opaque panic payload"
                };
                $crate::error::set_last_error(format!("internal panic: {detail}"));
                $on_panic
            }
        }
    };
}
pub(crate) use ffi_guard;

/// Diagnostics: deliberately panic inside the library so bindings can
/// verify the panic guard end-to-end (the call must return
/// `MOONLITT_ERR_PANIC` — never crash the process).
#[no_mangle]
pub extern "C" fn moonlitt_debug_trigger_panic() -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        panic!("moonlitt_debug_trigger_panic: intentional test panic");
        #[allow(unreachable_code)]
        MOONLITT_OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_and_owned_messages_roundtrip() {
        set_last_error("owned message");
        let p = moonlitt_last_error_message();
        assert_eq!(
            unsafe { CStr::from_ptr(p) }.to_str().unwrap(),
            "owned message"
        );

        set_last_error_static(c"static message");
        let p = moonlitt_last_error_message();
        assert_eq!(
            unsafe { CStr::from_ptr(p) }.to_str().unwrap(),
            "static message"
        );
    }

    #[test]
    fn nul_bytes_in_message_are_survivable() {
        set_last_error("bad\0message");
        let p = moonlitt_last_error_message();
        assert!(!p.is_null());
    }
}
