//! Built-in effect factories + Mixer pre-creation API.
//!
//! Allows C callers to:
//! 1. Create built-in effect engines (EQ, Compressor, Reverb) without loading a file
//! 2. Pre-build a Mixer with tracks before creating a Runtime

use crate::engine_api::EngineHandle;
use crate::runtime_api::RuntimeHandle;
use crate::util::cstr_to_str;
use moonlitt_engine::engine::Engine;
use moonlitt_runtime::mixer::Mixer;
use moonlitt_runtime::Runtime;
use std::ffi::{c_char, c_int};

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

/// Create a stereo reverb engine (Dattorro plate reverb algorithm).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_reverb(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let reverb = moonlitt_reverb::DattorroReverb::new(sr);
    let engine = Engine::from_backend(Box::new(reverb), sr, bs);
    Box::into_raw(Box::new(EngineHandle {
        engine: Some(engine),
        last_error: None,
        last_error_cstring: None,
    }))
}

// ---------------------------------------------------------------------------
// WAV loader: reads a mono WAV file and returns f32 samples.
// Uses hound for robust PCM-16, PCM-24, PCM-32 and IEEE-float-32 support.
// ---------------------------------------------------------------------------

fn load_wav_mono(path: &str) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();

    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => {
            let samples: Vec<f32> = reader
                .samples::<f32>()
                .step_by(spec.channels as usize)   // take first channel (mono/L)
                .filter_map(|s| s.ok())
                .collect();
            if samples.is_empty() { None } else { Some(samples) }
        }
        (hound::SampleFormat::Int, bits) => {
            let scale = 1.0 / (1u64 << (bits - 1)) as f32;
            let samples: Vec<f32> = reader
                .samples::<i32>()
                .step_by(spec.channels as usize)
                .filter_map(|s| s.ok())
                .map(|s| s as f32 * scale)
                .collect();
            if samples.is_empty() { None } else { Some(samples) }
        }
        _ => None,
    }
}

/// Create a convolution reverb engine from a mono WAV impulse response file.
///
/// `ir_path`     — UTF-8 path to a WAV file (PCM-16/24/32 or IEEE float-32, any channel count;
///                 only the first channel is used as the mono IR).
/// `sample_rate` — playback sample rate in Hz.
/// `buffer_size` — processing block size (determines FFT partition size and PDC latency).
///
/// Returns an EngineHandle on success, or null if the file cannot be read or parsed.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_convolver(
    ir_path: *const c_char,
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let path = match unsafe { cstr_to_str(ir_path) } {
        Some(p) => p,
        None => return std::ptr::null_mut(),
    };
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as usize;

    let ir = match load_wav_mono(path) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let conv = moonlitt_convolver::Convolver::from_ir(&ir, sr, bs);
    let engine = Engine::from_backend(Box::new(conv), sr, bs as u32);
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

// ---------------------------------------------------------------------------
// Shared multi-track: load SF2 once, clone for 16 tracks (Arc-shared samples)
// ---------------------------------------------------------------------------

/// Create a 16-track mixer + runtime from a single SF2 file.
/// Loads the SF2 ONCE, clones the SoundFont for each channel (Arc-shared sample data).
/// Memory: ~1× SF2 size instead of 16× SF2 size.
/// Returns a RuntimeHandle ready to use, or null on failure.
#[no_mangle]
pub extern "C" fn moonlitt_multitrack_create(
    sf2_path: *const c_char,
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut RuntimeHandle {
    let path = match unsafe { cstr_to_str(sf2_path) } {
        Some(p) => p,
        None => return std::ptr::null_mut(),
    };
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;

    // Load SF2 once
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return std::ptr::null_mut(),
    };
    let font = match oxisynth::SoundFont::load(&mut file) {
        Ok(f) => f,
        Err(_) => return std::ptr::null_mut(),
    };

    // Create mixer with 16 tracks, each cloning the font (Arc-shared)
    let mut mixer = Mixer::new(sr, bs as usize);
    for ch in 0u16..16 {
        let cloned_font = font.clone(); // Only clones Arc pointers, not sample data
        let engine = match Engine::from_shared_sf2(cloned_font, sr, bs) {
            Ok(e) => e,
            Err(_) => return std::ptr::null_mut(),
        };
        mixer.add_track(engine, 1 << ch);
    }

    // Create runtime from mixer
    match Runtime::with_mixer(mixer, bs) {
        Ok(runtime) => Box::into_raw(Box::new(RuntimeHandle { runtime })),
        Err(_) => std::ptr::null_mut(),
    }
}
