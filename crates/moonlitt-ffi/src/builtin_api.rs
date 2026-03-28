//! Built-in effect factories + Mixer pre-creation API.
//!
//! Allows C callers to:
//! 1. Create built-in effect engines (EQ, Compressor, Reverb) without loading a file
//! 2. Pre-build a Mixer with tracks before creating a Runtime

use crate::engine_api::EngineHandle;
use crate::runtime_api::RuntimeHandle;
use moonlitt_engine::engine::Engine;
use moonlitt_runtime::mixer::Mixer;
use moonlitt_runtime::Runtime;
use std::ffi::c_int;

// ---------------------------------------------------------------------------
// Built-in effect engine factories
// ---------------------------------------------------------------------------

/// Create an 8-band parametric EQ engine.
/// Returns an EngineHandle ready for use as an insert or send bus effect.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_eq(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let eq = moonlitt_eq::ParametricEq::new(sr);
    let engine = Engine::from_backend(Box::new(eq), sr, bs);
    Box::into_raw(Box::new(EngineHandle {
        engine: Some(engine),
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a dynamics compressor engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_compressor(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let comp = moonlitt_compressor::Compressor::new(sr);
    let engine = Engine::from_backend(Box::new(comp), sr, bs);
    Box::into_raw(Box::new(EngineHandle {
        engine: Some(engine),
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a stereo reverb engine (Freeverb algorithm).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_reverb(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let reverb = moonlitt_reverb::Reverb::new(sr);
    let engine = Engine::from_backend(Box::new(reverb), sr, bs);
    Box::into_raw(Box::new(EngineHandle {
        engine: Some(engine),
        last_error: None,
        last_error_cstring: None,
    }))
}

// ---------------------------------------------------------------------------
// Mixer pre-creation handle
// ---------------------------------------------------------------------------

/// Opaque mixer handle for pre-building a mixer before creating a Runtime.
pub struct MixerHandle {
    pub(crate) mixer: Option<Mixer>,
}

/// Create a new Mixer. Returns an opaque MixerHandle.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_create(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut MixerHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as usize;
    let mixer = Mixer::new(sr, bs);
    Box::into_raw(Box::new(MixerHandle {
        mixer: Some(mixer),
    }))
}

/// Destroy a mixer handle. Safe to call with null.
/// Only needed if the mixer was NOT consumed by `moonlitt_runtime_create_from_mixer`.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_destroy(m: *mut MixerHandle) {
    if !m.is_null() {
        unsafe { drop(Box::from_raw(m)); }
    }
}

/// Add a track to a pre-built mixer. Returns track ID, or -1 on error.
/// The engine is consumed on success (taken from the EngineHandle).
#[no_mangle]
pub extern "C" fn moonlitt_mixer_add_track(
    m: *mut MixerHandle,
    engine_handle: *mut EngineHandle,
    channel_mask: c_int,
) -> c_int {
    let mixer_h = match unsafe { m.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let mixer = match mixer_h.mixer.as_mut() {
        Some(m) => m,
        None => return -1,
    };
    let eh = match unsafe { engine_handle.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let engine = match eh.engine.take() {
        Some(e) => e,
        None => return -1,
    };
    mixer.add_track(engine, channel_mask as u16) as c_int
}

/// Add a send bus to a pre-built mixer. Returns bus ID, or -1 on error.
/// The engine is consumed on success.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_add_send_bus(
    m: *mut MixerHandle,
    engine_handle: *mut EngineHandle,
) -> c_int {
    let mixer_h = match unsafe { m.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let mixer = match mixer_h.mixer.as_mut() {
        Some(m) => m,
        None => return -1,
    };
    let eh = match unsafe { engine_handle.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let engine = match eh.engine.take() {
        Some(e) => e,
        None => return -1,
    };
    mixer.add_send_bus(engine) as c_int
}

/// Add an insert effect to a track in a pre-built mixer. Returns insert ID, or -1 on error.
/// The engine is consumed on success.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_add_insert(
    m: *mut MixerHandle,
    track_id: c_int,
    engine_handle: *mut EngineHandle,
) -> c_int {
    let mixer_h = match unsafe { m.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let mixer = match mixer_h.mixer.as_mut() {
        Some(m) => m,
        None => return -1,
    };
    let eh = match unsafe { engine_handle.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    let engine = match eh.engine.take() {
        Some(e) => e,
        None => return -1,
    };
    match mixer.add_insert(track_id as u32, engine) {
        Some(id) => id as c_int,
        None => -1,
    }
}

/// Create a Runtime from a pre-built Mixer.
/// The mixer is consumed. Returns a RuntimeHandle, or null on failure.
#[no_mangle]
pub extern "C" fn moonlitt_runtime_create_from_mixer(
    m: *mut MixerHandle,
    buffer_size: c_int,
) -> *mut RuntimeHandle {
    let mixer_h = match unsafe { m.as_mut() } {
        Some(h) => h,
        None => return std::ptr::null_mut(),
    };
    let mixer = match mixer_h.mixer.take() {
        Some(m) => m,
        None => return std::ptr::null_mut(),
    };
    match Runtime::with_mixer(mixer, buffer_size.max(1) as u32) {
        Ok(runtime) => Box::into_raw(Box::new(RuntimeHandle { runtime })),
        Err(_) => std::ptr::null_mut(),
    }
}
