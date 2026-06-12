//! Session FFI — load a fully-configured Runtime from a JSON session file.
//!
//! This is the one-call entrypoint for hosts like piano-block (Stardew
//! Valley mod) that want to ship a pre-configured audio engine: drop a
//! `.mlsession` file next to the game, call
//! [`moonlitt_session_load_from_file`], and the returned RuntimeHandle
//! has every plug-in loaded, every patch restored, and every
//! sample-streamer warmed up.
//!
//! Error reporting and threading follow the crate-wide conventions
//! (`MoonlittStatus` + `moonlitt_last_error_message()`; single control
//! thread).

use std::ffi::c_char;

use moonlitt_audio_io::Runtime;
use moonlitt_session::persistence::Session;

use crate::error::{
    ffi_guard, set_last_error, set_last_error_static, MoonlittStatus, MOONLITT_ERR_INVALID_ARG,
    MOONLITT_ERR_PANIC, MOONLITT_ERR_STATE, MOONLITT_OK,
};
use crate::runtime_api::RuntimeHandle;
use crate::util::{cstr_to_str, to_c_string};

/// Load a session JSON file and build a Runtime from it. The returned
/// RuntimeHandle is in pre-started state — call
/// `moonlitt_runtime_start_audio` before expecting sound.
///
/// Ownership: returns an owned RuntimeHandle* (free with
/// `moonlitt_runtime_destroy`), or NULL on failure (file missing,
/// schema mismatch, plug-in not found, state blob rejected, audio
/// device unavailable, …) with the reason in
/// `moonlitt_last_error_message()`.
#[no_mangle]
pub extern "C" fn moonlitt_session_load_from_file(
    path: *const c_char,
    buffer_size: u32,
) -> *mut RuntimeHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let path_str = match unsafe { cstr_to_str(path) } {
            Some(s) => s,
            None => {
                set_last_error_static(c"session path is NULL or not valid UTF-8");
                return std::ptr::null_mut();
            }
        };
        if buffer_size == 0 {
            set_last_error_static(c"buffer_size must be > 0");
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
    })
}

/// Probe a session file without instantiating a Runtime. Returns
/// `MOONLITT_OK` when the file parses cleanly and matches the schema
/// version this build expects; `MOONLITT_ERR_STATE` (or
/// `MOONLITT_ERR_INVALID_ARG` for a bad path) otherwise. Useful as a
/// startup pre-flight check.
#[no_mangle]
pub extern "C" fn moonlitt_session_validate_file(path: *const c_char) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        let path_str = match unsafe { cstr_to_str(path) } {
            Some(s) => s,
            None => {
                set_last_error_static(c"session path is NULL or not valid UTF-8");
                return MOONLITT_ERR_INVALID_ARG;
            }
        };
        match Session::load_from_file(path_str) {
            Ok(_) => MOONLITT_OK,
            Err(e) => {
                set_last_error(format!("validate session: {e}"));
                MOONLITT_ERR_STATE
            }
        }
    })
}

/// Read a session file and return its canonical JSON text (useful for
/// inspection/debugging without building a runtime).
///
/// Ownership: caller frees with `moonlitt_free_string`. Returns NULL +
/// last-error on failure.
#[no_mangle]
pub extern "C" fn moonlitt_session_read_json(path: *const c_char) -> *mut c_char {
    ffi_guard!(std::ptr::null_mut(), {
        let path_str = match unsafe { cstr_to_str(path) } {
            Some(s) => s,
            None => {
                set_last_error_static(c"session path is NULL or not valid UTF-8");
                return std::ptr::null_mut();
            }
        };
        match Session::load_from_file(path_str) {
            Ok(session) => match session.to_json() {
                Ok(json) => to_c_string(&json),
                Err(e) => {
                    set_last_error(format!("serialize session: {e}"));
                    std::ptr::null_mut()
                }
            },
            Err(e) => {
                set_last_error(format!("read session: {e}"));
                std::ptr::null_mut()
            }
        }
    })
}
