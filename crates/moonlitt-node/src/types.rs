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

/// Per-channel hint surfaced from the MIDI file — name (from a TrackName
/// meta event on the same MIDI track) and the first GM Program Change
/// observed on this channel. Either may be absent.
#[napi(object)]
pub struct MidiChannelInfo {
    pub channel: u32,
    /// 1-based MIDI channel number (1..16) — what the user sees.
    pub display_number: u32,
    /// First TrackName meta event in any MIDI track that emits notes on this
    /// channel, or `None` if no such name was found.
    pub track_name: Option<String>,
    /// First Program Change value (0..127) observed on this channel.
    /// For channel 10 (display 10), GM treats this as a drum kit selector.
    pub program: Option<u32>,
}

/// Summary of a MIDI file's contents.
#[napi(object)]
pub struct MidiInfo {
    /// Per-channel info for every channel that contains at least one
    /// note-on event, sorted by channel number.
    pub channels: Vec<MidiChannelInfo>,
    /// Number of MIDI tracks (chunks) in the SMF file.
    pub track_count: u32,
    /// Approximate duration in bars assuming the file's time signature
    /// (or 4/4 if none was set).
    pub length_bars: f64,
    /// First Tempo meta event, converted to BPM. None if no tempo set.
    pub tempo_bpm: Option<f64>,
    /// First TimeSignature meta event as `[numerator, denominator]`.
    /// None if no time signature event found.
    pub time_signature: Option<Vec<u32>>,
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
