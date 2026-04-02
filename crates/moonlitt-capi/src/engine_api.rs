//! Engine FFI — opaque handle wrapping `Box<dyn AudioBackend>`.
//!
//! All functions are NULL-safe. Functions that can fail return `c_int`
//! (0 = success, non-zero = error) and store the error message internally
//! for retrieval via `moonlitt_engine_get_error`.

use crate::util::{cstr_to_str, debug_warn_midi_range, json_escape, to_c_string};
use moonlitt_core::AudioBackend;
use std::ffi::{c_char, c_float, c_int};

/// Opaque engine handle exposed to C callers.
/// Stores the backend plus configuration and error state.
pub struct EngineHandle {
    pub(crate) backend: Option<Box<dyn AudioBackend>>,
    pub(crate) sample_rate: u32,
    pub(crate) buffer_size: u32,
    /// Path of the loaded file (for session persistence and FFI reload).
    pub(crate) loaded_path: Option<String>,
    pub(crate) last_error: Option<String>,
    /// Cached CString for FFI error retrieval. The pointer returned by
    /// `get_error` remains valid until the next engine operation that
    /// overwrites `last_error` (same lifetime contract as C's `strerror`).
    pub(crate) last_error_cstring: Option<std::ffi::CString>,
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

/// Create a new engine handle. Returns an opaque pointer.
/// The handle has no backend loaded — call `moonlitt_engine_load` next.
#[no_mangle]
pub extern "C" fn moonlitt_engine_create(sample_rate: c_int, buffer_size: c_int) -> *mut EngineHandle {
    let handle = Box::new(EngineHandle {
        backend: None,
        sample_rate: sample_rate.max(1) as u32,
        buffer_size: buffer_size.max(1) as u32,
        loaded_path: None,
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
    // Cannot load if backend was already consumed by runtime
    if handle.backend.is_some() || handle.loaded_path.is_some() {
        // Unload existing first (allow reloading)
        handle.backend = None;
        handle.loaded_path = None;
    }
    match moonlitt_engine::create(path, handle.sample_rate, handle.buffer_size) {
        Ok(backend) => {
            handle.backend = Some(backend);
            handle.loaded_path = Some(path.to_string());
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
        if let Some(ref mut backend) = handle.backend {
            backend.unload();
        }
        handle.backend = None;
        handle.loaded_path = None;
    }
}

/// Returns 1 if a backend is loaded, 0 otherwise.
#[no_mangle]
pub extern "C" fn moonlitt_engine_is_loaded(e: *mut EngineHandle) -> c_int {
    match unsafe { e.as_ref() } {
        Some(handle) => handle.backend.is_some() as c_int,
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// MIDI
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn moonlitt_engine_note_on(e: *mut EngineHandle, ch: c_int, note: c_int, vel: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(backend) = handle.backend.as_mut() {
            debug_warn_midi_range("engine_note_on", "ch", ch, 0, 15);
            debug_warn_midi_range("engine_note_on", "note", note, 0, 127);
            debug_warn_midi_range("engine_note_on", "vel", vel, 0, 127);
            let ch = (ch.max(0) as u8).min(15);
            let note = (note.max(0) as u8).min(127);
            let vel = (vel.max(0) as u8).min(127);
            backend.note_on(ch, note, vel);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_note_off(e: *mut EngineHandle, ch: c_int, note: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(backend) = handle.backend.as_mut() {
            debug_warn_midi_range("engine_note_off", "ch", ch, 0, 15);
            debug_warn_midi_range("engine_note_off", "note", note, 0, 127);
            let ch = (ch.max(0) as u8).min(15);
            let note = (note.max(0) as u8).min(127);
            backend.note_off(ch, note);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_cc(e: *mut EngineHandle, ch: c_int, cc: c_int, val: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(backend) = handle.backend.as_mut() {
            debug_warn_midi_range("engine_cc", "ch", ch, 0, 15);
            debug_warn_midi_range("engine_cc", "cc", cc, 0, 127);
            debug_warn_midi_range("engine_cc", "val", val, 0, 127);
            let ch = (ch.max(0) as u8).min(15);
            let cc = (cc.max(0) as u8).min(127);
            let val = (val.max(0) as u8).min(127);
            backend.cc(ch, cc, val);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_pitch_bend(e: *mut EngineHandle, ch: c_int, val: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(backend) = handle.backend.as_mut() {
            debug_warn_midi_range("engine_pitch_bend", "ch", ch, 0, 15);
            debug_warn_midi_range("engine_pitch_bend", "val", val, -8192, 8191);
            let ch = (ch.max(0) as u8).min(15);
            let val = (val.clamp(-8192, 8191)) as i16;
            backend.pitch_bend(ch, val);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_program_change(e: *mut EngineHandle, ch: c_int, prog: c_int) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(backend) = handle.backend.as_mut() {
            debug_warn_midi_range("engine_program_change", "ch", ch, 0, 15);
            debug_warn_midi_range("engine_program_change", "prog", prog, 0, 127);
            let ch = (ch.max(0) as u8).min(15);
            let prog = (prog.max(0) as u8).min(127);
            backend.program_change(ch, prog);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_all_notes_off(e: *mut EngineHandle) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(backend) = handle.backend.as_mut() {
            backend.all_notes_off();
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
    match handle.backend.as_mut() {
        Some(backend) => backend.render(left_slice, right_slice),
        None => {
            left_slice.fill(0.0);
            right_slice.fill(0.0);
        }
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_engine_set_volume(e: *mut EngineHandle, volume: c_float) {
    if let Some(handle) = unsafe { e.as_mut() } {
        if let Some(backend) = handle.backend.as_mut() {
            backend.set_volume(volume);
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
    let plugins = moonlitt_engine::scan_plugins(handle.sample_rate, handle.buffer_size);
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
    let backend = match handle.backend.as_ref() {
        Some(b) => b,
        None => return to_c_string("[]"),
    };
    let presets = backend.presets();
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
    let backend = match handle.backend.as_mut() {
        Some(b) => b,
        None => {
            handle.set_error("no backend loaded".into());
            return 1;
        }
    };
    match backend.load_preset(id) {
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
        Some(h) => match h.backend.as_ref() {
            Some(b) => b.param_count() as c_int,
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
    let backend = match handle.backend.as_ref() {
        Some(b) => b,
        None => return to_c_string("[]"),
    };
    let count = backend.param_count();
    let entries: Vec<String> = (0..count)
        .filter_map(|i| backend.param_info(i))
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
        Some(h) => match h.backend.as_ref() {
            Some(b) => b.get_param(id as u32).unwrap_or(f64::NAN),
            None => f64::NAN,
        },
        None => f64::NAN,
    }
}

/// Set a parameter value.
#[no_mangle]
pub extern "C" fn moonlitt_engine_set_param(e: *mut EngineHandle, id: c_int, value: f64) {
    if let Some(h) = unsafe { e.as_mut() } {
        if let Some(b) = h.backend.as_mut() {
            b.set_param(id as u32, value);
        }
    }
}

/// Get display string for a parameter value.
/// Caller must free with `moonlitt_free_string`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_param_display(e: *mut EngineHandle, id: c_int, value: f64) -> *mut c_char {
    match unsafe { e.as_ref() } {
        Some(h) => match h.backend.as_ref() {
            Some(b) => {
                let s = b.param_display(id as u32, value).unwrap_or_default();
                to_c_string(&s)
            }
            None => to_c_string(""),
        },
        None => to_c_string(""),
    }
}

// ---------------------------------------------------------------------------
// RMS measurement
// ---------------------------------------------------------------------------

/// Render a reference tone and measure RMS level in dBFS.
/// program: GM program (0=piano), note: MIDI note (60=C4),
/// velocity: 0-127, duration_ms: render length.
/// Returns dBFS (negative float). Returns -100.0 on error.
#[no_mangle]
pub extern "C" fn moonlitt_engine_measure_rms(
    e: *mut EngineHandle,
    program: c_int,
    note: c_int,
    velocity: c_int,
    duration_ms: c_int,
) -> c_float {
    let handle = match unsafe { e.as_mut() } {
        Some(h) => h,
        None => return -100.0,
    };
    let backend = match handle.backend.as_mut() {
        Some(b) => b,
        None => return -100.0,
    };

    let sr = handle.sample_rate;
    let total_frames = (sr as f64 * duration_ms as f64 / 1000.0) as usize;
    let buf_size = handle.buffer_size as usize;

    // Program change + note on
    backend.program_change(0, program.clamp(0, 127) as u8);
    backend.note_on(0, note.clamp(0, 127) as u8, velocity.clamp(1, 127) as u8);

    let mut sum_sq: f64 = 0.0;
    let mut count: usize = 0;
    let mut left = vec![0.0f32; buf_size];
    let mut right = vec![0.0f32; buf_size];

    let mut rendered = 0;
    while rendered < total_frames {
        let chunk = buf_size.min(total_frames - rendered);
        left[..chunk].fill(0.0);
        right[..chunk].fill(0.0);
        backend.render(&mut left[..chunk], &mut right[..chunk]);
        for i in 0..chunk {
            let mono = (left[i] as f64 + right[i] as f64) * 0.5;
            sum_sq += mono * mono;
            count += 1;
        }
        rendered += chunk;
    }

    // Note off + flush
    backend.note_off(0, note.clamp(0, 127) as u8);
    backend.all_notes_off();

    if count == 0 { return -100.0; }
    let rms = (sum_sq / count as f64).sqrt();
    if rms < 1e-10 { return -100.0; }
    (20.0 * rms.log10()) as c_float
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
