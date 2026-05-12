//! # moonlitt-core
//!
//! Core traits and types shared across all moonlitt crates.
//!
//! `AudioBackend` is the central abstraction — every audio engine
//! (sampler, VST3, CLAP) implements it. This crate exists to break
//! the cyclic dependency between moonlitt-engine and moonlitt-sampler.

mod caps;
mod event;
mod host;
mod null_backend;

pub use caps::BackendCaps;
pub use event::{AudioEvent, TimedEvent};
pub use host::{AudioCallback, AudioHost};
pub use null_backend::NullBackend;

/// All backends implement this trait. Public — community can extend.
pub trait AudioBackend: Send {
    fn info(&self) -> BackendInfo;
    fn load(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>>;
    fn unload(&mut self);

    // MIDI
    fn note_on(&mut self, channel: u8, note: u8, velocity: u8);
    fn note_off(&mut self, channel: u8, note: u8);
    fn cc(&mut self, channel: u8, cc: u8, value: u8);
    fn pitch_bend(&mut self, channel: u8, value: i16);
    fn program_change(&mut self, channel: u8, program: u8);
    fn all_notes_off(&mut self);

    // Audio
    fn render(&mut self, left: &mut [f32], right: &mut [f32]);
    /// Process audio as an effect (audio in -> audio out). Default: copy input to output.
    fn process_effect(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        out_l[..in_l.len()].copy_from_slice(in_l);
        out_r[..in_r.len()].copy_from_slice(in_r);
    }
    fn set_volume(&mut self, volume: f32);
    fn sample_rate(&self) -> u32;

    /// Report processing latency in samples.
    /// Used for Plugin Delay Compensation (PDC).
    /// Default: 0 (no latency).
    fn latency(&self) -> u32 { 0 }

    // Parameters — backends opt in by overriding these defaults
    fn param_count(&self) -> u32 { 0 }
    fn param_info(&self, _index: u32) -> Option<ParamInfo> { None }
    fn get_param(&self, _id: u32) -> Option<f64> { None }
    fn set_param(&mut self, _id: u32, _value: f64) {}
    fn param_display(&self, _id: u32, _value: f64) -> Option<String> { None }

    // Presets
    fn presets(&self) -> Vec<PresetInfo> { vec![] }
    fn load_preset(&mut self, _id: i32) -> Result<(), Box<dyn std::error::Error>> {
        Err("not supported".into())
    }
    fn save_state(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Err("not supported".into())
    }
    fn load_state(&mut self, _data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        Err("not supported".into())
    }

    /// Run `num_blocks` silent process cycles. Sample-streaming back-ends
    /// (Spectrasonics Keyscape/Omnisphere, Kontakt-class instruments)
    /// load patches asynchronously after `load_state` — notes fired in
    /// the loading window get silently dropped. Calling this between
    /// `load_state` and the first `note_on` lets the streamer's
    /// pipeline finish bringing the patch online.
    ///
    /// Default: no-op (synths, native back-ends don't need warm-up).
    /// Callers may always invoke this safely — the wasted DSP for
    /// non-streaming back-ends is sub-millisecond per session.
    fn warm_up(&mut self, _num_blocks: usize) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Suggested `warm_up` block count for this back-end after
    /// `load_state`. Session persistence captures this value alongside
    /// the state blob so restore can run the right amount of warm-up
    /// without the caller knowing plug-in internals.
    ///
    /// Back-ends override this when they ship asynchronously-loading
    /// content (sample streamers identify themselves here by returning
    /// a non-zero value). Default 0 means "no warm-up needed".
    fn recommended_warm_up_blocks(&self) -> usize { 0 }

    // Sidechain — effects opt in by overriding these defaults

    /// Provide external sidechain audio for this effect.
    /// Called by the mixer before `process_effect()` each render cycle.
    /// Effects that support sidechain override this to store the buffers internally.
    /// Default: ignore (use internal sidechain).
    fn set_sidechain(&mut self, _left: &[f32], _right: &[f32]) {}

    /// Whether this effect supports external sidechain input.
    fn supports_sidechain(&self) -> bool { false }
}

pub struct BackendInfo {
    pub name: &'static str,
    pub backend_type: BackendType,
    pub extensions: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    Sampler,
    PluginHost,
}

pub struct PresetInfo {
    pub id: i32,
    pub name: String,
}

/// Describes a single controllable parameter.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    /// Unique ID within this backend instance.
    pub id: u32,
    /// Display name (e.g., "Reverb Room Size").
    pub name: String,
    /// UI grouping (e.g., "Reverb", "Chorus", "Dynamics").
    pub group: String,
    /// Minimum value.
    pub min: f64,
    /// Maximum value.
    pub max: f64,
    /// Default value.
    pub default: f64,
    /// 0 = continuous, >0 = discrete steps.
    pub step_count: u32,
    /// Parameter flags.
    pub flags: ParamFlags,
}

bitflags::bitflags! {
    /// Parameter behavior flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ParamFlags: u32 {
        const HIDDEN   = 1 << 0;
        const READONLY = 1 << 1;
        const STEPPED  = 1 << 2;
    }
}
