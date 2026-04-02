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
