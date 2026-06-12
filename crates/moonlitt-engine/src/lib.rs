//! # moonlitt-engine
//!
//! Unified audio engine with multiple backends.
//!
//! Built-in samplers for the free world, plugin hosting for everything else.
//!
//! Use `create()` to build a `Box<dyn AudioBackend>` from a file path,
//! or `scan_plugins()` to discover available plugins.

pub mod backend;
pub mod backends;
pub mod engine;
pub mod error;
pub mod plugin_info;

// Re-export factory functions at crate level for convenience.
#[cfg(feature = "sf2")]
pub use engine::create_from_shared_sf2;
#[cfg(feature = "sf2-sampler")]
pub use engine::create_with_sampler;
pub use engine::{
    create, create_high_quality, scan_plugins, scan_plugins_excluding_vst3, supported_formats,
};
