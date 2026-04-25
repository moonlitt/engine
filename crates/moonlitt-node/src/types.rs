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
