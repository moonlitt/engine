//! Engine FFI — opaque handle wrapping `Box<dyn AudioBackend>`.
//!
//! Conventions (ABI draft 0.9):
//!
//! * Every fallible function returns a [`MoonlittStatus`]; the detail
//!   message is available via `moonlitt_last_error_message()` on the
//!   calling thread.
//! * Arguments are validated **before** backend presence is checked, so
//!   `MOONLITT_ERR_INVALID_ARG` always wins over `MOONLITT_ERR_NOT_LOADED`.
//! * Out-of-range values are rejected, never silently clamped.
//! * Every body is wrapped in `ffi_guard!` — a Rust panic cannot unwind
//!   into the host process.
//!
//! Threading: engine handles are single-owner. Call all functions for a
//! given handle from one thread at a time (the control thread). For
//! real-time playback use the `moonlitt_runtime_*` family instead.

use crate::error::{
    ffi_guard, set_last_error, set_last_error_static, MoonlittStatus, MOONLITT_ERR_INVALID_ARG,
    MOONLITT_ERR_IO, MOONLITT_ERR_NOT_LOADED, MOONLITT_ERR_PLUGIN, MOONLITT_OK,
};
use crate::util::{cstr_to_str, json_escape, to_c_string};
use moonlitt_core::AudioBackend;
use std::ffi::{c_char, c_float, c_int, CStr};

/// Opaque engine handle exposed to C callers.
pub struct EngineHandle {
    pub(crate) backend: Option<Box<dyn AudioBackend>>,
    pub(crate) sample_rate: u32,
    pub(crate) buffer_size: u32,
    /// Path of the loaded file (for session persistence and FFI reload).
    pub(crate) loaded_path: Option<String>,
}

impl EngineHandle {
    /// Wrap an already-created backend (used by the built-in effect
    /// factories). Returns an owned raw pointer; the caller must
    /// eventually `moonlitt_engine_destroy` it or transfer ownership
    /// (e.g. `moonlitt_runtime_add_track`).
    pub(crate) fn with_backend(
        backend: Box<dyn AudioBackend>,
        sample_rate: u32,
        buffer_size: u32,
    ) -> *mut EngineHandle {
        Box::into_raw(Box::new(EngineHandle {
            backend: Some(backend),
            sample_rate,
            buffer_size,
            loaded_path: None,
        }))
    }
}

// ---------------------------------------------------------------------------
// Validation helpers — reject with a static (allocation-free) message
// ---------------------------------------------------------------------------

const MSG_NULL_HANDLE: &CStr = c"engine handle is NULL";
const MSG_NOT_LOADED: &CStr = c"no backend loaded (call moonlitt_engine_load first)";
const MSG_BAD_CHANNEL: &CStr = c"MIDI channel out of range (0..=15)";
const MSG_BAD_DATA_BYTE: &CStr = c"MIDI data byte out of range (0..=127)";
const MSG_BAD_BEND: &CStr = c"pitch bend out of range (-8192..=8191)";

pub(crate) fn channel(v: c_int) -> Result<u8, MoonlittStatus> {
    if (0..=15).contains(&v) {
        Ok(v as u8)
    } else {
        set_last_error_static(MSG_BAD_CHANNEL);
        Err(MOONLITT_ERR_INVALID_ARG)
    }
}

pub(crate) fn data_byte(v: c_int) -> Result<u8, MoonlittStatus> {
    if (0..=127).contains(&v) {
        Ok(v as u8)
    } else {
        set_last_error_static(MSG_BAD_DATA_BYTE);
        Err(MOONLITT_ERR_INVALID_ARG)
    }
}

