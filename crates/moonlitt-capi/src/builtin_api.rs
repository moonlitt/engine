//! Built-in effect factories + Mixer pre-creation API.
//!
//! Lets C callers:
//!
//! 1. create built-in effect backends (EQ, compressor, reverb, …) without
//!    loading a plugin file, and
//! 2. pre-build a `Mixer` (tracks, sends, inserts) before starting a
//!    `Runtime`.
//!
//! Conventions (ABI draft 0.9): factories return an owned `EngineHandle*`
//! or NULL with the detail in `moonlitt_last_error_message()`. The
//! `moonlitt_mixer_add_*` family returns a non-negative id on success or
//! a negative [`MoonlittStatus`] on failure. Every function that accepts
//! an `EngineHandle*` **consumes the backend out of it** on success —
//! the handle itself must still be destroyed (cheap empty shell), and
//! using it again yields `MOONLITT_ERR_NOT_LOADED`.
//!
//! NOTE: factories are written out one by one instead of macro-generated
//! so cbindgen can see every symbol when emitting `include/moonlitt.h`.

use crate::engine_api::{take_backend, EngineHandle};
use crate::error::{
    ffi_guard, set_last_error, set_last_error_static, MoonlittStatus, MOONLITT_ERR_INVALID_ARG,
    MOONLITT_ERR_NOT_LOADED,
};
use crate::runtime_api::RuntimeHandle;
use crate::util::cstr_to_str;
use moonlitt_audio_io::mixer::Mixer;
use moonlitt_audio_io::Runtime;
use std::ffi::{c_char, c_int};

/// Validate (sample_rate, buffer_size); on failure set the thread-local
/// error and report `Err`.
fn audio_config(sample_rate: c_int, buffer_size: c_int) -> Result<(u32, u32), ()> {
    if sample_rate <= 0 || buffer_size <= 0 {
        set_last_error(format!(
            "invalid config: sample_rate={sample_rate}, buffer_size={buffer_size} (both must be > 0)"
        ));
        Err(())
    } else {
        Ok((sample_rate as u32, buffer_size as u32))
    }
}

// ---------------------------------------------------------------------------
// Built-in effect engine factories
//
// All factories share the contract: `sample_rate` Hz > 0, `buffer_size`
// frames > 0; returns an owned EngineHandle* (destroy or transfer it),
// or NULL with a last-error message.
// ---------------------------------------------------------------------------

/// Create an 8-band parametric EQ engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_eq(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::ParametricEq::new(sr)), sr, bs)
    })
}

/// Create a dynamics compressor engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_compressor(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Compressor::new(sr)), sr, bs)
    })
}

/// Create a stereo reverb engine (Dattorro plate algorithm).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_reverb(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::DattorroReverb::new(sr)), sr, bs)
    })
}

/// Create a brickwall limiter engine with lookahead.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_limiter(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Limiter::new(sr)), sr, bs)
    })
}

/// Create a noise gate / expander engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_gate(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Gate::new(sr)), sr, bs)
    })
}

/// Create a de-esser engine (sibilance reduction).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_deesser(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::DeEsser::new(sr)), sr, bs)
    })
}

/// Create a stereo delay engine with tempo sync and ping-pong.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_stereo_delay(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::StereoDelay::new(sr)), sr, bs)
    })
}

/// Create a 4-voice chorus engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_chorus(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Chorus::new(sr)), sr, bs)
    })
}

/// Create a through-zero flanger engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_flanger(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Flanger::new(sr)), sr, bs)
    })
}

/// Create an N-stage allpass phaser engine.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_phaser(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Phaser::new(sr)), sr, bs)
    })
}

/// Create a tremolo engine with tempo sync and stereo auto-pan.
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_tremolo(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Tremolo::new(sr)), sr, bs)
    })
}

/// Create a gain utility engine (gain, polarity invert, mono sum).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_gain(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Gain::new(sr)), sr, bs)
    })
}

/// Create a stereo width processor engine (mid/side encoding).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_stereo_width(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::StereoWidth::new(sr)), sr, bs)
    })
}

/// Create a saturator engine (5 distortion models with oversampling).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_saturator(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Saturator::new(sr)), sr, bs)
    })
}

/// Create a bitcrusher engine (sample-rate and bit-depth reduction).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_bitcrusher(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::Bitcrusher::new(sr)), sr, bs)
    })
}

/// Create a multiband compressor engine (4-band crossover + per-band dynamics).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_multiband_compressor(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(
            Box::new(moonlitt_effects::MultibandCompressor::new(sr)),
            sr,
            bs,
        )
    })
}

/// Create an auto-filter engine (LFO-modulated cutoff).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_auto_filter(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::AutoFilter::new(sr)), sr, bs)
    })
}

