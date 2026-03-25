//! AudioBackend trait — the core abstraction for all audio engines.

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
    fn set_volume(&mut self, volume: f32);
    fn sample_rate(&self) -> u32;

    // Optional capabilities
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
