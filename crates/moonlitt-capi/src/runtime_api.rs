//! Runtime FFI — opaque handle wrapping `moonlitt_audio_io::Runtime`.
//!
//! The runtime owns the live audio output stream and talks to the audio
//! thread through a lock-free SPSC (single-producer single-consumer)
//! ring buffer, so every function here is wait-free with respect to the
//! audio thread.
//!
//! Conventions (ABI draft 0.9):
//!
//! * Family prefix == handle type: everything taking a `RuntimeHandle*`
//!   is `moonlitt_runtime_*`.
//! * Fallible functions return [`MoonlittStatus`]; detail via
//!   `moonlitt_last_error_message()`. Event functions report
//!   `MOONLITT_ERR_QUEUE_FULL` when the ring buffer is full (the event
//!   is dropped, never blocked on — retry on the next tick).
//! * Arguments are validated before anything else; out-of-range values
//!   are rejected, never truncated.
//!
//! **Threading contract**: all functions for one runtime must be called
//! from a single thread (the producer side of the SPSC queue) — usually
//! the game/UI thread. The audio thread is the consumer.

use crate::engine_api::{channel, data_byte, take_backend, EngineHandle};
use crate::error::{
    ffi_guard, set_last_error, set_last_error_static, MoonlittStatus, MOONLITT_ERR_INVALID_ARG,
    MOONLITT_ERR_PANIC, MOONLITT_ERR_QUEUE_FULL, MOONLITT_OK,
};
use crate::util::{cstr_to_str, json_escape, to_c_string};
use moonlitt_audio_io::Runtime;
use std::ffi::{c_char, c_float, c_int};

/// Opaque runtime handle exposed to C callers.
pub struct RuntimeHandle {
    pub(crate) runtime: Runtime,
    /// Control-side mirror of the mixer, kept in lock-step by every
    /// mutation here so `moonlitt_runtime_save_session` can snapshot
    /// without touching the audio thread.
    pub(crate) shadow: crate::shadow::SessionShadow,
}

// ---------------------------------------------------------------------------
// Shared validation helpers
// ---------------------------------------------------------------------------

const MSG_NULL_RT: &std::ffi::CStr = c"runtime handle is NULL";
const MSG_QUEUE_FULL: &std::ffi::CStr =
    c"event queue to the audio thread is full; event dropped (retry next tick)";
const MSG_BAD_ID: &std::ffi::CStr = c"id out of range (0..=255)";
const MSG_BAD_BEND: &std::ffi::CStr = c"pitch bend out of range (-8192..=8191)";
const MSG_NAN: &std::ffi::CStr = c"value is NaN";

fn rt_mut<'a>(rt: *mut RuntimeHandle) -> Result<&'a mut RuntimeHandle, MoonlittStatus> {
    match unsafe { rt.as_mut() } {
        Some(h) => Ok(h),
        None => {
            set_last_error_static(MSG_NULL_RT);
            Err(MOONLITT_ERR_INVALID_ARG)
        }
    }
}

/// Validate a track/insert/bus id into the u8 space used by the event
/// protocol. Rejects instead of truncating.
fn id_u8(v: c_int) -> Result<u8, MoonlittStatus> {
    if (0..=255).contains(&v) {
        Ok(v as u8)
    } else {
        set_last_error_static(MSG_BAD_ID);
        Err(MOONLITT_ERR_INVALID_ARG)
    }
}

fn not_nan_f32(v: c_float) -> Result<f32, MoonlittStatus> {
    if v.is_nan() {
        set_last_error_static(MSG_NAN);
        Err(MOONLITT_ERR_INVALID_ARG)
    } else {
        Ok(v)
    }
}

fn not_nan_f64(v: f64) -> Result<f64, MoonlittStatus> {
    if v.is_nan() {
        set_last_error_static(MSG_NAN);
        Err(MOONLITT_ERR_INVALID_ARG)
    } else {
        Ok(v)
    }
}

/// Map the ring-buffer push result onto the status convention.
fn queued(pushed: bool) -> Result<MoonlittStatus, MoonlittStatus> {
    if pushed {
        Ok(MOONLITT_OK)
    } else {
        set_last_error_static(MSG_QUEUE_FULL);
        Err(MOONLITT_ERR_QUEUE_FULL)
    }
}

