//! WAV file reading — deinterleaves stereo into separate L/R f32 buffers.
//!
//! Supports 16/24/32-bit PCM and 32-bit IEEE float (the only formats
//! moonlitt itself emits).

use crate::AnalyzeError;
use std::path::Path;

pub fn read_stereo(path: &Path) -> Result<(Vec<f32>, Vec<f32>, u32), AnalyzeError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    if spec.channels != 2 {
        return Err(AnalyzeError::NotStereo(spec.channels));
    }

    let interleaved: Vec<f32> = match (spec.bits_per_sample, spec.sample_format) {
        (32, hound::SampleFormat::Float) => {
            reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?
        }
        (16, hound::SampleFormat::Int) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<Vec<_>, _>>()?,
        (24, hound::SampleFormat::Int) | (32, hound::SampleFormat::Int) => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / (1u32 << (spec.bits_per_sample - 1)) as f32))
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(AnalyzeError::UnsupportedFormat(
                spec.bits_per_sample,
                spec.sample_format,
            ))
        }
    };

    let frames = interleaved.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for i in 0..frames {
        left.push(interleaved[i * 2]);
        right.push(interleaved[i * 2 + 1]);
    }

    Ok((left, right, spec.sample_rate))
}
