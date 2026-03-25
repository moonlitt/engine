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
}