fn status(r: Result<MoonlittStatus, MoonlittStatus>) -> MoonlittStatus {
    r.unwrap_or_else(|s| s)
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a live-audio runtime from an engine handle.
///
/// **Ownership**: the backend is consumed out of `engine_handle` only on
/// success. On failure (e.g. no audio output device) the backend is put
/// back, so the caller may retry or keep using the engine offline. The
/// (now empty) engine handle must still be destroyed eventually.
///
/// Returns NULL + last-error on failure.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_create(engine_handle: *mut EngineHandle) -> *mut RuntimeHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let handle = match unsafe { engine_handle.as_mut() } {
            Some(h) => h,
            None => {
                set_last_error_static(c"engine handle is NULL");
                return std::ptr::null_mut();
            }
        };
        let backend = match handle.backend.take() {
            Some(b) => b,
            None => {
                set_last_error_static(
                    c"engine handle has no backend (never loaded, or already consumed)",
                );
                return std::ptr::null_mut();
            }
        };

        let sample_rate = handle.sample_rate;
        let buffer_size = handle.buffer_size;
        let source =
            crate::shadow::ShadowSource::from_backend(&*backend, handle.loaded_path.clone());

        match Runtime::new(backend, sample_rate, buffer_size) {
            Ok(runtime) => Box::into_raw(Box::new(RuntimeHandle {
                runtime,
                shadow: crate::shadow::SessionShadow::single_track(sample_rate, source),
            })),
            Err((err, backend)) => {
                // Put the backend back — caller can retry or render offline.
                handle.backend = Some(backend);
                set_last_error(format!("runtime creation failed: {err}"));
                std::ptr::null_mut()
            }
        }
    })
}

/// Destroy a runtime handle: stops audio output and frees resources.
/// Safe to call with NULL.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_destroy(rt: *mut RuntimeHandle) {
    ffi_guard!((), {
        if !rt.is_null() {
            unsafe {
                drop(Box::from_raw(rt));
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Audio stream control
// ---------------------------------------------------------------------------

/// Start the audio output stream (begin calling the audio device).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_start_audio(rt: *mut RuntimeHandle) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let h = rt_mut(rt)?;
            match h.runtime.start() {
                Ok(()) => Ok(MOONLITT_OK),
                Err(e) => {
                    set_last_error(format!("start audio: {e}"));
                    Err(crate::error::MOONLITT_ERR_IO)
                }
            }
        })())
    })
}

/// Pause the audio output stream (the device callback stops firing).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_stop_audio(rt: *mut RuntimeHandle) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let h = rt_mut(rt)?;
            match h.runtime.stop() {
                Ok(()) => Ok(MOONLITT_OK),
                Err(e) => {
                    set_last_error(format!("stop audio: {e}"));
                    Err(crate::error::MOONLITT_ERR_IO)
                }
            }
        })())
    })
}

// ---------------------------------------------------------------------------
// MIDI events (lock-free; QUEUE_FULL when the ring buffer is full)
// ---------------------------------------------------------------------------

/// Start a note. `ch` 0..=15, `note` 0..=127, `vel` 0..=127.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_on(
    rt: *mut RuntimeHandle,
    ch: c_int,
    note: c_int,
    vel: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, note, vel) = (channel(ch)?, data_byte(note)?, data_byte(vel)?);
            let h = rt_mut(rt)?;
            queued(h.runtime.note_on(ch, note, vel))
        })())
    })
}

/// Note-on delayed by `delay_samples` frames for sample-accurate timing
/// (`delay_samples` >= 0; at 48 kHz one frame ≈ 21 µs).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_on_delayed(
    rt: *mut RuntimeHandle,
    ch: c_int,
    note: c_int,
    vel: c_int,
    delay_samples: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, note, vel) = (channel(ch)?, data_byte(note)?, data_byte(vel)?);
            if delay_samples < 0 {
                set_last_error_static(c"delay_samples must be >= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let h = rt_mut(rt)?;
            queued(
                h.runtime
                    .note_on_delayed(ch, note, vel, delay_samples as u32),
            )
        })())
    })
}

/// Release a note. `ch` 0..=15, `note` 0..=127.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_off(
    rt: *mut RuntimeHandle,
    ch: c_int,
    note: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, note) = (channel(ch)?, data_byte(note)?);
            let h = rt_mut(rt)?;
            queued(h.runtime.note_off(ch, note))
        })())
    })
}