/// Take the backend out of an `EngineHandle*` (ownership transfer into a
/// mixer/runtime), with full error reporting.
pub(crate) fn take_backend(e: *mut EngineHandle) -> Result<Box<dyn AudioBackend>, MoonlittStatus> {
    let handle = match unsafe { e.as_mut() } {
        Some(h) => h,
        None => {
            set_last_error_static(MSG_NULL_HANDLE);
            return Err(MOONLITT_ERR_INVALID_ARG);
        }
    };
    match handle.backend.take() {
        Some(b) => Ok(b),
        None => {
            set_last_error_static(
                c"engine handle has no backend (never loaded, or already consumed by a mixer/runtime)",
            );
            Err(MOONLITT_ERR_NOT_LOADED)
        }
    }
}

fn handle_mut<'a>(e: *mut EngineHandle) -> Result<&'a mut EngineHandle, MoonlittStatus> {
    match unsafe { e.as_mut() } {
        Some(h) => Ok(h),
        None => {
            set_last_error_static(MSG_NULL_HANDLE);
            Err(MOONLITT_ERR_INVALID_ARG)
        }
    }
}

fn backend_mut(h: &mut EngineHandle) -> Result<&mut Box<dyn AudioBackend>, MoonlittStatus> {
    match h.backend.as_mut() {
        Some(b) => Ok(b),
        None => {
            set_last_error_static(MSG_NOT_LOADED);
            Err(MOONLITT_ERR_NOT_LOADED)
        }
    }
}

/// Collapse a `Result<MoonlittStatus, MoonlittStatus>` from the helper
/// chain into the plain status C sees.
fn status(r: Result<MoonlittStatus, MoonlittStatus>) -> MoonlittStatus {
    r.unwrap_or_else(|s| s)
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a new engine handle with no backend loaded.
///
/// * `sample_rate` — Hz, must be > 0 (typical: 44100 or 48000)
/// * `buffer_size` — frames per render block, must be > 0 (typical: 256)
///
/// Returns an owned opaque pointer (free with `moonlitt_engine_destroy`),
/// or NULL with a last-error message if the configuration is invalid.
#[no_mangle]
pub extern "C" fn moonlitt_engine_create(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        if sample_rate <= 0 || buffer_size <= 0 {
            set_last_error(format!(
                "invalid engine config: sample_rate={sample_rate}, buffer_size={buffer_size} (both must be > 0)"
            ));
            return std::ptr::null_mut();
        }
        Box::into_raw(Box::new(EngineHandle {
            backend: None,
            sample_rate: sample_rate as u32,
            buffer_size: buffer_size as u32,
            loaded_path: None,
        }))
    })
}

/// Destroy an engine handle and its backend. Safe to call with NULL.
/// After this the pointer is dangling — do not use it again.
#[no_mangle]
pub extern "C" fn moonlitt_engine_destroy(e: *mut EngineHandle) {
    ffi_guard!((), {
        if !e.is_null() {
            unsafe {
                drop(Box::from_raw(e));
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load a plugin or soundfont; the format is detected from the file
/// extension (`.sf2` / `.vst3` / `.clap`). Replaces any backend that was
/// already loaded.
///
/// Returns `MOONLITT_OK`, `MOONLITT_ERR_INVALID_ARG` (NULL handle/path),
/// `MOONLITT_ERR_IO` (file missing/unreadable) or `MOONLITT_ERR_PLUGIN`
/// (the file exists but the backend failed to initialise).
#[no_mangle]
pub extern "C" fn moonlitt_engine_load(
    e: *mut EngineHandle,
    path: *const c_char,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let handle = handle_mut(e)?;
            let path = match unsafe { cstr_to_str(path) } {
                Some(p) => p,
                None => {
                    set_last_error_static(c"path is NULL or not valid UTF-8");
                    return Err(MOONLITT_ERR_INVALID_ARG);
                }
            };
            // Allow reloading: drop any existing backend first.
            handle.backend = None;
            handle.loaded_path = None;
            match moonlitt_engine::create(path, handle.sample_rate, handle.buffer_size) {
                Ok(backend) => {
                    handle.backend = Some(backend);
                    handle.loaded_path = Some(path.to_string());
                    Ok(MOONLITT_OK)
                }
                Err(err) => {
                    set_last_error(format!("load '{path}': {err}"));
                    if std::path::Path::new(path).exists() {
                        Err(MOONLITT_ERR_PLUGIN)
                    } else {
                        Err(MOONLITT_ERR_IO)
                    }
                }
            }
        })())
    })
}

