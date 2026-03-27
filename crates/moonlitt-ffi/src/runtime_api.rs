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
use crate::util::{debug_warn_midi_range, json_escape, to_c_string};
use moonlitt_runtime::Runtime;
use std::ffi::{c_char, c_float, c_int};

/// Opaque runtime handle exposed to C callers.
pub struct RuntimeHandle {
    runtime: Runtime,
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a runtime from an engine handle.
///
/// **Ownership semantics**: the engine is only consumed on success.
/// If runtime creation fails (e.g. no audio device), the engine is
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

    // Take ownership of the engine, leaving None behind.
    let engine = match handle.engine.take() {
        Some(e) => e,
        None => {
            handle.last_error_set("engine already consumed or null");
            return std::ptr::null_mut();
        }
    };

    match Runtime::new(engine) {
        Ok(runtime) => {
            let rt = Box::new(RuntimeHandle { runtime });
            Box::into_raw(rt)
        }
        Err((err, engine)) => {
            // Put the engine back — caller can retry or use it for offline rendering.
            handle.engine = Some(engine);
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
