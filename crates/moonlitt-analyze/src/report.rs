//! Report struct — what `analyze_wav` returns.
//!
//! Designed to round-trip through `serde_json` so snapshot tests can compare
//! the latest measurement against a baseline file.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Channels {
    Stereo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakStats {
    pub sample_peak_l_dbfs: f64,
    pub sample_peak_r_dbfs: f64,
    pub true_peak_l_dbtp: f64,
    pub true_peak_r_dbtp: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RmsStats {
    pub l_dbfs: f64,
    pub r_dbfs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoudnessStats {
    pub integrated_lufs: f64,
    pub short_term_max_lufs: f64,
    pub momentary_max_lufs: f64,
    pub lra_lu: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Anomalies {
    pub nan_count: usize,
    pub inf_count: usize,
    pub denormal_count: usize,
    pub dc_offset_l: f64,
    pub dc_offset_r: f64,
    /// (start_sec, end_sec) of regions where 100ms RMS stayed below -60 dBFS.
    pub silence_segments: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub sample_rate: u32,
    pub channels: Channels,
    pub frames: usize,
    pub duration_sec: f64,
    pub peak: PeakStats,
    pub rms: RmsStats,
    pub loudness: LoudnessStats,
    pub anomalies: Anomalies,
}

impl Report {
    /// Crest factor (peak / RMS) in dB, averaged across channels.
    /// Higher values mean more dynamic, lower values mean more compressed.
    pub fn crest_factor_db(&self) -> f64 {
        let peak = self
            .peak
            .sample_peak_l_dbfs
            .max(self.peak.sample_peak_r_dbfs);
        let rms = (self.rms.l_dbfs + self.rms.r_dbfs) / 2.0;
        peak - rms
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("Report serialization is infallible")
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Format       {:?} {} Hz, {} frames ({:.2}s)",
            self.channels, self.sample_rate, self.frames, self.duration_sec
        )?;
        writeln!(f, "─────────────────────────────────────────────")?;
        writeln!(f, "Sample peak  L {:>7.2} dBFS  R {:>7.2} dBFS",
            self.peak.sample_peak_l_dbfs, self.peak.sample_peak_r_dbfs)?;
        writeln!(f, "True peak    L {:>7.2} dBTP   R {:>7.2} dBTP",
            self.peak.true_peak_l_dbtp, self.peak.true_peak_r_dbtp)?;
        writeln!(f, "RMS          L {:>7.2} dBFS  R {:>7.2} dBFS",
            self.rms.l_dbfs, self.rms.r_dbfs)?;
        writeln!(f, "Crest factor   {:>7.2} dB", self.crest_factor_db())?;
        writeln!(f, "─────────────────────────────────────────────")?;
        writeln!(f, "Integrated     {:>7.2} LUFS", self.loudness.integrated_lufs)?;
        writeln!(f, "Short-term max {:>7.2} LUFS", self.loudness.short_term_max_lufs)?;
        writeln!(f, "Momentary max  {:>7.2} LUFS", self.loudness.momentary_max_lufs)?;
        writeln!(f, "Loudness range {:>7.2} LU",   self.loudness.lra_lu)?;
        writeln!(f, "─────────────────────────────────────────────")?;
        writeln!(f, "DC offset    L {:+.6}  R {:+.6}",
            self.anomalies.dc_offset_l, self.anomalies.dc_offset_r)?;
        writeln!(f, "NaN samples    {}", self.anomalies.nan_count)?;
        writeln!(f, "Inf samples    {}", self.anomalies.inf_count)?;
        writeln!(f, "Denormals      {}", self.anomalies.denormal_count)?;
        let silent = self.anomalies.silence_segments.len();
        if silent > 0 {
            writeln!(f, "Silence regions ({} total):", silent)?;
            for (i, (s, e)) in self.anomalies.silence_segments.iter().enumerate().take(5) {
                writeln!(f, "  {:>3}. {:.2}s → {:.2}s ({:.2}s)", i + 1, s, e, e - s)?;
            }
            if silent > 5 {
                writeln!(f, "  ... and {} more", silent - 5)?;
            }
        } else {
            writeln!(f, "Silence regions  none (>100ms below -60 dBFS)")?;
        }
        Ok(())
    }
}
