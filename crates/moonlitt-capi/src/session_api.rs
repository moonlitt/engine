//! Session FFI — load a fully-configured Runtime from a JSON session file.
//!
//! This is the one-call entrypoint for hosts like piano-block (Stardew Valley
//! mod) that want to ship a pre-configured audio engine: drop a `.mlsession`
//! file next to the game, call [`moonlitt_session_load_from_file`], and the
//! returned RuntimeHandle has every plug-in loaded, every patch restored, and
//! every sample-streamer warmed up.
//!
//! ## Error reporting
//!
//! Failures return null and stash a human-readable message in thread-local
//! storage; the caller retrieves it via
//! [`moonlitt_session_last_error_message`]. Following the same pattern the
//! engine_api uses — keeps the C surface simple (no error-out parameter).
//!
//! ## Threading
//!
//! Like other moonlitt FFI functions, the session loader must be called from
//! a single thread (the producer side of the audio SPSC ring). Concurrent
//! `moonlitt_session_load_from_file` calls are not supported.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};

use moonlitt_audio_io::Runtime;
use moonlitt_session::persistence::Session;

use crate::runtime_api::RuntimeHandle;

thread_local! {
    /// Per-thread last error message. Cleared on the next successful call.
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(msg: impl Into<String>) {
    let s = msg.into();
    let cstr = CString::new(s).unwrap_or_else(|_| {
        CString::new("error message contained nul byte").expect("static valid")
    });
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(cstr);
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Retrieve the most recent session-loading error message, or null if the
/// previous call succeeded. The returned pointer is valid until the next
/// call to any `moonlitt_session_*` function on this thread.
///
/// Do NOT call `moonlitt_free_string` on this pointer — the buffer is
/// owned by thread-local storage and re-used.
#[no_mangle]
pub extern "C" fn moonlitt_session_last_error_message() -> *const c_char {
    LAST_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(s) => s.as_ptr(),
        None => std::ptr::null(),
    })
}

/// Load a session JSON file and build a Runtime from it. The returned
/// RuntimeHandle is in pre-started state — call
/// `moonlitt_runtime_start` before producing or consuming audio.
///
/// Returns null on failure (file missing, schema mismatch, plug-in not
/// found, state blob rejected, audio device unavailable, etc.).
/// Inspect the reason with [`moonlitt_session_last_error_message`].
#[no_mangle]
pub extern "C" fn moonlitt_session_load_from_file(
    path: *const c_char,
    buffer_size: u32,
) -> *mut RuntimeHandle {
    clear_last_error();

    if path.is_null() {
        set_last_error("session path is null");
        return std::ptr::null_mut();
    }
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("session path is not valid UTF-8");
            return std::ptr::null_mut();
        }
    };

    if buffer_size == 0 {
        set_last_error("buffer_size must be > 0");
        return std::ptr::null_mut();
    }

    let session = match Session::load_from_file(path_str) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("load session: {e}"));
            return std::ptr::null_mut();
        }
    };

    let restored = match session.restore(buffer_size as usize) {
        Ok(r) => r,
        Err(e) => {
            set_last_error(format!("restore session: {e}"));
            return std::ptr::null_mut();
        }
    };

    match Runtime::with_mixer_and_transport(restored.mixer, restored.transport, buffer_size) {
        Ok(runtime) => Box::into_raw(Box::new(RuntimeHandle { runtime })),
        Err(e) => {
            set_last_error(format!("create runtime: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Probe a session file without instantiating a Runtime. Returns 0 if
/// the file parses cleanly and matches the schema version this build
/// expects; nonzero otherwise (`moonlitt_session_last_error_message`
/// has details). Useful for piano-block's startup pre-flight check.
#[no_mangle]
pub extern "C" fn moonlitt_session_validate_file(path: *const c_char) -> c_int {
    clear_last_error();

    if path.is_null() {
        set_last_error("session path is null");
        return 1;
    }
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("session path is not valid UTF-8");
            return 1;
        }
    };
    match Session::load_from_file(path_str) {
        Ok(_) => 0,
        Err(e) => {
            set_last_error(format!("validate session: {e}"));
            1
        }
    }
}