/// Drop the loaded backend (if any). Safe on NULL and on an engine with
/// nothing loaded; never fails.
#[no_mangle]
pub extern "C" fn moonlitt_engine_unload(e: *mut EngineHandle) {
    ffi_guard!((), {
        if let Some(handle) = unsafe { e.as_mut() } {
            if let Some(ref mut backend) = handle.backend {
                backend.unload();
            }
            handle.backend = None;
            handle.loaded_path = None;
        }
    })
}

/// Returns 1 if a backend is loaded, 0 otherwise (including NULL handle).
#[no_mangle]
pub extern "C" fn moonlitt_engine_is_loaded(e: *mut EngineHandle) -> c_int {
    ffi_guard!(0, {
        match unsafe { e.as_ref() } {
            Some(handle) => handle.backend.is_some() as c_int,
            None => 0,
        }
    })
}

// ---------------------------------------------------------------------------
// MIDI (offline/control-thread path; for live audio use moonlitt_runtime_*)
// ---------------------------------------------------------------------------

/// Start a note.
///
/// * `ch`   — MIDI channel, 0..=15
/// * `note` — MIDI note number, 0..=127 (60 = middle C)
/// * `vel`  — velocity, 0..=127 (0 is interpreted as note-off by MIDI
///   convention; backends follow it)
#[no_mangle]
pub extern "C" fn moonlitt_engine_note_on(
    e: *mut EngineHandle,
    ch: c_int,
    note: c_int,
    vel: c_int,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, note, vel) = (channel(ch)?, data_byte(note)?, data_byte(vel)?);
            let backend = backend_mut(handle_mut(e)?)?;
            backend.note_on(ch, note, vel);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Release a note. `ch` 0..=15, `note` 0..=127.
#[no_mangle]
pub extern "C" fn moonlitt_engine_note_off(
    e: *mut EngineHandle,
    ch: c_int,
    note: c_int,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, note) = (channel(ch)?, data_byte(note)?);
            let backend = backend_mut(handle_mut(e)?)?;
            backend.note_off(ch, note);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Send a MIDI control change. `ch` 0..=15, `cc` 0..=127, `val` 0..=127.
/// Common controllers: 1 = mod wheel, 7 = volume, 11 = expression,
/// 64 = sustain pedal.
#[no_mangle]
pub extern "C" fn moonlitt_engine_cc(
    e: *mut EngineHandle,
    ch: c_int,
    cc: c_int,
    val: c_int,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, cc, val) = (channel(ch)?, data_byte(cc)?, data_byte(val)?);
            let backend = backend_mut(handle_mut(e)?)?;
            backend.cc(ch, cc, val);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Send a pitch-bend. `ch` 0..=15, `val` -8192..=8191 (0 = centre).
#[no_mangle]
pub extern "C" fn moonlitt_engine_pitch_bend(
    e: *mut EngineHandle,
    ch: c_int,
    val: c_int,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let ch = channel(ch)?;
            if !(-8192..=8191).contains(&val) {
                set_last_error_static(MSG_BAD_BEND);
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let backend = backend_mut(handle_mut(e)?)?;
            backend.pitch_bend(ch, val as i16);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Switch program/patch on a channel. `ch` 0..=15, `prog` 0..=127.
/// For VST3 sampler-style plugins prefer `moonlitt_engine_load_preset`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_program_change(
    e: *mut EngineHandle,
    ch: c_int,
    prog: c_int,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, prog) = (channel(ch)?, data_byte(prog)?);
            let backend = backend_mut(handle_mut(e)?)?;
            backend.program_change(ch, prog);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Release every sounding note on every channel.
#[no_mangle]
pub extern "C" fn moonlitt_engine_all_notes_off(e: *mut EngineHandle) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let backend = backend_mut(handle_mut(e)?)?;
            backend.all_notes_off();
            Ok(MOONLITT_OK)
        })())
    })
}

