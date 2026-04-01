//! Runtime FFI — opaque handle wrapping `moonlitt_runtime::Runtime`.
//!
//! The runtime owns the audio output stream and communicates with the
//! engine via a lock-free SPSC (single-producer, single-consumer) ring buffer.
//!
//! **Threading contract**: All MIDI/parameter/mixer FFI functions must be
//! called from a single thread (the producer side of the SPSC queue). The
//! audio thread is the consumer. Calling from multiple threads concurrently
//! is undefined behavior.

use crate::engine_api::EngineHandle;
use crate::util::{cstr_to_str, debug_warn_midi_range, json_escape, to_c_string};
use moonlitt_runtime::Runtime;
use std::ffi::{c_char, c_float, c_int};

/// Opaque runtime handle exposed to C callers.
pub struct RuntimeHandle {
    pub(crate) runtime: Runtime,
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a runtime from an engine handle.
///
/// **Ownership semantics**: the backend is only consumed on success.
/// If runtime creation fails (e.g. no audio device), the backend is
/// put back into the handle and the caller may retry or continue
/// using the engine for offline rendering.
///
/// Returns null on failure (retrieve error via the engine handle's
/// `moonlitt_engine_get_error`).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_create(engine_handle: *mut EngineHandle) -> *mut RuntimeHandle {
    let handle = match unsafe { engine_handle.as_mut() } {
        Some(h) => h,
        None => return std::ptr::null_mut(),
    };

    // Take ownership of the backend, leaving None behind.
    let backend = match handle.backend.take() {
        Some(b) => b,
        None => {
            handle.last_error_set("backend already consumed or null");
            return std::ptr::null_mut();
        }
    };

    let sample_rate = handle.sample_rate;
    let buffer_size = handle.buffer_size;

    match Runtime::new(backend, sample_rate, buffer_size) {
        Ok(runtime) => {
            let rt = Box::new(RuntimeHandle { runtime });
            Box::into_raw(rt)
        }
        Err((err, backend)) => {
            // Put the backend back — caller can retry or use it for offline rendering.
            handle.backend = Some(backend);
            handle.last_error_set(&err);
            std::ptr::null_mut()
        }
    }
}

/// Destroy a runtime handle. Safe to call with null.
/// This will stop audio output and clean up resources.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_destroy(rt: *mut RuntimeHandle) {
    if !rt.is_null() {
        unsafe {
            drop(Box::from_raw(rt));
        }
    }
}

// ---------------------------------------------------------------------------
// Audio output
// ---------------------------------------------------------------------------

/// Start audio output. Returns 0 on success, 1 on error.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_start(rt: *mut RuntimeHandle) -> c_int {
    match unsafe { rt.as_ref() } {
        Some(h) => match h.runtime.start() {
            Ok(()) => 0,
            Err(_) => 1,
        },
        None => 1,
    }
}

/// Stop (pause) audio output. Returns 0 on success, 1 on error.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_stop(rt: *mut RuntimeHandle) -> c_int {
    match unsafe { rt.as_ref() } {
        Some(h) => match h.runtime.stop() {
            Ok(()) => 0,
            Err(_) => 1,
        },
        None => 1,
    }
}

// ---------------------------------------------------------------------------
// MIDI events (lock-free SPSC ring buffer — single caller only,
// audio thread is the consumer)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_on(rt: *mut RuntimeHandle, ch: c_int, note: c_int, vel: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        debug_warn_midi_range("note_on", "ch", ch, 0, 15);
        debug_warn_midi_range("note_on", "note", note, 0, 127);
        debug_warn_midi_range("note_on", "vel", vel, 0, 127);
        let ch = (ch.max(0) as u8).min(15);
        let note = (note.max(0) as u8).min(127);
        let vel = (vel.max(0) as u8).min(127);
        h.runtime.note_on(ch, note, vel);
    }
}

/// Note-on with sample-accurate delay (23us precision).
/// `delay_samples` = number of samples to wait before triggering.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_on_delayed(
    rt: *mut RuntimeHandle, ch: c_int, note: c_int, vel: c_int, delay_samples: c_int,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        debug_warn_midi_range("note_on_delayed", "ch", ch, 0, 15);
        debug_warn_midi_range("note_on_delayed", "note", note, 0, 127);
        debug_warn_midi_range("note_on_delayed", "vel", vel, 0, 127);
        let ch = (ch.max(0) as u8).min(15);
        let note = (note.max(0) as u8).min(127);
        let vel = (vel.max(0) as u8).min(127);
        h.runtime.note_on_delayed(ch, note, vel, delay_samples.max(0) as u32);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_off(rt: *mut RuntimeHandle, ch: c_int, note: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        debug_warn_midi_range("note_off", "ch", ch, 0, 15);
        debug_warn_midi_range("note_off", "note", note, 0, 127);
        let ch = (ch.max(0) as u8).min(15);
        let note = (note.max(0) as u8).min(127);
        h.runtime.note_off(ch, note);
    }
}

