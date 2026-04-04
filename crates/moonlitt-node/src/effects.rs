//! Effect factory functions.
//!
//! Each factory returns a `Backend` that can be used as a track insert
//! via `Session.addInsert()`.

use napi::Result;
use napi_derive::napi;

use crate::engine::Backend;

/// Create an 8-band parametric EQ.
#[napi]
pub fn create_eq(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::ParametricEq::new(sample_rate))),
    }
}

/// Create a dynamics compressor.
#[napi]
pub fn create_compressor(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Compressor::new(sample_rate))),
    }
}

/// Create a Freeverb reverb.
#[napi]
pub fn create_reverb(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Reverb::new(sample_rate))),
    }
}

/// Create a Dattorro plate reverb.
#[napi]
pub fn create_dattorro_reverb(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::DattorroReverb::new(sample_rate))),
    }
}

/// Create a convolution reverb from an impulse response buffer.
///
/// `ir` contains the impulse response samples as f64 (converted to f32 internally).
/// `block_size` determines processing latency (typically 512 or 1024).
#[napi]
pub fn create_convolver(ir: Vec<f64>, sample_rate: u32, block_size: u32) -> Result<Backend> {
    if ir.is_empty() {
        return Err(napi::Error::from_reason("IR buffer must not be empty"));
    }
    let ir_f32: Vec<f32> = ir.iter().map(|&v| v as f32).collect();
    Ok(Backend {
        inner: Some(Box::new(moonlitt_effects::Convolver::from_ir(
            &ir_f32,
            sample_rate,
            block_size as usize,
        ))),
    })
}

/// Create a brickwall limiter with lookahead.
#[napi]
pub fn create_limiter(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Limiter::new(sample_rate))),
    }
}

/// Create a noise gate / expander.
#[napi]
pub fn create_gate(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Gate::new(sample_rate))),
    }
}

/// Create a de-esser (sibilance reduction).
#[napi]
pub fn create_deesser(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::DeEsser::new(sample_rate))),
    }
}

/// Create a stereo delay with tempo sync and ping-pong.
#[napi]
pub fn create_stereo_delay(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::StereoDelay::new(sample_rate))),
    }
}

/// Create a 4-voice chorus.
#[napi]
pub fn create_chorus(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Chorus::new(sample_rate))),
    }
}

/// Create a through-zero flanger.
#[napi]
pub fn create_flanger(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Flanger::new(sample_rate))),
    }
}

/// Create an N-stage allpass phaser.
#[napi]
pub fn create_phaser(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Phaser::new(sample_rate))),
    }
}

/// Create a tremolo with tempo sync and stereo auto-pan.
#[napi]
pub fn create_tremolo(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Tremolo::new(sample_rate))),
    }
}

/// Create a gain utility (gain, polarity invert, mono sum).
#[napi]
pub fn create_gain(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::Gain::new(sample_rate))),
    }
}

/// Create a stereo width processor (mid/side encoding).
#[napi]
pub fn create_stereo_width(sample_rate: u32) -> Backend {
    Backend {
        inner: Some(Box::new(moonlitt_effects::StereoWidth::new(sample_rate))),
    }
}