/// Note-off delayed by `delay_samples` frames (>= 0).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_note_off_delayed(
    rt: *mut RuntimeHandle,
    ch: c_int,
    note: c_int,
    delay_samples: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, note) = (channel(ch)?, data_byte(note)?);
            if delay_samples < 0 {
                set_last_error_static(c"delay_samples must be >= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let h = rt_mut(rt)?;
            queued(h.runtime.note_off_delayed(ch, note, delay_samples as u32))
        })())
    })
}

/// Send a MIDI control change. `ch` 0..=15, `cc` 0..=127, `val` 0..=127.
/// Common controllers: 1 = mod wheel, 7 = volume, 11 = expression,
/// 64 = sustain pedal.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_cc(
    rt: *mut RuntimeHandle,
    ch: c_int,
    cc: c_int,
    val: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, cc, val) = (channel(ch)?, data_byte(cc)?, data_byte(val)?);
            let h = rt_mut(rt)?;
            queued(h.runtime.cc(ch, cc, val))
        })())
    })
}

/// Send a pitch-bend. `ch` 0..=15, `val` -8192..=8191 (0 = centre).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_pitch_bend(
    rt: *mut RuntimeHandle,
    ch: c_int,
    val: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let ch = channel(ch)?;
            if !(-8192..=8191).contains(&val) {
                set_last_error_static(MSG_BAD_BEND);
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let h = rt_mut(rt)?;
            queued(h.runtime.pitch_bend(ch, val as i16))
        })())
    })
}

/// Switch program/patch on a channel. `ch` 0..=15, `prog` 0..=127.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_program_change(
    rt: *mut RuntimeHandle,
    ch: c_int,
    prog: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (ch, prog) = (channel(ch)?, data_byte(prog)?);
            let h = rt_mut(rt)?;
            queued(h.runtime.program_change(ch, prog))
        })())
    })
}

/// Release every sounding note on every channel.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_all_notes_off(rt: *mut RuntimeHandle) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let h = rt_mut(rt)?;
            queued(h.runtime.all_notes_off())
        })())
    })
}

/// Set the source backend's output volume (linear gain, 1.0 = unity).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_volume(
    rt: *mut RuntimeHandle,
    volume: c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let volume = not_nan_f32(volume)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.set_volume(volume))
        })())
    })
}

// ---------------------------------------------------------------------------
// Backend parameters
// ---------------------------------------------------------------------------

/// Set backend parameter `id` to `value` (f64, matching the engine-side
/// parameter precision).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_param(
    rt: *mut RuntimeHandle,
    id: c_int,
    value: f64,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            if id < 0 {
                set_last_error_static(c"parameter id must be >= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let value = not_nan_f64(value)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.set_param(id as u32, value))
        })())
    })
}

/// Set backend parameter `param_id` on one specific track.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_param(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    param_id: c_int,
    value: f64,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let track = id_u8(track_id)?;
            if !(0..=u16::MAX as c_int).contains(&param_id) {
                set_last_error_static(c"param_id out of range (0..=65535)");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let value = not_nan_f64(value)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.set_param_for_track(track, param_id as u16, value))
        })())
    })
}

/// Set parameter `param_id` on one insert effect of a track.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_insert_param(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    insert_id: c_int,
    param_id: c_int,
    value: f64,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (track, insert) = (id_u8(track_id)?, id_u8(insert_id)?);
            if !(0..=u16::MAX as c_int).contains(&param_id) {
                set_last_error_static(c"param_id out of range (0..=65535)");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let value = not_nan_f64(value)?;
            let h = rt_mut(rt)?;
            queued(
                h.runtime
                    .set_insert_param(track, insert, param_id as u16, value),
            )
        })())
    })
}

/// Set parameter `param_id` on a send bus effect.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_send_bus_param(
    rt: *mut RuntimeHandle,
    bus_id: c_int,
    param_id: c_int,
    value: f64,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let bus = id_u8(bus_id)?;
            if !(0..=u16::MAX as c_int).contains(&param_id) {
                set_last_error_static(c"param_id out of range (0..=65535)");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let value = not_nan_f64(value)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.set_send_bus_param(bus, param_id as u16, value))
        })())
    })
}