// ---------------------------------------------------------------------------
// Render / Volume
// ---------------------------------------------------------------------------

/// Render `frames` samples of audio into the caller's `left`/`right`
/// buffers (each must hold at least `frames` f32 values — the library
/// cannot verify this).
///
/// With no backend loaded the buffers are filled with silence and
/// `MOONLITT_ERR_NOT_LOADED` is returned, so a lazy caller still gets
/// valid audio while an attentive one can detect the missing load.
#[no_mangle]
pub extern "C" fn moonlitt_engine_render(
    e: *mut EngineHandle,
    left: *mut c_float,
    right: *mut c_float,
    frames: c_int,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            if left.is_null() || right.is_null() || frames <= 0 {
                set_last_error_static(c"render: NULL buffer or frames <= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let handle = handle_mut(e)?;
            let n = frames as usize;
            let left_slice = unsafe { std::slice::from_raw_parts_mut(left, n) };
            let right_slice = unsafe { std::slice::from_raw_parts_mut(right, n) };
            match handle.backend.as_mut() {
                Some(backend) => {
                    backend.render(left_slice, right_slice);
                    Ok(MOONLITT_OK)
                }
                None => {
                    left_slice.fill(0.0);
                    right_slice.fill(0.0);
                    set_last_error_static(MSG_NOT_LOADED);
                    Err(MOONLITT_ERR_NOT_LOADED)
                }
            }
        })())
    })
}

/// Set the backend's output volume. `volume` is linear gain (1.0 =
/// unity); NaN is rejected.
#[no_mangle]
pub extern "C" fn moonlitt_engine_set_volume(
    e: *mut EngineHandle,
    volume: c_float,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            if volume.is_nan() {
                set_last_error_static(c"volume is NaN");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let backend = backend_mut(handle_mut(e)?)?;
            backend.set_volume(volume);
            Ok(MOONLITT_OK)
        })())
    })
}

// ---------------------------------------------------------------------------
// Plugin scanning
// ---------------------------------------------------------------------------