/// Note-off with sample-accurate delay.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_off_delayed(
    rt: *mut RuntimeHandle, ch: c_int, note: c_int, delay_samples: c_int,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        debug_warn_midi_range("note_off_delayed", "ch", ch, 0, 15);
        debug_warn_midi_range("note_off_delayed", "note", note, 0, 127);
        let ch = (ch.max(0) as u8).min(15);
        let note = (note.max(0) as u8).min(127);
        h.runtime.note_off_delayed(ch, note, delay_samples.max(0) as u32);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_cc(rt: *mut RuntimeHandle, ch: c_int, cc: c_int, val: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        debug_warn_midi_range("cc", "ch", ch, 0, 15);
        debug_warn_midi_range("cc", "cc", cc, 0, 127);
        debug_warn_midi_range("cc", "val", val, 0, 127);
        let ch = (ch.max(0) as u8).min(15);
        let cc = (cc.max(0) as u8).min(127);
        let val = (val.max(0) as u8).min(127);
        h.runtime.cc(ch, cc, val);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_pitch_bend(rt: *mut RuntimeHandle, ch: c_int, val: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        debug_warn_midi_range("pitch_bend", "ch", ch, 0, 15);
        debug_warn_midi_range("pitch_bend", "val", val, -8192, 8191);
        let ch = (ch.max(0) as u8).min(15);
        let val = (val.clamp(-8192, 8191)) as i16;
        h.runtime.pitch_bend(ch, val);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_program_change(rt: *mut RuntimeHandle, ch: c_int, prog: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        debug_warn_midi_range("program_change", "ch", ch, 0, 15);
        debug_warn_midi_range("program_change", "prog", prog, 0, 127);
        let ch = (ch.max(0) as u8).min(15);
        let prog = (prog.max(0) as u8).min(127);
        h.runtime.program_change(ch, prog);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_all_notes_off(rt: *mut RuntimeHandle) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.all_notes_off();
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_volume(rt: *mut RuntimeHandle, volume: c_float) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.set_volume(volume);
    }
}

// ---------------------------------------------------------------------------
// Parameters (lock-free SPSC — single caller only)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_param(rt: *mut RuntimeHandle, id: c_int, value: c_float) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.set_param(id as u32, value);
    }
}

// ---------------------------------------------------------------------------
// Mixer control (lock-free SPSC — single caller only)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_track_volume(rt: *mut RuntimeHandle, track_id: c_int, vol: c_float) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_track_volume(track_id as u8, vol);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_track_trim(
    rt: *mut RuntimeHandle, track_id: c_int, trim_db: c_float,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_track_trim(track_id as u8, trim_db);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_track_pan(rt: *mut RuntimeHandle, track_id: c_int, pan: c_float) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_track_pan(track_id as u8, pan);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_track_mute(rt: *mut RuntimeHandle, track_id: c_int, mute: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_track_mute(track_id as u8, mute != 0);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_track_solo(rt: *mut RuntimeHandle, track_id: c_int, solo: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_track_solo(track_id as u8, solo != 0);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_track_send(rt: *mut RuntimeHandle, track_id: c_int, bus_id: c_int, level: c_float) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_track_send(track_id as u8, bus_id as u8, level);
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_master_volume(rt: *mut RuntimeHandle, vol: c_float) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_master_volume(vol);
    }
}

// ---------------------------------------------------------------------------
// Insert effect control (lock-free SPSC — single caller only)
// ---------------------------------------------------------------------------

/// Set bypass state for an insert effect on a track.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_insert_bypass(
    rt: *mut RuntimeHandle, track_id: c_int, insert_id: c_int, bypass: c_int,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_insert_bypass(track_id as u8, insert_id as u8, bypass != 0);
    }
}

/// Set a parameter on a specific track's backend.
#[no_mangle]
pub extern "C" fn moonlitt_set_param_for_track(
    rt: *mut RuntimeHandle, track_id: c_int, param_id: c_int, value: c_float,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.set_param_for_track(track_id as u8, param_id as u16, value);
    }
}

/// Set a parameter on a specific insert effect.
#[no_mangle]
pub extern "C" fn moonlitt_set_insert_param(
    rt: *mut RuntimeHandle, track_id: c_int, insert_id: c_int, param_id: c_int, value: c_float,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.set_insert_param(track_id as u8, insert_id as u8, param_id as u16, value);
    }
}

// ---------------------------------------------------------------------------
// Dynamic track/insert/bus management (via command channel)
// ---------------------------------------------------------------------------

/// Add a track from an engine handle at runtime. Returns track ID, or -1 on error.
/// The backend is consumed on success (taken from the handle).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_add_track(
    rt: *mut RuntimeHandle, engine_handle: *mut EngineHandle, channel_mask: c_int,
) -> c_int {
    let rt = match unsafe { rt.as_mut() } {
        Some(r) => r,
        None => return -1,
    };
    let eh = match unsafe { engine_handle.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let backend = match eh.backend.take() {
        Some(b) => b,
        None => return -1,
    };
    rt.runtime.add_track(backend, channel_mask as u16) as c_int
}

/// Remove a track at runtime. Notes are silenced before removal.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_remove_track(rt: *mut RuntimeHandle, track_id: c_int) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.remove_track(track_id as u32);
    }
}