/// Create a pitch shifter engine (FFT-based frequency-domain shifting).
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_pitch_shifter(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        EngineHandle::with_backend(Box::new(moonlitt_effects::PitchShifter::new(sr)), sr, bs)
    })
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
                .step_by(spec.channels as usize) // take first channel (mono/L)
                .filter_map(|s| s.ok())
                .collect();
            if samples.is_empty() {
                None
            } else {
                Some(samples)
            }
        }
        (hound::SampleFormat::Int, bits) => {
            let scale = 1.0 / (1u64 << (bits - 1)) as f32;
            let samples: Vec<f32> = reader
                .samples::<i32>()
                .step_by(spec.channels as usize)
                .filter_map(|s| s.ok())
                .map(|s| s as f32 * scale)
                .collect();
            if samples.is_empty() {
                None
            } else {
                Some(samples)
            }
        }
        _ => None,
    }
}

/// Create a convolution reverb engine from a mono WAV impulse response.
///
/// * `ir_path` — UTF-8 path to a WAV (PCM-16/24/32 or float-32; only the
///   first channel is used)
/// * `sample_rate` / `buffer_size` — as for the other factories (the
///   buffer size determines FFT partition size and PDC latency)
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_convolver(
    ir_path: *const c_char,
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut EngineHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let path = match unsafe { cstr_to_str(ir_path) } {
            Some(p) => p,
            None => {
                set_last_error_static(c"ir_path is NULL or not valid UTF-8");
                return std::ptr::null_mut();
            }
        };
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        let ir = match load_wav_mono(path) {
            Some(s) => s,
            None => {
                set_last_error(format!("cannot read impulse response WAV '{path}'"));
                return std::ptr::null_mut();
            }
        };
        let conv = moonlitt_effects::Convolver::from_ir(&ir, sr, bs as usize);
        EngineHandle::with_backend(Box::new(conv), sr, bs)
    })
}

// ---------------------------------------------------------------------------
// Mixer pre-creation handle
// ---------------------------------------------------------------------------

/// Opaque mixer handle for pre-building a mixer before creating a Runtime.
pub struct MixerHandle {
    pub(crate) mixer: Option<Mixer>,
    pub(crate) sample_rate: u32,
}

/// Create a new Mixer for pre-building (tracks → sends → inserts →
/// `moonlitt_runtime_create_from_mixer`).
///
/// Ownership: free with `moonlitt_mixer_destroy` unless consumed by
/// `moonlitt_runtime_create_from_mixer`.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_create(
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut MixerHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };
        Box::into_raw(Box::new(MixerHandle {
            mixer: Some(Mixer::new(sr, bs as usize)),
            sample_rate: sr,
        }))
    })
}

/// Destroy a mixer handle. Safe to call with NULL. Only needed when the
/// mixer was NOT consumed by `moonlitt_runtime_create_from_mixer`.
#[no_mangle]
pub extern "C" fn moonlitt_mixer_destroy(m: *mut MixerHandle) {
    ffi_guard!((), {
        if !m.is_null() {
            unsafe {
                drop(Box::from_raw(m));
            }
        }
    })
}

fn mixer_mut<'a>(m: *mut MixerHandle) -> Result<&'a mut Mixer, MoonlittStatus> {
    let handle = match unsafe { m.as_mut() } {
        Some(h) => h,
        None => {
            set_last_error_static(c"mixer handle is NULL");
            return Err(MOONLITT_ERR_INVALID_ARG);
        }
    };
    match handle.mixer.as_mut() {
        Some(m) => Ok(m),
        None => {
            set_last_error_static(c"mixer already consumed by moonlitt_runtime_create_from_mixer");
            Err(MOONLITT_ERR_NOT_LOADED)
        }
    }
}

/// Add a track to a pre-built mixer. **Consumes** the backend out of
/// `engine_handle` (the empty handle must still be destroyed).
///
/// * `channel_mask` — bitmask of MIDI channels this track responds to
///   (bit N = channel N; 0xFFFF = all)
///
/// Returns the track id (>= 0), or a negative [`MoonlittStatus`].
#[no_mangle]
pub extern "C" fn moonlitt_mixer_add_track(
    m: *mut MixerHandle,
    engine_handle: *mut EngineHandle,
    channel_mask: c_int,
) -> c_int {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        let mixer = match mixer_mut(m) {
            Ok(x) => x,
            Err(s) => return s,
        };
        let backend = match take_backend(engine_handle) {
            Ok(b) => b,
            Err(s) => return s,
        };
        mixer.add_track(backend, channel_mask as u16) as c_int
    })
}

/// Add a send bus to a pre-built mixer. **Consumes** the backend.
/// Returns the bus id (>= 0), or a negative [`MoonlittStatus`].
#[no_mangle]
pub extern "C" fn moonlitt_mixer_add_send_bus(
    m: *mut MixerHandle,
    engine_handle: *mut EngineHandle,
) -> c_int {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        let mixer = match mixer_mut(m) {
            Ok(x) => x,
            Err(s) => return s,
        };
        let backend = match take_backend(engine_handle) {
            Ok(b) => b,
            Err(s) => return s,
        };
        mixer.add_send_bus(backend) as c_int
    })
}