/// Scan the system plugin directories. Returns a JSON array
/// `[{"name","path","format"}, …]`.
///
/// Ownership: the returned string is owned by the caller — free it with
/// `moonlitt_free_string`. Returns NULL on NULL handle.
#[no_mangle]
pub extern "C" fn moonlitt_engine_scan_plugins(e: *mut EngineHandle) -> *mut c_char {
    ffi_guard!(std::ptr::null_mut(), {
        let handle = match unsafe { e.as_ref() } {
            Some(h) => h,
            None => {
                set_last_error_static(MSG_NULL_HANDLE);
                return std::ptr::null_mut();
            }
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
        to_c_string(&format!("[{}]", entries.join(",")))
    })
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// List the loaded backend's presets as JSON `[{"id","name"}, …]`.
/// An empty list (`[]`) means the backend exposes no presets.
///
/// Ownership: caller frees with `moonlitt_free_string`. Returns NULL
/// (with NOT_LOADED / INVALID_ARG detail) when no backend is loaded.
#[no_mangle]
pub extern "C" fn moonlitt_engine_get_presets(e: *mut EngineHandle) -> *mut c_char {
    ffi_guard!(std::ptr::null_mut(), {
        let handle = match unsafe { e.as_ref() } {
            Some(h) => h,
            None => {
                set_last_error_static(MSG_NULL_HANDLE);
                return std::ptr::null_mut();
            }
        };
        let backend = match handle.backend.as_ref() {
            Some(b) => b,
            None => {
                set_last_error_static(MSG_NOT_LOADED);
                return std::ptr::null_mut();
            }
        };
        let entries: Vec<String> = backend
            .presets()
            .iter()
            .map(|p| format!(r#"{{"id":{},"name":"{}"}}"#, p.id, json_escape(&p.name)))
            .collect();
        to_c_string(&format!("[{}]", entries.join(",")))
    })
}

/// Load a preset by the id reported in `moonlitt_engine_get_presets`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_load_preset(e: *mut EngineHandle, id: c_int) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            let backend = backend_mut(handle_mut(e)?)?;
            match backend.load_preset(id) {
                Ok(()) => Ok(MOONLITT_OK),
                Err(err) => {
                    set_last_error(format!("load_preset({id}): {err}"));
                    Err(MOONLITT_ERR_PLUGIN)
                }
            }
        })())
    })
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Number of automatable parameters on the loaded backend (0 when
/// nothing is loaded or the handle is NULL).
#[no_mangle]
pub extern "C" fn moonlitt_engine_param_count(e: *mut EngineHandle) -> c_int {
    ffi_guard!(0, {
        match unsafe { e.as_ref() } {
            Some(h) => match h.backend.as_ref() {
                Some(b) => b.param_count() as c_int,
                None => 0,
            },
            None => 0,
        }
    })
}

/// All parameter metadata as JSON
/// `[{"id","name","group","min","max","default","step_count","flags"}, …]`.
///
/// Ownership: caller frees with `moonlitt_free_string`. Returns NULL
/// when no backend is loaded.
#[no_mangle]
pub extern "C" fn moonlitt_engine_param_info_json(e: *mut EngineHandle) -> *mut c_char {
    ffi_guard!(std::ptr::null_mut(), {
        let handle = match unsafe { e.as_ref() } {
            Some(h) => h,
            None => {
                set_last_error_static(MSG_NULL_HANDLE);
                return std::ptr::null_mut();
            }
        };
        let backend = match handle.backend.as_ref() {
            Some(b) => b,
            None => {
                set_last_error_static(MSG_NOT_LOADED);
                return std::ptr::null_mut();
            }
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
                    p.min,
                    p.max,
                    p.default,
                    p.step_count,
                    p.flags.bits(),
                )
            })
            .collect();
        to_c_string(&format!("[{}]", entries.join(",")))
    })
}

/// Current value of parameter `id`, or NaN when the handle/backend/id
/// is invalid (documented sentinel — check with `isnan`).
#[no_mangle]
pub extern "C" fn moonlitt_engine_get_param(e: *mut EngineHandle, id: c_int) -> f64 {
    ffi_guard!(f64::NAN, {
        match unsafe { e.as_ref() } {
            Some(h) => match h.backend.as_ref() {
                Some(b) if id >= 0 => b.get_param(id as u32).unwrap_or(f64::NAN),
                _ => f64::NAN,
            },
            None => f64::NAN,
        }
    })
}

