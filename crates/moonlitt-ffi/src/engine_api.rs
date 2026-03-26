//! Engine FFI — opaque handle wrapping `moonlitt_engine::Engine`.
//!
//! All functions are NULL-safe. Functions that can fail return `c_int`
//! (0 = success, non-zero = error) and store the error message internally
//! for retrieval via `moonlitt_engine_get_error`.

use crate::util::{cstr_to_str, json_escape, to_c_string};
use moonlitt_engine::engine::Engine;
use std::ffi::{c_char, c_float, c_int};

/// Opaque engine handle exposed to C callers.
/// Stores the engine plus the last error message for `get_error`.
pub struct EngineHandle {
    pub(crate) engine: Option<Engine>,
    last_error: Option<String>,
    /// Cached CString for FFI error retrieval. The pointer returned by
    /// `get_error` remains valid until the next engine operation that
    /// overwrites `last_error` (same lifetime contract as C's `strerror`).
    last_error_cstring: Option<std::ffi::CString>,
}

impl EngineHandle {
    fn set_error(&mut self, msg: String) {
        self.last_error = Some(msg);
    }

    fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Set an error message (accessible from sibling modules).
    pub(crate) fn last_error_set(&mut self, msg: &str) {
        self.last_error = Some(msg.to_string());
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a new engine. Returns an opaque pointer, or null on failure.
#[no_mangle]
pub extern "C" fn moonlitt_engine_create(sample_rate: c_int, buffer_size: c_int) -> *mut EngineHandle {
    let engine = Engine::new(sample_rate.max(1) as u32, buffer_size.max(1) as u32);
    let handle = Box::new(EngineHandle {
        engine: Some(engine),
        last_error: None,
        last_error_cstring: None,
    });
    Box::into_raw(handle)
}

/// Destroy an engine handle. Safe to call with null.
#[no_mangle]
pub extern "C" fn moonlitt_engine_destroy(e: *mut EngineHandle) {
    if !e.is_null() {
        unsafe {
            drop(Box::from_raw(e));
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load a plugin/soundfont. Auto-detects format by file extension.
/// Returns 0 on success, 1 on error (retrieve via `moonlitt_engine_get_error`).
#[no_mangle]
pub extern "C" fn moonlitt_engine_load(e: *mut EngineHandle, path: *const c_char) -> c_int {
    let handle = match unsafe { e.as_mut() } {
        Some(h) => h,
        None => return 1,
    };
    let path = match unsafe { cstr_to_str(path) } {
        Some(p) => p,
        None => {
            handle.set_error("null path".into());
            return 1;
        }
    };
    let engine = match handle.engine.as_mut() {
        Some(eng) => eng,
        None => {
            handle.set_error("engine already consumed by runtime".into());
            return 1;
        }
    };
    match engine.load(path) {
        Ok(()) => {
            handle.clear_error();
            0
        }
        Err(err) => {
            handle.set_error(err.to_string());
            1
        }
    }
}

/// Unload the current backend.
#[no_mangle]
pub extern "C" fn moonlitt_engine_unload(e: *mut EngineHandle) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.unload();
        }
    }
}

/// Returns 1 if a backend is loaded, 0 otherwise.
#[no_mangle]
pub extern "C" fn moonlitt_engine_is_loaded(e: *mut EngineHandle) -> c_int {
    match unsafe { e.as_ref() } {
        Some(handle) => match handle.engine.as_ref() {
            Some(engine) => engine.is_loaded() as c_int,
            None => 0,
        },
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// MIDI
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn moonlitt_engine_note_on(e: *mut EngineHandle, ch: c_int, note: c_int, vel: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.note_on(ch as u8, note as u8, vel as u8);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_note_off(e: *mut EngineHandle, ch: c_int, note: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.note_off(ch as u8, note as u8);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_cc(e: *mut EngineHandle, ch: c_int, cc: c_int, val: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.cc(ch as u8, cc as u8, val as u8);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_pitch_bend(e: *mut EngineHandle, ch: c_int, val: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.pitch_bend(ch as u8, val as i16);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_program_change(e: *mut EngineHandle, ch: c_int, prog: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.program_change(ch as u8, prog as u8);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_all_notes_off(e: *mut EngineHandle) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.all_notes_off();
        }
    }
}

// ---------------------------------------------------------------------------
// Render / Volume
// ---------------------------------------------------------------------------

/// Render `frames` samples into `left` and `right` buffers.
/// Fills with silence if no backend is loaded.
#[no_mangle]
pub extern "C" fn moonlitt_engine_render(
    e: *mut EngineHandle,
    left: *mut c_float,
    right: *mut c_float,
    frames: c_int,
) {
    if left.is_null() || right.is_null() || frames <= 0 {
        return;
    }
    let handle = match unsafe { e.as_mut() } {
        Some(h) => h,
        None => return,
    };
    let n = frames as usize;
    let left_slice = unsafe { std::slice::from_raw_parts_mut(left, n) };
    let right_slice = unsafe { std::slice::from_raw_parts_mut(right, n) };
    match handle.engine.as_mut() {
        Some(engine) => engine.render(left_slice, right_slice),
        None => {
            left_slice.fill(0.0);
            right_slice.fill(0.0);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_set_volume(e: *mut EngineHandle, volume: c_float) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(engine) = handle.engine.as_mut() {
            engine.set_volume(volume);
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin scanning
// ---------------------------------------------------------------------------

/// Scan for available plugins. Returns a JSON array string.
/// Caller must free the returned string with `moonlitt_free_string`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_scan_plugins(e: *mut EngineHandle) -> *mut c_char {
    let handle = match unsafe { e.as_ref() } {
        Some(h) => h,
        None => return to_c_string("[]"),
    };
    let engine = match handle.engine.as_ref() {
        Some(eng) => eng,
        None => return to_c_string("[]"),
    };
    let plugins = engine.scan_plugins();
    let entries: Vec<String> = plugins
        .iter()
        .map(|p| {
            format!(
                r#"{{"name":"{}","path":"{}","format":"{:?}"}}"#,
                json_escape(&p.name),
                json_escape(&p.path),
                p.format,
            )
        })
        .collect();
    let json = format!("[{}]", entries.join(","));
    to_c_string(&json)
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// Get presets for the currently loaded backend. Returns a JSON array string.
/// Caller must free with `moonlitt_free_string`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_get_presets(e: *mut EngineHandle) -> *mut c_char {
    let handle = match unsafe { e.as_ref() } {
        Some(h) => h,
        None => return to_c_string("[]"),
    };
    let engine = match handle.engine.as_ref() {
        Some(eng) => eng,
        None => return to_c_string("[]"),
    };
    let presets = engine.presets();
    let entries: Vec<String> = presets
        .iter()
        .map(|p| {
            format!(
                r#"{{"id":{},"name":"{}"}}"#,
                p.id,
                json_escape(&p.name),
            )
        })
        .collect();
    let json = format!("[{}]", entries.join(","));
    to_c_string(&json)
}

/// Load a preset by ID. Returns 0 on success, 1 on error.
#[no_mangle]
pub extern "C" fn moonlitt_engine_load_preset(e: *mut EngineHandle, id: c_int) -> c_int {
    let handle = match unsafe { e.as_mut() } {
        Some(h) => h,
        None => return 1,
    };
    let engine = match handle.engine.as_mut() {
        Some(eng) => eng,
        None => {
            handle.set_error("engine already consumed by runtime".into());
            return 1;
        }
    };
    match engine.load_preset(id) {
        Ok(()) => {
            handle.clear_error();
            0
        }
        Err(err) => {
            handle.set_error(err.to_string());
            1
        }
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Get the number of parameters for the loaded backend.
#[no_mangle]
pub extern "C" fn moonlitt_engine_param_count(e: *mut EngineHandle) -> c_int {
    match unsafe { e.as_ref() } {
        Some(h) => match h.engine.as_ref() {
            Some(eng) => eng.param_count() as c_int,
            None => 0,
        },
        None => 0,
    }
}

/// Get all parameter info as a JSON array string.
/// Caller must free with `moonlitt_free_string`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_param_info_json(e: *mut EngineHandle) -> *mut c_char {
    let handle = match unsafe { e.as_ref() } {
        Some(h) => h,
        None => return to_c_string("[]"),
    };
    let engine = match handle.engine.as_ref() {
        Some(eng) => eng,
        None => return to_c_string("[]"),
    };
    let count = engine.param_count();
    let entries: Vec<String> = (0..count)
        .filter_map(|i| engine.param_info(i))
        .map(|p| {
            format!(
                r#"{{"id":{},"name":"{}","group":"{}","min":{},"max":{},"default":{},"step_count":{},"flags":{}}}"#,
                p.id,
                json_escape(&p.name),
                json_escape(&p.group),
                p.min, p.max, p.default,
                p.step_count,
                p.flags.bits(),
            )
        })
        .collect();
    to_c_string(&format!("[{}]", entries.join(",")))
}

/// Get current value of a parameter. Returns NaN if invalid.
#[no_mangle]
pub extern "C" fn moonlitt_engine_get_param(e: *mut EngineHandle, id: c_int) -> f64 {
    match unsafe { e.as_ref() } {
        Some(h) => match h.engine.as_ref() {
            Some(eng) => eng.get_param(id as u32).unwrap_or(f64::NAN),
            None => f64::NAN,
        },
        None => f64::NAN,
    }
}

/// Set a parameter value.
#[no_mangle]
pub extern "C" fn moonlitt_engine_set_param(e: *mut EngineHandle, id: c_int, value: f64) {
    if let Some(h) = unsafe { e.as_mut() } {
        if let Some(eng) = h.engine.as_mut() {
            eng.set_param(id as u32, value);
        }
    }
}

/// Get display string for a parameter value.
/// Caller must free with `moonlitt_free_string`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_param_display(e: *mut EngineHandle, id: c_int, value: f64) -> *mut c_char {
    match unsafe { e.as_ref() } {
        Some(h) => match h.engine.as_ref() {
            Some(eng) => {
                let s = eng.param_display(id as u32, value).unwrap_or_default();
                to_c_string(&s)
            }
            None => to_c_string(""),
        },
        None => to_c_string(""),
    }
}

// ---------------------------------------------------------------------------
// Error retrieval
// ---------------------------------------------------------------------------

/// Get the last error message, or null if no error.
///
/// The returned pointer is valid until the next engine operation that may
/// produce an error — same lifetime contract as C's `strerror()`. The
/// CString is stored in the EngineHandle itself, so it is safe to call
/// from any thread as long as the caller has exclusive access to the handle.
///
/// Do NOT free this pointer — it is owned by the engine handle.
#[no_mangle]
pub extern "C" fn moonlitt_engine_get_error(e: *mut EngineHandle) -> *const c_char {
    if e.is_null() {
        return std::ptr::null();
    }
    let handle = unsafe { &mut *e };
    match &handle.last_error {
        Some(msg) => {
            handle.last_error_cstring = std::ffi::CString::new(msg.as_str()).ok();
            handle
                .last_error_cstring
                .as_ref()
                .map(|cs| cs.as_ptr())
                .unwrap_or(std::ptr::null())
        }
        None => std::ptr::null(),
    }
}