// ---------------------------------------------------------------------------
// Track mixer controls (f32 — these are mixer gains, not backend params)
// ---------------------------------------------------------------------------

/// Track fader volume (linear gain, 1.0 = unity).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_volume(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    vol: c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let track = id_u8(track_id)?;
            let vol = not_nan_f32(vol)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_track_volume(track, vol))
                .inspect(|_| h.shadow.set_track_volume(track as u32, vol))
        })())
    })
}

/// Track input trim in dB (0.0 = no trim).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_trim(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    trim_db: c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let track = id_u8(track_id)?;
            let trim = not_nan_f32(trim_db)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_track_trim(track, trim))
                .inspect(|_| h.shadow.set_track_trim(track as u32, trim))
        })())
    })
}

/// Track pan: 0.0 = hard left, 0.5 = centre, 1.0 = hard right.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_pan(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    pan: c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let track = id_u8(track_id)?;
            let pan = not_nan_f32(pan)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_track_pan(track, pan))
                .inspect(|_| h.shadow.set_track_pan(track as u32, pan))
        })())
    })
}

/// Mute (1) / unmute (0) a track.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_mute(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    mute: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let track = id_u8(track_id)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_track_mute(track, mute != 0))
                .inspect(|_| h.shadow.set_track_mute(track as u32, mute != 0))
        })())
    })
}

/// Solo (1) / unsolo (0) a track.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_solo(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    solo: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let track = id_u8(track_id)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_track_solo(track, solo != 0))
                .inspect(|_| h.shadow.set_track_solo(track as u32, solo != 0))
        })())
    })
}

/// Per-track send level into a send bus (linear gain).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_send(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    bus_id: c_int,
    level: c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (track, bus) = (id_u8(track_id)?, id_u8(bus_id)?);
            let level = not_nan_f32(level)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_track_send(track, bus, level))
                .inspect(|_| h.shadow.set_track_send(track as u32, bus as usize, level))
        })())
    })
}

/// Route a track's output: `target_id` 0xFF (255) = master bus, any
/// other value = a group track id.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_track_route(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    target_id: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (track, target) = (id_u8(track_id)?, id_u8(target_id)?);
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_track_route(track, target))
        })())
    })
}

/// Master bus volume (linear gain, 1.0 = unity).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_master_volume(
    rt: *mut RuntimeHandle,
    vol: c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let vol = not_nan_f32(vol)?;
            let h = rt_mut(rt)?;
            queued(h.runtime.mixer_set_master_volume(vol))
                .inspect(|_| h.shadow.set_master_volume(vol))
        })())
    })
}

/// Bypass (1) / engage (0) an insert effect on a track.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_insert_bypass(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    insert_id: c_int,
    bypass: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (track, insert) = (id_u8(track_id)?, id_u8(insert_id)?);
            let h = rt_mut(rt)?;
            queued(
                h.runtime
                    .mixer_set_insert_bypass(track, insert, bypass != 0),
            )
            .inspect(|_| {
                h.shadow
                    .set_insert_bypass(track as u32, insert as u32, bypass != 0)
            })
        })())
    })
}

/// Feed an insert's external sidechain from another track.
/// `source_track_id` -1 reverts to the internal sidechain.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_set_insert_sidechain(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    insert_id: c_int,
    source_track_id: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let (track, insert) = (id_u8(track_id)?, id_u8(insert_id)?);
            let source = if source_track_id < 0 {
                None
            } else {
                Some(id_u8(source_track_id)?)
            };
            let h = rt_mut(rt)?;
            queued(h.runtime.set_insert_sidechain(track, insert, source))
        })())
    })
}

// ---------------------------------------------------------------------------
// Dynamic track/insert/bus management (via command channel)
// ---------------------------------------------------------------------------