/// Set parameter `id` to `value` (normalised to the range reported in
/// `moonlitt_engine_param_info_json`). NaN and negative ids are rejected.
#[no_mangle]
pub extern "C" fn moonlitt_engine_set_param(
    e: *mut EngineHandle,
    id: c_int,
    value: f64,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            if id < 0 {
                set_last_error_static(c"parameter id must be >= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            if value.is_nan() {
                set_last_error_static(c"parameter value is NaN");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let backend = backend_mut(handle_mut(e)?)?;
            backend.set_param(id as u32, value);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Human-readable display string for a parameter value (e.g. `"-6.0 dB"`).
///
/// Ownership: caller frees with `moonlitt_free_string`. Returns NULL when
/// the handle/backend/id is invalid.
#[no_mangle]
pub extern "C" fn moonlitt_engine_param_display(
    e: *mut EngineHandle,
    id: c_int,
    value: f64,
) -> *mut c_char {
    ffi_guard!(std::ptr::null_mut(), {
        let handle = match unsafe { e.as_ref() } {
            Some(h) => h,
            None => {
                set_last_error_static(MSG_NULL_HANDLE);
                return std::ptr::null_mut();
            }
        };
        let backend = match handle.backend.as_ref() {
            Some(b) => b,
            None => {
                set_last_error_static(MSG_NOT_LOADED);
                return std::ptr::null_mut();
            }
        };
        if id < 0 {
            set_last_error_static(c"parameter id must be >= 0");
            return std::ptr::null_mut();
        }
        match backend.param_display(id as u32, value) {
            Some(s) => to_c_string(&s),
            None => {
                set_last_error_static(c"unknown parameter id");
                std::ptr::null_mut()
            }
        }
    })
}

// ---------------------------------------------------------------------------
// RMS measurement (diagnostics)
// ---------------------------------------------------------------------------

/// Render a reference tone offline and measure its RMS level.
///
/// * `program` — GM program (0 = piano), 0..=127
/// * `note` — MIDI note, 0..=127 (60 = C4)
/// * `velocity` — 1..=127
/// * `duration_ms` — render length, must be > 0
///
/// Returns dBFS (negative). `-100.0` is the documented error/silence
/// sentinel; the detail is in `moonlitt_last_error_message()`.
#[no_mangle]
pub extern "C" fn moonlitt_engine_measure_rms(
    e: *mut EngineHandle,
    program: c_int,
    note: c_int,
    velocity: c_int,
    duration_ms: c_int,
) -> c_float {
    ffi_guard!(-100.0, {
        let handle = match unsafe { e.as_mut() } {
            Some(h) => h,
            None => {
                set_last_error_static(MSG_NULL_HANDLE);
                return -100.0;
            }
        };
        let backend = match handle.backend.as_mut() {
            Some(b) => b,
            None => {
                set_last_error_static(MSG_NOT_LOADED);
                return -100.0;
            }
        };
        if duration_ms <= 0 {
            set_last_error_static(c"duration_ms must be > 0");
            return -100.0;
        }

        let sr = handle.sample_rate;
        let total_frames = (sr as f64 * duration_ms as f64 / 1000.0) as usize;
        let buf_size = handle.buffer_size as usize;

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

        backend.note_off(0, note.clamp(0, 127) as u8);
        backend.all_notes_off();

        if count == 0 {
            set_last_error_static(c"nothing rendered");
            return -100.0;
        }
        let rms = (sum_sq / count as f64).sqrt();
        if rms < 1e-10 {
            set_last_error_static(c"rendered signal is silent");
            return -100.0;
        }
        (20.0 * rms.log10()) as c_float
    })
}

// ---------------------------------------------------------------------------
// Patch state (single-patch workflow: capture once in a GUI host, replay
// headless forever — the Keyscape/Omnisphere story)
// ---------------------------------------------------------------------------

/// Whether the loaded backend supports `save_state`/`load_state` (1/0).
/// VST3 plugins do; SF2 soundfonts address sounds by preset/program
/// instead. Returns 0 for NULL or empty handles.
#[no_mangle]
pub extern "C" fn moonlitt_engine_supports_state(e: *mut EngineHandle) -> c_int {
    ffi_guard!(0, {
        match unsafe { e.as_ref() } {
            Some(h) => match h.backend.as_ref() {
                Some(b) => b.supports_state() as c_int,
                None => 0,
            },
            None => 0,
        }
    })
}

/// Serialise the loaded backend's full patch state into an owned binary
/// buffer.
///
/// On success writes the buffer pointer to `*out_data` and its length to
/// `*out_len`. Ownership: the buffer belongs to the caller — release it
/// with `moonlitt_free_buffer(data, len)`.
///
/// Errors: `INVALID_ARG` (NULL handle/out-params), `NOT_LOADED`,
/// `UNSUPPORTED` (backend has no state story — check
/// `moonlitt_engine_supports_state` first), `STATE` (the plugin failed
/// to serialise).
#[no_mangle]
pub extern "C" fn moonlitt_engine_save_state(
    e: *mut EngineHandle,
    out_data: *mut *mut u8,
    out_len: *mut usize,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            if out_data.is_null() || out_len.is_null() {
                set_last_error_static(c"out_data / out_len must not be NULL");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let backend = backend_mut(handle_mut(e)?)?;
            if !backend.supports_state() {
                set_last_error_static(
                    c"backend does not support state (SF2: address sounds by preset instead)",
                );
                return Err(crate::error::MOONLITT_ERR_UNSUPPORTED);
            }
            match backend.save_state() {
                Ok(bytes) => {
                    let boxed = bytes.into_boxed_slice();
                    let len = boxed.len();
                    let ptr = Box::into_raw(boxed) as *mut u8;
                    unsafe {
                        *out_data = ptr;
                        *out_len = len;
                    }
                    Ok(MOONLITT_OK)
                }
                Err(err) => {
                    set_last_error(format!("save_state: {err}"));
                    Err(crate::error::MOONLITT_ERR_STATE)
                }
            }
        })())
    })
}