/// Add an insert effect to a track of a pre-built mixer. **Consumes**
/// the backend. Returns the insert id (>= 0), or a negative
/// [`MoonlittStatus`] (`INVALID_ARG` when the track id is unknown).
#[no_mangle]
pub extern "C" fn moonlitt_mixer_add_insert(
    m: *mut MixerHandle,
    track_id: c_int,
    engine_handle: *mut EngineHandle,
) -> c_int {
    ffi_guard!(crate::error::MOONLITT_ERR_PANIC, {
        let mixer = match mixer_mut(m) {
            Ok(x) => x,
            Err(s) => return s,
        };
        if track_id < 0 {
            set_last_error_static(c"track_id must be >= 0");
            return MOONLITT_ERR_INVALID_ARG;
        }
        let backend = match take_backend(engine_handle) {
            Ok(b) => b,
            Err(s) => return s,
        };
        match mixer.add_insert(track_id as u32, backend) {
            Some(id) => id as c_int,
            None => {
                set_last_error(format!("unknown track id {track_id}"));
                MOONLITT_ERR_INVALID_ARG
            }
        }
    })
}

/// Create a Runtime (live audio output) from a pre-built mixer.
/// **Consumes** the mixer on success. Returns NULL + last-error on
/// failure (e.g. no audio output device).
#[no_mangle]
pub extern "C" fn moonlitt_runtime_create_from_mixer(
    m: *mut MixerHandle,
    buffer_size: c_int,
) -> *mut RuntimeHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let handle = match unsafe { m.as_mut() } {
            Some(h) => h,
            None => {
                set_last_error_static(c"mixer handle is NULL");
                return std::ptr::null_mut();
            }
        };
        if buffer_size <= 0 {
            set_last_error_static(c"buffer_size must be > 0");
            return std::ptr::null_mut();
        }
        let mixer = match handle.mixer.take() {
            Some(m) => m,
            None => {
                set_last_error_static(c"mixer already consumed");
                return std::ptr::null_mut();
            }
        };
        // Capture the shadow while the backends are still reachable —
        // Runtime::with_mixer consumes the mixer into the audio thread.
        let shadow = crate::shadow::SessionShadow::from_mixer(handle.sample_rate, &mixer);
        match Runtime::with_mixer(mixer, buffer_size as u32) {
            Ok(runtime) => Box::into_raw(Box::new(RuntimeHandle { runtime, shadow })),
            Err(e) => {
                set_last_error(format!("runtime creation failed: {e}"));
                std::ptr::null_mut()
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Shared multi-track: load SF2 once, clone for 16 tracks (Arc-shared samples)
// ---------------------------------------------------------------------------

/// Create a 16-track mixer + live runtime from a single SF2 file: the
/// SF2 is loaded ONCE and each MIDI channel's track clones the font
/// (Arc-shared sample data, ~1× memory instead of 16×).
///
/// Ownership: returns an owned RuntimeHandle* (free with
/// `moonlitt_runtime_destroy`), or NULL + last-error.
#[no_mangle]
pub extern "C" fn moonlitt_multitrack_create(
    sf2_path: *const c_char,
    sample_rate: c_int,
    buffer_size: c_int,
) -> *mut RuntimeHandle {
    ffi_guard!(std::ptr::null_mut(), {
        let path = match unsafe { cstr_to_str(sf2_path) } {
            Some(p) => p,
            None => {
                set_last_error_static(c"sf2_path is NULL or not valid UTF-8");
                return std::ptr::null_mut();
            }
        };
        let Ok((sr, bs)) = audio_config(sample_rate, buffer_size) else {
            return std::ptr::null_mut();
        };

        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                set_last_error(format!("open '{path}': {e}"));
                return std::ptr::null_mut();
            }
        };
        let font = match oxisynth::SoundFont::load(&mut file) {
            Ok(f) => f,
            Err(e) => {
                set_last_error(format!("parse SF2 '{path}': {e:?}"));
                return std::ptr::null_mut();
            }
        };

        let mut mixer = Mixer::new(sr, bs as usize);
        for ch in 0u16..16 {
            let cloned_font = font.clone(); // clones Arc pointers, not sample data
            let backend = match moonlitt_engine::create_from_shared_sf2(cloned_font, sr) {
                Ok(b) => b,
                Err(e) => {
                    set_last_error(format!("create SF2 backend for channel {ch}: {e}"));
                    return std::ptr::null_mut();
                }
            };
            mixer.add_track_with_source(backend, Some(path.to_string()), 1 << ch);
        }

        let shadow = crate::shadow::SessionShadow::from_mixer(sr, &mixer);
        match Runtime::with_mixer(mixer, bs) {
            Ok(runtime) => Box::into_raw(Box::new(RuntimeHandle { runtime, shadow })),
            Err(e) => {
                set_last_error(format!("runtime creation failed: {e}"));
                std::ptr::null_mut()
            }
        }
    })
}