/// Add an insert effect to a track at runtime. Returns insert ID, or -1 on error.
/// The backend is consumed on success.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_add_insert(
    rt: *mut RuntimeHandle, track_id: c_int, engine_handle: *mut EngineHandle,
) -> c_int {
    let rt = match unsafe { rt.as_mut() } {
        Some(r) => r,
        None => return -1,
    };
    let eh = match unsafe { engine_handle.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let backend = match eh.backend.take() {
        Some(b) => b,
        None => return -1,
    };
    rt.runtime.add_insert(track_id as u32, backend) as c_int
}

/// Remove an insert effect from a track at runtime.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_remove_insert(
    rt: *mut RuntimeHandle, track_id: c_int, insert_id: c_int,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.remove_insert(track_id as u32, insert_id as u32);
    }
}

/// Add a send bus at runtime. Returns bus ID, or -1 on error.
/// The backend is consumed on success.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_add_send_bus(
    rt: *mut RuntimeHandle, engine_handle: *mut EngineHandle,
) -> c_int {
    let rt = match unsafe { rt.as_mut() } {
        Some(r) => r,
        None => return -1,
    };
    let eh = match unsafe { engine_handle.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let backend = match eh.backend.take() {
        Some(b) => b,
        None => return -1,
    };
    rt.runtime.add_send_bus(backend) as c_int
}

/// Set a parameter on a send bus effect backend.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_send_bus_param(
    rt: *mut RuntimeHandle, bus_id: c_int, param_id: c_int, value: c_float,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.set_send_bus_param(bus_id as u8, param_id as u16, value);
    }
}

/// Route a track's output. target_id = 0xFF for master, else group track ID.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_set_track_route(
    rt: *mut RuntimeHandle, track_id: c_int, target_id: c_int,
) {
    if let Some(h) = unsafe { rt.as_mut() } {
        h.runtime.mixer_set_track_route(track_id as u8, target_id as u8);
    }
}

// ---------------------------------------------------------------------------
// MIDI device listing
// ---------------------------------------------------------------------------

/// List available MIDI input devices. Returns a JSON array string.
/// Caller must free with `moonlitt_free_string`.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_list_midi_inputs() -> *mut c_char {
    match Runtime::list_midi_inputs() {
        Ok(devices) => {
            let entries: Vec<String> = devices
                .iter()
                .map(|d| {
                    format!(
                        r#"{{"id":{},"name":"{}"}}"#,
                        d.id,
                        json_escape(&d.name),
                    )
                })
                .collect();
            let json = format!("[{}]", entries.join(","));
            to_c_string(&json)
        }
        Err(_) => to_c_string("[]"),
    }
}

// ---------------------------------------------------------------------------
// Transport (sequencer control)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn moonlitt_runtime_play(rt: *mut RuntimeHandle) {
    if let Some(h) = unsafe { rt.as_ref() } {
        h.runtime.play();
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_pause(rt: *mut RuntimeHandle) {
    if let Some(h) = unsafe { rt.as_ref() } {
        h.runtime.pause_playback();
    }
}

#[no_mangle]
pub extern "C" fn moonlitt_runtime_stop_playback(rt: *mut RuntimeHandle) {
    if let Some(h) = unsafe { rt.as_ref() } {
        h.runtime.stop_playback();
    }
}

// ---------------------------------------------------------------------------
// Session save/load
// ---------------------------------------------------------------------------

/// Save mixer session to a file. Returns 0 on success, 1 on error.
/// The path must be a valid UTF-8 C string.
#[no_mangle]
pub extern "C" fn moonlitt_session_save(path: *const c_char) -> c_int {
    let path = match unsafe { cstr_to_str(path) } {
        Some(p) => p,
        None => return 1,
    };
    // Session save requires access to the mixer on the audio thread.
    // For FFI, we provide file-based save/load that works with Session JSON.
    // The caller is expected to get the JSON string and write it themselves,
    // or use the standalone save function below.
    // This is a placeholder — full integration requires command channel coordination.
    let _ = path;
    1 // Not yet wired to runtime; use moonlitt_session_save_json instead
}

/// Get session JSON from a mixer state snapshot.
/// Caller must free the returned string with `moonlitt_free_string`.
/// Returns null if the path is invalid.
#[no_mangle]
pub extern "C" fn moonlitt_session_load_file(path: *const c_char) -> *mut c_char {
    let path = match unsafe { cstr_to_str(path) } {
        Some(p) => p,
        None => return std::ptr::null_mut(),
    };
    match moonlitt_runtime::Session::load_from_file(path) {
        Ok(session) => match session.to_json() {
            Ok(json) => to_c_string(&json),
            Err(_) => std::ptr::null_mut(),
        },
        Err(_) => std::ptr::null_mut(),
    }
}