/// Add a track at runtime. **Consumes** the backend out of
/// `engine_handle`. `channel_mask`: bit N = MIDI channel N (0xFFFF = all).
/// Returns the track id (>= 0), or a negative [`MoonlittStatus`].
#[no_mangle]
pub extern "C" fn moonlitt_runtime_add_track(
    rt: *mut RuntimeHandle,
    engine_handle: *mut EngineHandle,
    channel_mask: c_int,
) -> c_int {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        let h = match rt_mut(rt) {
            Ok(h) => h,
            Err(s) => return s,
        };
        let path = unsafe { engine_handle.as_ref() }.and_then(|e| e.loaded_path.clone());
        let backend = match take_backend(engine_handle) {
            Ok(b) => b,
            Err(s) => return s,
        };
        let source = crate::shadow::ShadowSource::from_backend(&*backend, path);
        let id = h.runtime.add_track(backend, channel_mask as u16);
        h.shadow.add_track(id, channel_mask as u16, source);
        id as c_int
    })
}

/// Remove a track at runtime (notes are silenced first). Unknown ids
/// are ignored by the audio thread.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_remove_track(
    rt: *mut RuntimeHandle,
    track_id: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            if track_id < 0 {
                set_last_error_static(c"track_id must be >= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let h = rt_mut(rt)?;
            h.runtime.remove_track(track_id as u32);
            h.shadow.remove_track(track_id as u32);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Add an insert effect to a track at runtime. **Consumes** the backend.
/// Returns the insert id (>= 0), or a negative [`MoonlittStatus`].
#[no_mangle]
pub extern "C" fn moonlitt_runtime_add_insert(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    engine_handle: *mut EngineHandle,
) -> c_int {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        let h = match rt_mut(rt) {
            Ok(h) => h,
            Err(s) => return s,
        };
        if track_id < 0 {
            set_last_error_static(c"track_id must be >= 0");
            return MOONLITT_ERR_INVALID_ARG;
        }
        let path = unsafe { engine_handle.as_ref() }.and_then(|e| e.loaded_path.clone());
        let backend = match take_backend(engine_handle) {
            Ok(b) => b,
            Err(s) => return s,
        };
        let source = crate::shadow::ShadowSource::from_backend(&*backend, path);
        let id = h.runtime.add_insert(track_id as u32, backend);
        h.shadow.add_insert(track_id as u32, id, source);
        id as c_int
    })
}

/// Remove an insert effect from a track at runtime. Unknown ids are
/// ignored by the audio thread.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_remove_insert(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    insert_id: c_int,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            if track_id < 0 || insert_id < 0 {
                set_last_error_static(c"track_id and insert_id must be >= 0");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let h = rt_mut(rt)?;
            h.runtime.remove_insert(track_id as u32, insert_id as u32);
            h.shadow.remove_insert(track_id as u32, insert_id as u32);
            Ok(MOONLITT_OK)
        })())
    })
}

/// Add a send bus at runtime. **Consumes** the backend. Returns the bus
/// id (>= 0), or a negative [`MoonlittStatus`].
#[no_mangle]
pub extern "C" fn moonlitt_runtime_add_send_bus(
    rt: *mut RuntimeHandle,
    engine_handle: *mut EngineHandle,
) -> c_int {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        let h = match rt_mut(rt) {
            Ok(h) => h,
            Err(s) => return s,
        };
        let path = unsafe { engine_handle.as_ref() }.and_then(|e| e.loaded_path.clone());
        let backend = match take_backend(engine_handle) {
            Ok(b) => b,
            Err(s) => return s,
        };
        let source = crate::shadow::ShadowSource::from_backend(&*backend, path);
        let id = h.runtime.add_send_bus(backend);
        h.shadow.add_send_bus(id, source);
        id as c_int
    })
}

// ---------------------------------------------------------------------------
// MIDI device listing
// ---------------------------------------------------------------------------

/// List available MIDI input devices as JSON `[{"id","name"}, …]`.
///
/// Ownership: caller frees with `moonlitt_free_string`. Returns NULL +
/// last-error when the MIDI subsystem is unavailable.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_list_midi_inputs() -> *mut c_char {
    ffi_guard!(std::ptr::null_mut(), {
        match Runtime::list_midi_inputs() {
            Ok(devices) => {
                let entries: Vec<String> = devices
                    .iter()
                    .map(|d| format!(r#"{{"id":{},"name":"{}"}}"#, d.id, json_escape(&d.name)))
                    .collect();
                to_c_string(&format!("[{}]", entries.join(",")))
            }
            Err(e) => {
                set_last_error(format!("MIDI subsystem unavailable: {e}"));
                std::ptr::null_mut()
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Transport (sequencer control)
// ---------------------------------------------------------------------------

/// Start sequencer playback from the current position.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_play(rt: *mut RuntimeHandle) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let h = rt_mut(rt)?;
            h.runtime.play();
            Ok(MOONLITT_OK)
        })())
    })
}

