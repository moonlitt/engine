//! # moonlitt-analyze
//!
//! Objective audio measurement for regression testing — no ears required.
//!
//! Reports peak, true peak, RMS, DC offset, EBU R128 loudness (integrated /
//! short-term / momentary / LRA), silence segments, and NaN/Inf anomalies.
//!
//! ```no_run
//! use moonlitt_analyze::analyze_wav;
//! let report = analyze_wav("output.wav").unwrap();
//! println!("{report}");
//! ```

mod anomaly;
mod loudness;
mod peak;
mod report;
mod wav;

pub use report::{Anomalies, Channels, LoudnessStats, PeakStats, Report, RmsStats};

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum AnalyzeError {
    #[error("WAV decode error: {0}")]
    Wav(#[from] hound::Error),

    #[error("EBU R128 error: {0}")]
    Ebur128(#[from] ebur128::Error),

    #[error("WAV must be stereo (got {0} channels)")]
    NotStereo(u16),

    #[error("WAV must be 16/24/32-bit int or 32-bit float (got {0}-bit {1:?})")]
    UnsupportedFormat(u16, hound::SampleFormat),
}

/// Analyze a WAV file on disk.
pub fn analyze_wav<P: AsRef<Path>>(path: P) -> Result<Report, AnalyzeError> {
    let (left, right, sample_rate) = wav::read_stereo(path.as_ref())?;
    Ok(analyze_stereo(&left, &right, sample_rate)?)
}

/// Analyze an in-memory stereo buffer.
pub fn analyze_stereo(
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<Report, ebur128::Error> {
    let frames = left.len().min(right.len());
    let duration_sec = frames as f64 / sample_rate as f64;

    let peak_stats = peak::measure(&left[..frames], &right[..frames]);
    let loudness_stats = loudness::measure(&left[..frames], &right[..frames], sample_rate)?;
    let anomalies = anomaly::scan(&left[..frames], &right[..frames], sample_rate);

    Ok(Report {
        sample_rate,
        channels: Channels::Stereo,
        frames,
        duration_sec,
        peak: peak_stats,
        rms: RmsStats {
            l_dbfs: peak::rms_dbfs(&left[..frames]),
            r_dbfs: peak::rms_dbfs(&right[..frames]),
        },
        loudness: loudness_stats,
        anomalies,
    })
}
