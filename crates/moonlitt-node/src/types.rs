//! Shared data-transfer types for the Node.js binding.

use napi_derive::napi;

/// Discovered audio plugin metadata.
#[napi(object)]
pub struct PluginInfo {
    pub name: String,
    pub path: String,
    pub format: String,
}

/// Peak metering levels for a stereo channel.
#[napi(object)]
pub struct TrackLevels {
    pub peak_l: f64,
    pub peak_r: f64,
}

/// Available MIDI input device.
#[napi(object)]
pub struct MidiDevice {
    pub id: u32,
    pub name: String,
}

/// Summary of a MIDI file's contents — channels in use and rough duration.
/// Used by the multi-track import flow to spin up one DAW track per channel.
#[napi(object)]
pub struct MidiInfo {
    /// Distinct MIDI channels (0..15) that contain at least one note-on event,
    /// sorted ascending. Channels with only meta events are not listed.
    pub channels: Vec<u32>,
    /// Number of MIDI tracks (chunks) in the SMF file.
    pub track_count: u32,
    /// Approximate duration in bars assuming 4/4 — best-effort estimate.
    pub length_bars: f64,
}

/// Metadata for a single backend parameter.
#[napi(object)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub group: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// 0 = continuous, >0 = number of discrete steps.
    pub step_count: u32,
}