/// Pause sequencer playback, keeping the position.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_pause(rt: *mut RuntimeHandle) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let h = rt_mut(rt)?;
            h.runtime.pause_playback();
            Ok(MOONLITT_OK)
        })())
    })
}

/// Stop sequencer playback and rewind to the start.
/// (The audio stream itself is controlled by
/// `moonlitt_runtime_start_audio` / `moonlitt_runtime_stop_audio`.)
#[no_mangle]
pub extern "C" fn moonlitt_runtime_stop(rt: *mut RuntimeHandle) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let h = rt_mut(rt)?;
            h.runtime.stop_playback();
            Ok(MOONLITT_OK)
        })())
    })
}

// ---------------------------------------------------------------------------
// Session save
// ---------------------------------------------------------------------------

/// Save the runtime's full session — mixer topology, levels, plugin
/// patch states and transport flags — to a `.mlsession` JSON file that
/// `moonlitt_session_load_from_file` restores.
///
/// Plugin states are pulled through shared handles with a brief
/// per-plugin lock; sample streamers (Spectrasonics) can hold that lock
/// up to ~1 s, during which the audio thread renders silence rather
/// than stalling. Save is a user-initiated, rare operation — schedule
/// it accordingly.
///
/// Errors: `INVALID_ARG` (NULL handle/path), `STATE` (a plugin failed
/// to serialise — message names the track), `IO` (file write failed).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_save_session(
    rt: *mut RuntimeHandle,
    path: *const c_char,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            let h = rt_mut(rt)?;
            let path = match unsafe { cstr_to_str(path) } {
                Some(p) => p,
                None => {
                    set_last_error_static(c"path is NULL or not valid UTF-8");
                    return Err(MOONLITT_ERR_INVALID_ARG);
                }
            };
            let session = match h.shadow.to_session(h.runtime.is_metronome_enabled()) {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(e);
                    return Err(crate::error::MOONLITT_ERR_STATE);
                }
            };
            match session.save_to_file(path) {
                Ok(()) => Ok(MOONLITT_OK),
                Err(e) => {
                    set_last_error(format!("write session '{path}': {e}"));
                    Err(crate::error::MOONLITT_ERR_IO)
                }
            }
        })())
    })
}

// ---------------------------------------------------------------------------
// Queries (atomic reads — safe to poll from the control thread)
// ---------------------------------------------------------------------------

/// 1 while the audio output stream is running, 0 otherwise (including
/// NULL handles).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_is_running(rt: *mut RuntimeHandle) -> c_int {
    ffi_guard!(0, {
        match unsafe { rt.as_ref() } {
            Some(h) => h.runtime.is_audio_running() as c_int,
            None => 0,
        }
    })
}

/// Master-bus sample peak of the most recent audio block, written to
/// `*out_left` / `*out_right` (linear, 0.0 = silence, 1.0 = full scale).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_master_peak(
    rt: *mut RuntimeHandle,
    out_left: *mut c_float,
    out_right: *mut c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            if out_left.is_null() || out_right.is_null() {
                set_last_error_static(c"out_left / out_right must not be NULL");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let h = rt_mut(rt)?;
            let (l, r) = h.runtime.master_peak();
            unsafe {
                *out_left = l;
                *out_right = r;
            }
            Ok(MOONLITT_OK)
        })())
    })
}

/// Master-bus RMS of the most recent audio block, written to
/// `*out_left` / `*out_right` (linear).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_master_rms(
    rt: *mut RuntimeHandle,
    out_left: *mut c_float,
    out_right: *mut c_float,
) -> MoonlittStatus {
    ffi_guard!(MOONLITT_ERR_PANIC, {
        status((|| {
            if out_left.is_null() || out_right.is_null() {
                set_last_error_static(c"out_left / out_right must not be NULL");
                return Err(MOONLITT_ERR_INVALID_ARG);
            }
            let h = rt_mut(rt)?;
            let (l, r) = h.runtime.master_rms();
            unsafe {
                *out_left = l;
                *out_right = r;
            }
            Ok(MOONLITT_OK)
        })())
    })
}
