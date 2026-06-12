//! Plugin discovery types.

/// Format of a discovered audio plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginFormat {
    Vst3,
    Clap,
    Sf2,
    Sfz,
}

/// Information about a discovered audio plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub path: String,
    pub format: PluginFormat,
    /// Can this plug-in act as a sound source? `false` for effect-only
    /// plug-ins (VST3 subcategory "Fx" without "Instrument") so
    /// instrument pickers can hide them. Formats without reliable
    /// metadata default to `true`.
    pub is_instrument: bool,
}