/// Restore a patch state previously produced by
/// `moonlitt_engine_save_state` (or captured in a GUI host).
///
/// For sample streamers, follow up with `moonlitt_engine_warm_up`
/// (`moonlitt_engine_recommended_warm_up_blocks` tells you how much)
/// before expecting audible output.
#[no_mangle]
pub extern "C" fn moonlitt_engine_load_state(
    e: *mut EngineHandle,
    data: *const u8,
    len: usize,
) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            if data.is_null() {
                set_last_error_static(c"data must not be NULL");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let backend = backend_mut(handle_mut(e)?)?;
            if !backend.supports_state() {
                set_last_error_static(
                    c"backend does not support state (SF2: address sounds by preset instead)",
                );
                return Err(crate::error::MOONLITT_ERR_UNSUPPORTED);
            }
            let slice = unsafe { std::slice::from_raw_parts(data, len) };
            match backend.load_state(slice) {
                Ok(()) => Ok(MOONLITT_OK),
                Err(err) => {
                    set_last_error(format!("load_state: {err}"));
                    Err(crate::error::MOONLITT_ERR_STATE)
                }
            }
        })())
    })
}

/// Advisory warm-up block count after `load_state` for this backend.
/// Sample streamers (Spectrasonics) report non-zero; synths report 0.
/// Returns 0 for NULL/empty handles.
#[no_mangle]
pub extern "C" fn moonlitt_engine_recommended_warm_up_blocks(e: *mut EngineHandle) -> c_int {
    ffi_guard!(0, {
        match unsafe { e.as_ref() } {
            Some(h) => match h.backend.as_ref() {
                Some(b) => b.recommended_warm_up_blocks() as c_int,
                None => 0,
            },
            None => 0,
        }
    })
}

/// Pump `blocks` silent render cycles so asynchronously-loading content
/// (sample streamers) comes online before the first note. Always safe —
/// a no-op for backends that don't need it. `blocks` must be >= 0.
#[no_mangle]
pub extern "C" fn moonlitt_engine_warm_up(e: *mut EngineHandle, blocks: c_int) -> MoonlittStatus {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        status((|| {
            if blocks < 0 {
                set_last_error_static(c"blocks must be >= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let backend = backend_mut(handle_mut(e)?)?;
            match backend.warm_up(blocks as usize) {
                Ok(()) => Ok(MOONLITT_OK),
                Err(err) => {
                    set_last_error(format!("warm_up: {err}"));
                    Err(crate::error::MOONLITT_ERR_PLUGIN)
                }
            }
        })())
    })
}
