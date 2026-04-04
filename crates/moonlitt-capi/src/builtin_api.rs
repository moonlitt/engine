//! Built-in effect factories + Mixer pre-creation API.
//!
//! Allows C callers to:
//! 1. Create built-in effect backends (EQ, Compressor, Reverb) without loading a file
//! 2. Pre-build a Mixer with tracks before creating a Runtime

use crate::engine_api::EngineHandle;
use crate::runtime_api::RuntimeHandle;
use crate::util::cstr_to_str;
use moonlitt_audio_io::mixer::Mixer;
use moonlitt_audio_io::Runtime;
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
    let eq = moonlitt_effects::ParametricEq::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(eq)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
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
    let comp = moonlitt_effects::Compressor::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(comp)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
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
    let reverb = moonlitt_effects::DattorroReverb::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(reverb)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a brickwall limiter engine with lookahead.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_limiter(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let limiter = moonlitt_effects::Limiter::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(limiter)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a noise gate / expander engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_gate(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let gate = moonlitt_effects::Gate::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(gate)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a de-esser engine (sibilance reduction).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_deesser(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let deesser = moonlitt_effects::DeEsser::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(deesser)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a stereo delay engine with tempo sync and ping-pong.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_stereo_delay(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let delay = moonlitt_effects::StereoDelay::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(delay)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a 4-voice chorus engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_chorus(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let chorus = moonlitt_effects::Chorus::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(chorus)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a through-zero flanger engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_flanger(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let flanger = moonlitt_effects::Flanger::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(flanger)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create an N-stage allpass phaser engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_phaser(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let phaser = moonlitt_effects::Phaser::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(phaser)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a tremolo engine with tempo sync and stereo auto-pan.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_tremolo(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let tremolo = moonlitt_effects::Tremolo::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(tremolo)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a gain utility engine (gain, polarity invert, mono sum).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_gain(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let gain = moonlitt_effects::Gain::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(gain)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
        last_error: None,
        last_error_cstring: None,
    }))
}

/// Create a stereo width processor engine (mid/side encoding).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_stereo_width(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    let sr = sample_rate.max(1) as u32;
    let bs = buffer_size.max(1) as u32;
    let sw = moonlitt_effects::StereoWidth::new(sr);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(sw)),
        sample_rate: sr,
        buffer_size: bs,
        loaded_path: None,
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

    let conv = moonlitt_effects::Convolver::from_ir(&ir, sr, bs);
    Box::into_raw(Box::new(EngineHandle {
        backend: Some(Box::new(conv)),
        sample_rate: sr,
        buffer_size: bs as u32,
        loaded_path: None,
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
/// The backend is consumed on success (taken from the EngineHandle).
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
    let backend = match eh.backend.take() {
        Some(b) => b,
        None => return -1,
    };
    mixer.add_track(backend, channel_mask as u16) as c_int
}

/// Add a send bus to a pre-built mixer. Returns bus ID, or -1 on error.
/// The backend is consumed on success.
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
    let backend = match eh.backend.take() {
        Some(b) => b,
        None => return -1,
    };
    mixer.add_send_bus(backend) as c_int
}

/// Add an insert effect to a track in a pre-built mixer. Returns insert ID, or -1 on error.
/// The backend is consumed on success.
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
    let backend = match eh.backend.take() {
        Some(b) => b,
        None => return -1,
    };
    match mixer.add_insert(track_id as u32, backend) {
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
/// Memory: ~1x SF2 size instead of 16x SF2 size.
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
        let backend = match moonlitt_engine::create_from_shared_sf2(cloned_font, sr) {
            Ok(b) => b,
            Err(_) => return std::ptr::null_mut(),
        };
        mixer.add_track(backend, 1 << ch);
    }

    // Create runtime from mixer
    match Runtime::with_mixer(mixer, bs) {
        Ok(runtime) => Box::into_raw(Box::new(RuntimeHandle { runtime })),
        Err(_) => std::ptr::null_mut(),
    }
}
