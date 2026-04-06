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
