# Crate Restructuring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure moonlitt from 13 crates to 14 — merge effects, extract mixer/session/audio-io, slim engine, rename ffi→capi, add Node.js binding.

**Architecture:** Layered model (Vue runtime-core/runtime-dom). core → effects/mixer → session → platform bindings (audio-io/capi/node). Dependency arrows inward only, never reverse.

**Tech Stack:** Rust workspace, napi-rs 3 (Node.js), rtrb (lock-free SPSC), cpal (audio I/O), midir (MIDI I/O)

**Spec:** `docs/superpowers/specs/2026-04-01-crate-restructuring-design.md`

**Baseline:** All 356 tests pass: `cargo test --workspace -- --skip pianoteq --skip keyscape`

---

## Phase 1 — Foundation

### Task 1: Expand moonlitt-core

Add `BackendCaps`, `AudioEvent`, `TimedEvent`, and `AudioHost` trait to moonlitt-core. These types must live in core because mixer, session, capi, and node all need them — keeping them in a higher crate would create reverse dependencies.

**Files:**
- Modify: `crates/moonlitt-core/src/lib.rs`
- Create: `crates/moonlitt-core/src/event.rs`
- Create: `crates/moonlitt-core/src/caps.rs`
- Create: `crates/moonlitt-core/src/host.rs`
- Create: `crates/moonlitt-core/tests/core_types.rs`

- [ ] **Step 1: Write failing tests for new core types**

Create `crates/moonlitt-core/tests/core_types.rs`:

```rust
use std::mem;

#[test]
fn backend_caps_source_and_effect() {
    use moonlitt_core::BackendCaps;
    let caps = BackendCaps::SOURCE | BackendCaps::EFFECT;
    assert_eq!(caps, BackendCaps::BOTH);
    assert!(caps.contains(BackendCaps::SOURCE));
    assert!(caps.contains(BackendCaps::EFFECT));
}

#[test]
fn backend_caps_empty() {
    use moonlitt_core::BackendCaps;
    let caps = BackendCaps::empty();
    assert!(!caps.contains(BackendCaps::SOURCE));
    assert!(!caps.contains(BackendCaps::EFFECT));
}

#[test]
fn audio_event_is_copy() {
    use moonlitt_core::AudioEvent;
    let e = AudioEvent::NoteOn { channel: 0, note: 60, velocity: 100 };
    let e2 = e; // Copy
    let _ = e;  // Still usable after copy
    let _ = e2;
}

#[test]
fn audio_event_size_le_16_bytes() {
    use moonlitt_core::AudioEvent;
    assert!(
        mem::size_of::<AudioEvent>() <= 16,
        "AudioEvent is {} bytes, must be <= 16",
        mem::size_of::<AudioEvent>()
    );
}

#[test]
fn timed_event_size_le_24_bytes() {
    use moonlitt_core::TimedEvent;
    assert!(
        mem::size_of::<TimedEvent>() <= 24,
        "TimedEvent is {} bytes, must be <= 24",
        mem::size_of::<TimedEvent>()
    );
}

#[test]
fn audio_event_all_variants_roundtrip() {
    use moonlitt_core::AudioEvent;
    let events = [
        AudioEvent::NoteOn { channel: 15, note: 127, velocity: 127 },
        AudioEvent::NoteOff { channel: 0, note: 0, velocity: 0 },
        AudioEvent::CC { channel: 9, cc: 64, value: 127 },
        AudioEvent::PitchBend { channel: 0, value: -8192 },
        AudioEvent::ProgramChange { channel: 0, program: 127 },
        AudioEvent::AllNotesOff,
        AudioEvent::SetVolume(0.5),
        AudioEvent::SetParam { id: 42, value: 0.75 },
        AudioEvent::MixerTrackVolume { track_id: 255, volume: -6.0 },
        AudioEvent::MixerTrackPan { track_id: 0, pan: -1.0 },
        AudioEvent::MixerTrackTrim { track_id: 0, trim_db: 6.0 },
        AudioEvent::MixerTrackMute { track_id: 0, mute: true },
        AudioEvent::MixerTrackSolo { track_id: 0, solo: true },
        AudioEvent::MixerTrackSend { track_id: 0, bus_id: 0, level: 0.5 },
        AudioEvent::MixerMasterVolume(0.0),
        AudioEvent::InsertBypass { track_id: 0, insert_id: 0, bypass: true },
        AudioEvent::SetParamForTrack { track_id: 0, param_id: 0, value: 0.0 },
        AudioEvent::SetInsertParam { track_id: 0, insert_id: 0, param_id: 0, value: 0.0 },
        AudioEvent::SetSendBusParam { bus_id: 0, param_id: 0, value: 0.0 },
        AudioEvent::MixerTrackRoute { track_id: 0, target_id: 0xFF },
        AudioEvent::Stop,
    ];
    // All variants are Copy — this compiles only if Copy is derived
    for e in events {
        let _ = e;
    }
    assert_eq!(events.len(), 21);
}

#[test]
fn audio_host_trait_is_object_safe() {
    use moonlitt_core::AudioHost;
    // This compiles only if AudioHost is object-safe
    fn _assert_object_safe(_: &dyn AudioHost) {}
}
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
cargo test -p moonlitt-core --test core_types 2>&1
```

Expected: compilation errors — `BackendCaps`, `AudioEvent`, `TimedEvent`, `AudioHost` not found.

- [ ] **Step 3: Create `crates/moonlitt-core/src/caps.rs`**

```rust
bitflags::bitflags! {
    /// Capability flags indicating what an AudioBackend can do.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BackendCaps: u32 {
        /// Can generate audio via `render()` (synthesizers, samplers).
        const SOURCE = 0b01;
        /// Can process audio via `process_effect()` (EQ, reverb).
        const EFFECT = 0b10;
        /// Both source and effect (some VST3/CLAP plugins).
        const BOTH = Self::SOURCE.bits() | Self::EFFECT.bits();
    }
}
```

- [ ] **Step 4: Create `crates/moonlitt-core/src/event.rs`**

Port from `crates/moonlitt-runtime/src/event.rs`, keeping the exact same variants and field types (u8 for track_id/insert_id, u16 for param_id, f32 for values) to maintain the ≤16 byte size constraint:

```rust
/// Unified audio event — fits in a lock-free ring buffer slot.
///
/// All fields use small types (u8, u16, f32) to keep size ≤ 16 bytes.
#[derive(Clone, Copy, Debug)]
pub enum AudioEvent {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    CC { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: i16 },
    ProgramChange { channel: u8, program: u8 },
    AllNotesOff,
    SetVolume(f32),
    SetParam { id: u32, value: f32 },
    MixerTrackVolume { track_id: u8, volume: f32 },
    MixerTrackPan { track_id: u8, pan: f32 },
    MixerTrackTrim { track_id: u8, trim_db: f32 },
    MixerTrackMute { track_id: u8, mute: bool },
    MixerTrackSolo { track_id: u8, solo: bool },
    MixerTrackSend { track_id: u8, bus_id: u8, level: f32 },
    MixerMasterVolume(f32),
    InsertBypass { track_id: u8, insert_id: u8, bypass: bool },
    SetParamForTrack { track_id: u8, param_id: u16, value: f32 },
    SetInsertParam { track_id: u8, insert_id: u8, param_id: u16, value: f32 },
    SetSendBusParam { bus_id: u8, param_id: u16, value: f32 },
    MixerTrackRoute { track_id: u8, target_id: u8 },
    Stop,
}

/// An event with a sample-accurate delay for insertion into the render loop.
#[derive(Clone, Copy, Debug)]
pub struct TimedEvent {
    pub event: AudioEvent,
    pub delay_samples: u32,
}

// Compile-time size assertions
const _: () = assert!(std::mem::size_of::<AudioEvent>() <= 16);
const _: () = assert!(std::mem::size_of::<TimedEvent>() <= 24);
```

- [ ] **Step 5: Create `crates/moonlitt-core/src/host.rs`**

```rust
use std::error::Error;

/// Callback invoked by the platform's audio thread to fill the output buffer.
///
/// The buffer is interleaved stereo: `[L0, R0, L1, R1, ...]`.
pub type AudioCallback = Box<dyn FnMut(&mut [f32]) + Send>;

/// Platform-specific audio output driver.
///
/// Defined in core so that `moonlitt-session` can depend on it without
/// knowing about any specific platform (cpal, Web Audio, etc.).
pub trait AudioHost: Send {
    /// Start the audio output stream, invoking `callback` on the audio thread.
    fn start(&mut self, callback: AudioCallback) -> Result<(), Box<dyn Error>>;

    /// Stop the audio output stream.
    fn stop(&mut self);

    /// The sample rate negotiated with the audio device.
    fn sample_rate(&self) -> u32;

    /// The buffer size (in frames) negotiated with the audio device.
    fn buffer_size(&self) -> u32;
}
```

- [ ] **Step 6: Update `crates/moonlitt-core/src/lib.rs` to export new modules**

Add the new modules and re-exports. Keep all existing types unchanged:

```rust
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

pub use caps::BackendCaps;
pub use event::{AudioEvent, TimedEvent};
pub use host::{AudioCallback, AudioHost};

// --- Everything below is unchanged from the original lib.rs ---

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
```

- [ ] **Step 7: Run core tests — verify they pass**

```bash
cargo test -p moonlitt-core 2>&1
```

Expected: all 7 new tests pass.

- [ ] **Step 8: Run full workspace — verify no regressions**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

Expected: all 356+ tests pass. The new core types don't affect existing code yet.

- [ ] **Step 9: Commit**

```bash
git add crates/moonlitt-core/
git commit -m "feat(core): add BackendCaps, AudioEvent, TimedEvent, AudioHost trait

Foundation for crate restructuring. These types live in core
because mixer, session, and all platform bindings need them.

AudioEvent preserves the exact field types from runtime/event.rs
(u8 track_id, u16 param_id, f32 values) to maintain ≤16 byte size."
```

---

### Task 2: Create moonlitt-effects (merge 4 effect crates)

Merge moonlitt-eq, moonlitt-compressor, moonlitt-reverb, moonlitt-convolver into a single moonlitt-effects crate with feature flags. Move source files into categorized modules.

**Files:**
- Create: `crates/moonlitt-effects/Cargo.toml`
- Create: `crates/moonlitt-effects/src/lib.rs`
- Create: `crates/moonlitt-effects/src/dynamics/mod.rs`
- Create: `crates/moonlitt-effects/src/eq/mod.rs`
- Create: `crates/moonlitt-effects/src/spatial/mod.rs`
- Move: `crates/moonlitt-compressor/src/compressor.rs` → `crates/moonlitt-effects/src/dynamics/compressor.rs`
- Move: `crates/moonlitt-compressor/src/envelope.rs` → `crates/moonlitt-effects/src/dynamics/envelope.rs`
- Move: `crates/moonlitt-eq/src/biquad.rs` → `crates/moonlitt-effects/src/eq/biquad.rs`
- Move: `crates/moonlitt-eq/src/eq.rs` → `crates/moonlitt-effects/src/eq/parametric.rs`
- Move: `crates/moonlitt-reverb/src/reverb.rs` → `crates/moonlitt-effects/src/spatial/reverb.rs`
- Move: `crates/moonlitt-reverb/src/dattorro.rs` → `crates/moonlitt-effects/src/spatial/dattorro.rs`
- Move: `crates/moonlitt-reverb/src/allpass.rs` → `crates/moonlitt-effects/src/spatial/allpass.rs`
- Move: `crates/moonlitt-reverb/src/comb.rs` → `crates/moonlitt-effects/src/spatial/comb.rs`
- Move: `crates/moonlitt-reverb/src/mod_allpass.rs` → `crates/moonlitt-effects/src/spatial/mod_allpass.rs`
- Move: `crates/moonlitt-convolver/src/convolver.rs` → `crates/moonlitt-effects/src/spatial/convolver.rs`
- Move: `crates/moonlitt-convolver/src/partition.rs` → `crates/moonlitt-effects/src/spatial/partition.rs`
- Modify: `Cargo.toml` (workspace members)
- Modify: `crates/moonlitt-ffi/Cargo.toml` (depend on effects instead of 4 crates)
- Modify: `crates/moonlitt-test-suite/Cargo.toml` (depend on effects instead of 4 crates)
- Delete: `crates/moonlitt-eq/`, `crates/moonlitt-compressor/`, `crates/moonlitt-reverb/`, `crates/moonlitt-convolver/`

- [ ] **Step 1: Create moonlitt-effects directory structure**

```bash
mkdir -p crates/moonlitt-effects/src/{dynamics,eq,spatial}
```

- [ ] **Step 2: Create `crates/moonlitt-effects/Cargo.toml`**

```toml
[package]
name = "moonlitt-effects"
description = "Built-in audio effects — dynamics, EQ, spatial processing"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
rustfft = { version = "6", optional = true }

[dev-dependencies]
approx = "0.5"
rustfft = "6"

[features]
default = ["all"]
all = ["dynamics", "eq", "spatial"]

# Category level
dynamics = ["compressor"]
eq = ["parametric-eq"]
spatial = ["reverb", "convolver"]

# Individual effect level
compressor = []
parametric-eq = []
reverb = []
convolver = ["dep:rustfft"]
```

- [ ] **Step 3: Copy source files from existing effect crates into moonlitt-effects**

Copy each source file to its new location. The files themselves need minimal changes — only internal `use` paths for cross-module references within the same crate.

```bash
# Dynamics
cp crates/moonlitt-compressor/src/compressor.rs crates/moonlitt-effects/src/dynamics/compressor.rs
cp crates/moonlitt-compressor/src/envelope.rs crates/moonlitt-effects/src/dynamics/envelope.rs

# EQ
cp crates/moonlitt-eq/src/biquad.rs crates/moonlitt-effects/src/eq/biquad.rs
cp crates/moonlitt-eq/src/eq.rs crates/moonlitt-effects/src/eq/parametric.rs

# Spatial
cp crates/moonlitt-reverb/src/reverb.rs crates/moonlitt-effects/src/spatial/reverb.rs
cp crates/moonlitt-reverb/src/dattorro.rs crates/moonlitt-effects/src/spatial/dattorro.rs
cp crates/moonlitt-reverb/src/allpass.rs crates/moonlitt-effects/src/spatial/allpass.rs
cp crates/moonlitt-reverb/src/comb.rs crates/moonlitt-effects/src/spatial/comb.rs
cp crates/moonlitt-reverb/src/mod_allpass.rs crates/moonlitt-effects/src/spatial/mod_allpass.rs
cp crates/moonlitt-convolver/src/convolver.rs crates/moonlitt-effects/src/spatial/convolver.rs
cp crates/moonlitt-convolver/src/partition.rs crates/moonlitt-effects/src/spatial/partition.rs
```

- [ ] **Step 4: Fix internal `use` paths in copied files**

Each file currently uses `use crate::` paths relative to their old crate. Update them:

**dynamics/compressor.rs:** Change `use crate::envelope::` → `use super::envelope::`

**eq/parametric.rs:** Change `use crate::biquad::` → `use super::biquad::`

**spatial/reverb.rs:** Change `use crate::allpass::` → `use super::allpass::`, `use crate::comb::` → `use super::comb::`

**spatial/dattorro.rs:** Change `use crate::mod_allpass::` → `use super::mod_allpass::`, `use crate::allpass::` → `use super::allpass::`

**spatial/convolver.rs:** Change `use crate::partition::` → `use super::partition::`

- [ ] **Step 5: Create module files**

**`crates/moonlitt-effects/src/dynamics/mod.rs`:**
```rust
pub mod compressor;
pub(crate) mod envelope;
```

**`crates/moonlitt-effects/src/eq/mod.rs`:**
```rust
pub mod biquad;
pub mod parametric;
```

**`crates/moonlitt-effects/src/spatial/mod.rs`:**
```rust
#[cfg(feature = "reverb")]
pub mod allpass;
#[cfg(feature = "reverb")]
pub mod comb;
#[cfg(feature = "reverb")]
pub mod mod_allpass;
#[cfg(feature = "reverb")]
pub mod reverb;
#[cfg(feature = "reverb")]
pub mod dattorro;
#[cfg(feature = "convolver")]
pub mod convolver;
#[cfg(feature = "convolver")]
pub mod partition;
```

- [ ] **Step 6: Create `crates/moonlitt-effects/src/lib.rs`**

```rust
//! Built-in audio effects for moonlitt.
//!
//! Effects are organized by category and controlled via feature flags.
//! Enable individual effects or entire categories:
//!
//! ```toml
//! moonlitt-effects = { default-features = false, features = ["parametric-eq", "compressor"] }
//! ```

#[cfg(feature = "compressor")]
pub mod dynamics;

#[cfg(feature = "parametric-eq")]
pub mod eq;

#[cfg(any(feature = "reverb", feature = "convolver"))]
pub mod spatial;

// Convenience re-exports
#[cfg(feature = "compressor")]
pub use dynamics::compressor::Compressor;

#[cfg(feature = "parametric-eq")]
pub use eq::parametric::ParametricEq;

#[cfg(feature = "parametric-eq")]
pub use eq::biquad::{Biquad, BiquadCoeffs, FilterType};

#[cfg(feature = "reverb")]
pub use spatial::reverb::Reverb;

#[cfg(feature = "reverb")]
pub use spatial::dattorro::DattorroReverb;

#[cfg(feature = "convolver")]
pub use spatial::convolver::Convolver;
```

- [ ] **Step 7: Verify moonlitt-effects builds**

```bash
cargo build -p moonlitt-effects 2>&1
```

Fix any compilation errors from `use` path mismatches. This is the most likely place for issues — each moved file may have slightly different internal references.

- [ ] **Step 8: Run moonlitt-effects tests**

```bash
cargo test -p moonlitt-effects 2>&1
```

Expected: All effect tests pass (13 EQ + 9 compressor + 18 reverb + 7 convolver = 47+ tests). Every test from every original crate must pass with identical assertions.

- [ ] **Step 9: Update workspace Cargo.toml**

In the root `Cargo.toml`, replace the 4 effect crate members with `moonlitt-effects`:

Remove from members: `"crates/moonlitt-eq"`, `"crates/moonlitt-compressor"`, `"crates/moonlitt-reverb"`, `"crates/moonlitt-convolver"`

Add to members: `"crates/moonlitt-effects"`

- [ ] **Step 10: Update moonlitt-ffi Cargo.toml**

Replace 4 separate effect dependencies with moonlitt-effects:

Remove:
```toml
moonlitt-eq = { path = "../moonlitt-eq" }
moonlitt-compressor = { path = "../moonlitt-compressor" }
moonlitt-reverb = { path = "../moonlitt-reverb" }
moonlitt-convolver = { path = "../moonlitt-convolver" }
```

Add:
```toml
moonlitt-effects = { path = "../moonlitt-effects" }
```

- [ ] **Step 11: Update moonlitt-ffi source imports**

In `crates/moonlitt-ffi/src/builtin_api.rs`, update all `use` statements:

Replace:
```rust
use moonlitt_eq::ParametricEq;
use moonlitt_compressor::Compressor;
use moonlitt_reverb::{Reverb, DattorroReverb};
use moonlitt_convolver::Convolver;
```

With:
```rust
use moonlitt_effects::{ParametricEq, Compressor, Reverb, DattorroReverb, Convolver};
```

- [ ] **Step 12: Update moonlitt-test-suite Cargo.toml**

Replace 4 separate effect dependencies with moonlitt-effects:

Remove:
```toml
moonlitt-reverb = { path = "../moonlitt-reverb" }
moonlitt-compressor = { path = "../moonlitt-compressor" }
moonlitt-eq = { path = "../moonlitt-eq" }
```

Add:
```toml
moonlitt-effects = { path = "../moonlitt-effects" }
```

- [ ] **Step 13: Update moonlitt-test-suite source imports**

Search all test files in `crates/moonlitt-test-suite/tests/` for `use moonlitt_eq::`, `use moonlitt_compressor::`, `use moonlitt_reverb::`, `use moonlitt_convolver::` and replace with `use moonlitt_effects::` equivalents. Key files:

- `aes17_compliance.rs` — uses EQ, Compressor, Reverb
- `dynamics_compliance.rs` — uses Compressor
- `eq_cookbook_compliance.rs` — uses EQ, Biquad
- `reverb_compliance.rs` — uses Reverb internals (allpass, comb)
- `quality_verification.rs` — uses Reverb, EQ, Compressor

For test files that reference internal modules (like `reverb_compliance.rs` using `moonlitt_reverb::allpass`), update to `moonlitt_effects::spatial::allpass`.

- [ ] **Step 14: Delete old effect crate directories**

```bash
rm -rf crates/moonlitt-eq crates/moonlitt-compressor crates/moonlitt-reverb crates/moonlitt-convolver
```

- [ ] **Step 15: Run full workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

Expected: all 356+ tests pass. Zero regressions.

- [ ] **Step 16: Run clippy**

```bash
cargo clippy --workspace -- -D warnings -A clippy::not_unsafe_ptr_arg_deref 2>&1
```

Expected: no warnings.

- [ ] **Step 17: Commit**

```bash
git add -A
git commit -m "refactor(effects): merge 4 effect crates into moonlitt-effects

Consolidate moonlitt-eq, moonlitt-compressor, moonlitt-reverb,
moonlitt-convolver into a single crate with feature flags.

Category modules: dynamics/, eq/, spatial/
Feature flags: individual (compressor, parametric-eq, reverb, convolver)
               and category (dynamics, eq, spatial)

All 47+ effect tests pass unchanged."
```

---

## Phase 2 — Core Split

### Task 3: Extract moonlitt-mixer from runtime

Extract `mixer.rs` (1436 lines) and `dither.rs` from moonlitt-runtime into an independent moonlitt-mixer crate. The mixer comment at line 9 already says: "can be extracted with zero API changes."

**Strategy:** Create moonlitt-mixer crate, copy mixer.rs + dither.rs into it, make runtime re-export from the new crate. This avoids breaking all downstream consumers at once — they can migrate incrementally.

**Files:**
- Create: `crates/moonlitt-mixer/Cargo.toml`
- Create: `crates/moonlitt-mixer/src/lib.rs`
- Move: `crates/moonlitt-runtime/src/mixer.rs` → `crates/moonlitt-mixer/src/mixer.rs`
- Move: `crates/moonlitt-runtime/src/dither.rs` → `crates/moonlitt-mixer/src/dither.rs`
- Modify: `crates/moonlitt-runtime/Cargo.toml` — add moonlitt-mixer dependency
- Modify: `crates/moonlitt-runtime/src/lib.rs` — re-export from moonlitt-mixer
- Modify: `Cargo.toml` — add moonlitt-mixer to workspace members

- [ ] **Step 1: Create moonlitt-mixer crate structure**

```bash
mkdir -p crates/moonlitt-mixer/src
```

- [ ] **Step 2: Create `crates/moonlitt-mixer/Cargo.toml`**

```toml
[package]
name = "moonlitt-mixer"
description = "Audio mixing graph — tracks, send buses, groups, PDC, metering"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
moonlitt-engine = { path = "../moonlitt-engine", features = ["sf2"] }
```

Note: moonlitt-mixer currently depends on `moonlitt-engine` because `Mixer` stores `Engine` objects in tracks. This dependency will be removed in Task 4 when Engine struct is eliminated and Mixer holds `Box<dyn AudioBackend>` directly.

- [ ] **Step 3: Move mixer.rs and dither.rs**

```bash
cp crates/moonlitt-runtime/src/mixer.rs crates/moonlitt-mixer/src/mixer.rs
cp crates/moonlitt-runtime/src/dither.rs crates/moonlitt-mixer/src/dither.rs
```

- [ ] **Step 4: Update internal imports in copied mixer.rs**

Replace `use crate::dither::` with `use crate::dither::` (same, since dither.rs is in the same new crate).

Replace `use crate::event::AudioEvent` with `use moonlitt_core::AudioEvent`. **Wait** — AudioEvent is not yet exported from moonlitt-core in the runtime's usage; runtime still uses its own event.rs. For now, keep the mixer using `moonlitt_engine::Engine` as-is and import AudioEvent from where it's needed. The full migration to core's AudioEvent happens in Task 5 when session is extracted.

Actually, mixer.rs dispatches AudioEvent variants from runtime. We need to check the exact imports and handle this carefully. The mixer needs to import AudioEvent — at this stage, keep a local copy or re-export. The cleanest approach: **make mixer.rs import AudioEvent from moonlitt-core** (which we added in Task 1).

In `crates/moonlitt-mixer/src/mixer.rs`, change:
```rust
use crate::event::AudioEvent;
```
to:
```rust
use moonlitt_core::AudioEvent;
```

This works because the AudioEvent in core has the exact same variants as the one in runtime (we designed it that way in Task 1).

- [ ] **Step 5: Create `crates/moonlitt-mixer/src/lib.rs`**

```rust
//! Audio mixing graph — tracks, send buses, groups, PDC, metering, dither.
//!
//! Platform-agnostic. No audio I/O or threading — just pure DSP computation.

pub mod dither;
pub mod mixer;

pub use dither::{Dither, StereoDither};
pub use mixer::{
    InsertEffect, LevelMeter, Mixer, OutputTarget, SendBus, Track, MasterBus,
};
```

Adjust the pub use list to match the actual public types in mixer.rs.

- [ ] **Step 6: Add moonlitt-mixer to workspace**

In root `Cargo.toml`, add `"crates/moonlitt-mixer"` to workspace members.

- [ ] **Step 7: Add moonlitt-mixer as dependency of moonlitt-runtime**

In `crates/moonlitt-runtime/Cargo.toml`, add:
```toml
moonlitt-mixer = { path = "../moonlitt-mixer" }
```

- [ ] **Step 8: Update moonlitt-runtime to re-export from moonlitt-mixer**

In `crates/moonlitt-runtime/src/lib.rs`, replace:
```rust
pub mod dither;
pub mod mixer;
```
with:
```rust
pub use moonlitt_mixer::dither;
pub use moonlitt_mixer::mixer;
```

Remove the old `crates/moonlitt-runtime/src/mixer.rs` and `crates/moonlitt-runtime/src/dither.rs`.

- [ ] **Step 9: Run full workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

Expected: all tests pass. The re-export means all existing `use moonlitt_runtime::mixer::` paths still work.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "refactor(mixer): extract moonlitt-mixer from runtime

Move mixer.rs (1436 lines) and dither.rs into independent crate.
Runtime re-exports for backward compatibility.
Mixer now imports AudioEvent from moonlitt-core."
```

---

### Task 4: Slim moonlitt-engine

Remove the `Engine` struct wrapper. Keep only the factory function `create()`, plugin scanning, and error types. All callers will use `Box<dyn AudioBackend>` directly.

**Files:**
- Modify: `crates/moonlitt-engine/src/lib.rs`
- Modify: `crates/moonlitt-engine/src/engine.rs` — replace Engine struct with `create()` function
- Modify: `crates/moonlitt-engine/src/backends/mod.rs` — export backend types directly
- Create: `crates/moonlitt-engine/tests/factory_tests.rs`
- Modify: `crates/moonlitt-mixer/src/mixer.rs` — Track holds `Box<dyn AudioBackend>` instead of `Engine`
- Modify: `crates/moonlitt-mixer/Cargo.toml` — remove moonlitt-engine dependency
- Modify: `crates/moonlitt-runtime/src/runtime.rs` — use `create()` instead of `Engine::new()`
- Modify: `crates/moonlitt-runtime/src/audio_thread.rs` — adapt to new mixer API
- Modify: `crates/moonlitt-runtime/src/session.rs` — adapt to Box<dyn AudioBackend>
- Modify: `crates/moonlitt-ffi/src/engine_api.rs` — adapt to factory API
- Modify: `crates/moonlitt-cli/src/main.rs` — use `create()` instead of `Engine`

**This is the largest single task.** The Engine struct is used everywhere. The key insight: every call to `engine.note_on()` becomes `backend.note_on()`. It's mechanical.

- [ ] **Step 1: Write failing test for factory function**

Create `crates/moonlitt-engine/tests/factory_tests.rs`:

```rust
use moonlitt_core::AudioBackend;

#[test]
fn create_returns_backend_for_sf2_extension() {
    // This test needs a real .sf2 file — skip if unavailable
    let sf2_path = std::env::var("MOONLITT_TEST_SF2")
        .unwrap_or_else(|_| "test_data/test.sf2".to_string());
    if !std::path::Path::new(&sf2_path).exists() {
        eprintln!("Skipping: no SF2 file at {sf2_path}");
        return;
    }
    let backend = moonlitt_engine::create(&sf2_path, 44100, 512);
    assert!(backend.is_ok(), "create() should succeed for .sf2");
    let backend = backend.unwrap();
    assert_eq!(backend.sample_rate(), 44100);
}

#[test]
fn create_returns_error_for_unknown_extension() {
    let result = moonlitt_engine::create("unknown.xyz", 44100, 512);
    assert!(result.is_err());
}

#[test]
fn supported_formats_includes_sf2() {
    let formats = moonlitt_engine::supported_formats();
    assert!(formats.contains(&"sf2"), "sf2 must be in supported formats");
}
```

- [ ] **Step 2: Run test — verify it fails**

```bash
cargo test -p moonlitt-engine --test factory_tests 2>&1
```

Expected: compilation error — `moonlitt_engine::create` not found.

- [ ] **Step 3: Rewrite engine.rs as factory module**

Replace the entire `Engine` struct with factory functions. Keep `scan_plugins()` and backend-specific creation logic. The `from_backend()` and `from_shared_sf2()` constructors become standalone functions too.

Key changes in `crates/moonlitt-engine/src/engine.rs`:
- Delete `pub struct Engine` and all `impl Engine` methods
- Keep backend creation logic as `pub fn create(path, sample_rate, buffer_size) -> Result<Box<dyn AudioBackend>, EngineError>`
- Keep `pub fn create_high_quality(path, sample_rate, buffer_size) -> Result<Box<dyn AudioBackend>, EngineError>` (for offline rendering with Sinc72)
- Keep `pub fn scan_plugins(dirs) -> Vec<PluginInfo>`
- Add `pub fn supported_formats() -> Vec<&'static str>`
- Add `pub fn from_backend(backend: Box<dyn AudioBackend>) -> Box<dyn AudioBackend>` (identity, for API symmetry)

- [ ] **Step 4: Update lib.rs exports**

```rust
pub mod backends;
pub mod engine;
pub mod error;
pub mod plugin_info;

pub use engine::{create, create_high_quality, scan_plugins, supported_formats};
pub use error::EngineError;
pub use moonlitt_core::{AudioBackend, BackendInfo, BackendType};
pub use plugin_info::{PluginFormat, PluginInfo};
```

- [ ] **Step 5: Run factory tests — verify they pass**

```bash
cargo test -p moonlitt-engine --test factory_tests 2>&1
```

- [ ] **Step 6: Update moonlitt-mixer — Track holds Box<dyn AudioBackend>**

In `crates/moonlitt-mixer/src/mixer.rs`:

1. Replace every `engine: Engine` field with `backend: Box<dyn AudioBackend>`
2. Replace every `engine.note_on(...)` call with `backend.note_on(...)`
3. Replace every `engine.render(...)` call with `backend.render(...)`
4. Replace every `engine.process_effect(...)` call with `backend.process_effect(...)`
5. Add `source_path: Option<String>` field to Track
6. Update `add_track()` signature: `fn add_track(&mut self, backend: Box<dyn AudioBackend>, source_path: Option<String>, channel_mask: u16) -> u32`

In `crates/moonlitt-mixer/Cargo.toml`:

Remove `moonlitt-engine` dependency — mixer now only depends on `moonlitt-core`.

- [ ] **Step 7: Update moonlitt-runtime to use factory API**

In `crates/moonlitt-runtime/src/runtime.rs`:
- `Runtime::new(engine)` → `Runtime::new(backend: Box<dyn AudioBackend>)`
- All internal `Engine` types → `Box<dyn AudioBackend>`
- Call `moonlitt_engine::create()` where needed, or accept backends directly

In `crates/moonlitt-runtime/Cargo.toml`:
- Keep `moonlitt-engine` dependency (runtime still needs the factory for convenience methods)

- [ ] **Step 8: Update moonlitt-runtime session.rs**

Session persistence uses `Engine::load()` to reconstruct backends from saved paths. Update to use `moonlitt_engine::create()`:

In `restore()` method:
```rust
// Before:
let mut engine = Engine::new(sample_rate, buffer_size);
engine.load(&source.path)?;
// After:
let mut backend = moonlitt_engine::create(&source.path, sample_rate, buffer_size)?;
```

- [ ] **Step 9: Update moonlitt-ffi engine_api.rs**

The FFI layer's `EngineHandle` currently wraps `Engine`. Change it to wrap `Box<dyn AudioBackend>`:

```rust
// Before:
pub struct EngineHandle(Engine);
// After:
pub struct EngineHandle(Box<dyn AudioBackend>);
```

Update all FFI functions to call `backend.method()` instead of `engine.method()`.

`moonlitt_engine_create()` calls `moonlitt_engine::create()` and wraps the result.

- [ ] **Step 10: Update moonlitt-cli main.rs**

Replace all `Engine::new()` + `engine.load()` patterns with `moonlitt_engine::create()`:

```rust
// Before:
let mut engine = Engine::new(sample_rate, buffer_size);
engine.load(path)?;
// After:
let mut backend = moonlitt_engine::create(path, sample_rate, buffer_size)?;
```

- [ ] **Step 11: Run full workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

Expected: all tests pass. This is the riskiest step — mechanical but wide-reaching.

- [ ] **Step 12: Run clippy**

```bash
cargo clippy --workspace -- -D warnings -A clippy::not_unsafe_ptr_arg_deref 2>&1
```

- [ ] **Step 13: Commit**

```bash
git add -A
git commit -m "refactor(engine): remove Engine struct, keep factory function

Engine was a zero-value proxy wrapping Box<dyn AudioBackend>.
Now moonlitt_engine::create() returns Box<dyn AudioBackend> directly.

Mixer tracks hold Box<dyn AudioBackend> + source_path metadata.
moonlitt-mixer no longer depends on moonlitt-engine."
```

---

### Task 5: Extract moonlitt-session from runtime

Extract transport, sequencer, event bus, session persistence, and audio processing loop from moonlitt-runtime into moonlitt-session. This is the platform-agnostic orchestration core.

**Files:**
- Create: `crates/moonlitt-session/Cargo.toml`
- Create: `crates/moonlitt-session/src/lib.rs`
- Create: `crates/moonlitt-session/src/session.rs`
- Create: `crates/moonlitt-session/src/processor.rs`
- Create: `crates/moonlitt-session/src/event_bus.rs`
- Move: `crates/moonlitt-runtime/src/transport.rs` → `crates/moonlitt-session/src/transport.rs`
- Move: `crates/moonlitt-runtime/src/sequencer.rs` → `crates/moonlitt-session/src/sequencer.rs`
- Move: `crates/moonlitt-runtime/src/session.rs` → `crates/moonlitt-session/src/persistence.rs`
- Modify: `Cargo.toml` — add moonlitt-session to workspace
- Modify: `crates/moonlitt-runtime/Cargo.toml` — depend on moonlitt-session

- [ ] **Step 1: Create moonlitt-session crate**

```bash
mkdir -p crates/moonlitt-session/src
```

- [ ] **Step 2: Create `crates/moonlitt-session/Cargo.toml`**

```toml
[package]
name = "moonlitt-session"
description = "Platform-agnostic audio session — transport, sequencer, event bus, persistence"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
moonlitt-mixer = { path = "../moonlitt-mixer" }
rtrb = "0.3"
midly = "0.5"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"

[dev-dependencies]
rtrb = "0.3"
```

- [ ] **Step 3: Move transport.rs and sequencer.rs**

```bash
cp crates/moonlitt-runtime/src/transport.rs crates/moonlitt-session/src/transport.rs
cp crates/moonlitt-runtime/src/sequencer.rs crates/moonlitt-session/src/sequencer.rs
```

Update imports in both files:
- Replace `use crate::event::AudioEvent` with `use moonlitt_core::AudioEvent`
- Replace `use crate::transport::` with `use crate::transport::` (stays same in new crate)

- [ ] **Step 4: Move session.rs → persistence.rs**

```bash
cp crates/moonlitt-runtime/src/session.rs crates/moonlitt-session/src/persistence.rs
```

Update imports: replace `use crate::mixer::` with `use moonlitt_mixer::mixer::`

- [ ] **Step 5: Create event_bus.rs**

Extract the SPSC ring buffer setup from runtime.rs into a standalone module:

```rust
use moonlitt_core::TimedEvent;
use rtrb::RingBuffer;

/// Event bus capacity — must be power of 2 for rtrb.
pub const EVENT_BUS_CAPACITY: usize = 1024;

/// Create a producer/consumer pair for the event bus.
pub fn create_event_bus() -> (rtrb::Producer<TimedEvent>, rtrb::Consumer<TimedEvent>) {
    RingBuffer::new(EVENT_BUS_CAPACITY)
}
```

- [ ] **Step 6: Create processor.rs (AudioProcessor)**

Port the audio processing loop from `audio_thread.rs`, adapted to own a Mixer directly:

```rust
use moonlitt_core::{AudioEvent, TimedEvent};
use moonlitt_mixer::mixer::Mixer;
use crate::sequencer::Sequencer;
use crate::transport::Transport;
use std::sync::Arc;

/// Process plane — lives on the audio thread.
pub struct AudioProcessor {
    pub(crate) mixer: Mixer,
    pub(crate) sequencer: Option<Sequencer>,
    pub(crate) transport: Arc<Transport>,
    pub(crate) consumer: rtrb::Consumer<TimedEvent>,
    // structural command receiver for add/remove track
    pub(crate) command_rx: std::sync::mpsc::Receiver<MixerCommand>,
}

pub enum MixerCommand {
    AddTrack {
        backend: Box<dyn moonlitt_core::AudioBackend>,
        source_path: Option<String>,
        channel_mask: u16,
        result_tx: std::sync::mpsc::Sender<u32>,
    },
    RemoveTrack { track_id: u32 },
    AddInsert {
        track_id: u32,
        effect: Box<dyn moonlitt_core::AudioBackend>,
        result_tx: std::sync::mpsc::Sender<Option<u32>>,
    },
    RemoveInsert { track_id: u32, insert_id: u32 },
    AddSendBus {
        backend: Box<dyn moonlitt_core::AudioBackend>,
        result_tx: std::sync::mpsc::Sender<u32>,
    },
}

impl AudioProcessor {
    /// Called by the platform's audio callback.
    /// `output` is interleaved stereo: [L0, R0, L1, R1, ...]
    pub fn process(&mut self, output: &mut [f32]) {
        // 1. Drain structural commands
        self.drain_commands();

        // 2. Drain SPSC event queue
        while let Ok(timed) = self.consumer.pop() {
            self.mixer.dispatch_event(timed.event);
        }

        // 3. Advance sequencer
        if let Some(ref mut seq) = self.sequencer {
            if self.transport.is_playing() {
                let frames = output.len() / 2;
                let sr = self.mixer.sample_rate();
                let mut events = Vec::new();
                seq.advance(frames, sr, &mut events, self.transport.tempo(), self.transport.looping());
                for te in events {
                    self.mixer.dispatch_event(te.event);
                }
            }
        }

        // 4. Render mixer
        let frames = output.len() / 2;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        self.mixer.render(&mut left, &mut right);

        // 5. Interleave to output
        for i in 0..frames {
            output[i * 2] = left[i];
            output[i * 2 + 1] = right[i];
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                MixerCommand::AddTrack { backend, source_path, channel_mask, result_tx } => {
                    let id = self.mixer.add_track(backend, source_path, channel_mask);
                    let _ = result_tx.send(id);
                }
                MixerCommand::RemoveTrack { track_id } => {
                    self.mixer.remove_track(track_id);
                }
                MixerCommand::AddInsert { track_id, effect, result_tx } => {
                    let id = self.mixer.add_insert(track_id, effect);
                    let _ = result_tx.send(id);
                }
                MixerCommand::RemoveInsert { track_id, insert_id } => {
                    self.mixer.remove_insert(track_id, insert_id);
                }
                MixerCommand::AddSendBus { backend, result_tx } => {
                    let id = self.mixer.add_send_bus(backend);
                    let _ = result_tx.send(id);
                }
            }
        }
    }
}
```

Note: This is a simplified version. The actual implementation must preserve the sample-accurate delayed event handling from audio_thread.rs (pending events, render splits at event boundaries). Copy that logic faithfully.

- [ ] **Step 7: Create session.rs (Session control plane)**

```rust
use moonlitt_core::{AudioBackend, AudioEvent, TimedEvent};
use moonlitt_mixer::mixer::Mixer;
use crate::event_bus;
use crate::processor::{AudioProcessor, MixerCommand};
use crate::sequencer::Sequencer;
use crate::transport::Transport;
use std::sync::Arc;

/// Control plane — lives on the main thread.
pub struct Session {
    producer: rtrb::Producer<TimedEvent>,
    transport: Arc<Transport>,
    command_tx: std::sync::mpsc::Sender<MixerCommand>,
    sample_rate: u32,
    buffer_size: u32,
}

impl Session {
    /// Create a Session + AudioProcessor pair.
    pub fn new(sample_rate: u32, buffer_size: u32) -> (Session, AudioProcessor) {
        let (producer, consumer) = event_bus::create_event_bus();
        let transport = Arc::new(Transport::new());
        let mixer = Mixer::new(sample_rate, buffer_size as usize);
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        let session = Session {
            producer,
            transport: transport.clone(),
            command_tx,
            sample_rate,
            buffer_size,
        };

        let processor = AudioProcessor {
            mixer,
            sequencer: None,
            transport,
            consumer,
            command_rx,
        };

        (session, processor)
    }

    // --- Transport ---
    pub fn play(&self) { self.transport.play(); }
    pub fn pause(&self) { self.transport.pause(); }
    pub fn stop(&self) { self.transport.stop(); }
    pub fn is_playing(&self) -> bool { self.transport.is_playing() }
    pub fn set_tempo(&self, bpm: f64) { self.transport.set_tempo(bpm); }
    pub fn clear_tempo(&self) { self.transport.clear_tempo(); }
    pub fn set_loop(&self, enabled: bool) { self.transport.set_loop(enabled); }

    // --- MIDI (via event bus) ---
    pub fn note_on(&self, channel: u8, note: u8, velocity: u8) {
        let _ = self.producer.push(TimedEvent {
            event: AudioEvent::NoteOn { channel, note, velocity },
            delay_samples: 0,
        });
    }

    pub fn note_off(&self, channel: u8, note: u8) {
        let _ = self.producer.push(TimedEvent {
            event: AudioEvent::NoteOff { channel, note, velocity: 0 },
            delay_samples: 0,
        });
    }

    // ... remaining event methods follow the same pattern

    // --- Structural commands (via mpsc) ---
    pub fn add_track(&self, backend: Box<dyn AudioBackend>, source_path: Option<String>, channel_mask: u16) -> u32 {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = self.command_tx.send(MixerCommand::AddTrack {
            backend, source_path, channel_mask, result_tx: tx,
        });
        rx.recv().unwrap_or(0)
    }

    pub fn add_insert(&self, track_id: u32, effect: Box<dyn AudioBackend>) -> Option<u32> {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = self.command_tx.send(MixerCommand::AddInsert {
            track_id, effect, result_tx: tx,
        });
        rx.recv().ok().flatten()
    }

    pub fn add_send_bus(&self, backend: Box<dyn AudioBackend>) -> u32 {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = self.command_tx.send(MixerCommand::AddSendBus {
            backend, result_tx: tx,
        });
        rx.recv().unwrap_or(0)
    }

    pub fn remove_track(&self, track_id: u32) {
        let _ = self.command_tx.send(MixerCommand::RemoveTrack { track_id });
    }

    // --- Mixer control (via event bus) ---
    pub fn set_track_volume(&self, track_id: u8, volume: f32) {
        let _ = self.producer.push(TimedEvent {
            event: AudioEvent::MixerTrackVolume { track_id, volume },
            delay_samples: 0,
        });
    }

    pub fn set_track_pan(&self, track_id: u8, pan: f32) {
        let _ = self.producer.push(TimedEvent {
            event: AudioEvent::MixerTrackPan { track_id, pan },
            delay_samples: 0,
        });
    }

    pub fn set_master_volume(&self, volume: f32) {
        let _ = self.producer.push(TimedEvent {
            event: AudioEvent::MixerMasterVolume(volume),
            delay_samples: 0,
        });
    }
}
```

- [ ] **Step 8: Create lib.rs**

```rust
pub mod event_bus;
pub mod persistence;
pub mod processor;
pub mod sequencer;
pub mod session;
pub mod transport;

pub use processor::{AudioProcessor, MixerCommand};
pub use session::Session;
pub use transport::Transport;
```

- [ ] **Step 9: Add moonlitt-session to workspace and as runtime dependency**

Root Cargo.toml: add `"crates/moonlitt-session"` to members.

`crates/moonlitt-runtime/Cargo.toml`: add `moonlitt-session = { path = "../moonlitt-session" }`

- [ ] **Step 10: Update runtime to re-export from session**

In `crates/moonlitt-runtime/src/lib.rs`, replace direct module declarations for transport, sequencer, session with re-exports from moonlitt-session. Delete the moved source files from runtime.

- [ ] **Step 11: Run workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

All tests must pass.

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "refactor(session): extract moonlitt-session from runtime

Platform-agnostic orchestration core:
- Session (control plane) + AudioProcessor (process plane)
- Transport, Sequencer, EventBus, Persistence
- No cpal, no midir — pure scheduling and rendering logic"
```

---

## Phase 3 — Platform Bindings

### Task 6: Extract moonlitt-audio-io from runtime

Extract cpal and midir wrappers into moonlitt-audio-io. After this, moonlitt-runtime is deleted.

**Files:**
- Create: `crates/moonlitt-audio-io/Cargo.toml`
- Create: `crates/moonlitt-audio-io/src/lib.rs`
- Move: `crates/moonlitt-runtime/src/audio_output.rs` → `crates/moonlitt-audio-io/src/audio_output.rs`
- Move: `crates/moonlitt-runtime/src/midi_input.rs` → `crates/moonlitt-audio-io/src/midi_input.rs`
- Delete: `crates/moonlitt-runtime/`
- Modify: `Cargo.toml` — replace moonlitt-runtime with moonlitt-audio-io

- [ ] **Step 1: Create moonlitt-audio-io crate**

```bash
mkdir -p crates/moonlitt-audio-io/src
```

- [ ] **Step 2: Create Cargo.toml**

```toml
[package]
name = "moonlitt-audio-io"
description = "Native audio I/O — cpal output + midir MIDI input"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
moonlitt-session = { path = "../moonlitt-session" }
cpal = "0.15"
midir = "0.10"
```

- [ ] **Step 3: Move audio_output.rs and midi_input.rs**

```bash
cp crates/moonlitt-runtime/src/audio_output.rs crates/moonlitt-audio-io/src/audio_output.rs
cp crates/moonlitt-runtime/src/midi_input.rs crates/moonlitt-audio-io/src/midi_input.rs
```

Update imports to use moonlitt-core and moonlitt-session types.

- [ ] **Step 4: Implement AudioHost for CpalHost**

Wrap the existing `AudioOutput` as an `AudioHost` implementation:

```rust
use moonlitt_core::{AudioCallback, AudioHost};

pub struct CpalHost {
    output: Option<AudioOutput>,
    sample_rate: u32,
    buffer_size: u32,
}

impl CpalHost {
    pub fn new(desired_rate: u32, buffer_size: u32) -> Result<Self, Box<dyn std::error::Error>> {
        // Pre-check audio device availability (ported from AudioOutput::pre_check)
        let host = cpal::default_host();
        let device = host.default_output_device()
            .ok_or("No audio output device")?;
        Ok(CpalHost { output: None, sample_rate: desired_rate, buffer_size })
    }
}

impl AudioHost for CpalHost {
    fn start(&mut self, mut callback: AudioCallback) -> Result<(), Box<dyn std::error::Error>> {
        // Port the stream-building logic from audio_output.rs:
        // 1. Get default output device
        // 2. Negotiate config (stereo F32 at desired sample rate)
        // 3. Build output stream with callback
        // 4. stream.play()
        // The callback closure calls `callback(output_data)` on each audio cycle.
        let host = cpal::default_host();
        let device = host.default_output_device()
            .ok_or("No audio output device")?;
        use cpal::traits::*;
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(self.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(self.buffer_size),
        };
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                callback(data);
            },
            |err| eprintln!("Audio stream error: {err}"),
            None,
        )?;
        stream.play()?;
        // Store stream to keep it alive (field needs to be added to CpalHost)
        Ok(())
    }
    fn stop(&mut self) { self.output = None; }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn buffer_size(&self) -> u32 { self.buffer_size }
}
```

- [ ] **Step 5: Create lib.rs**

```rust
pub mod audio_output;
pub mod midi_input;

pub use audio_output::CpalHost;
pub use midi_input::{MidiDeviceInfo, MidiInputConnection};
```

- [ ] **Step 6: Update workspace — remove runtime, add audio-io**

Root Cargo.toml: remove `"crates/moonlitt-runtime"`, add `"crates/moonlitt-audio-io"`.

- [ ] **Step 7: Update all crates that depended on moonlitt-runtime**

- `moonlitt-ffi` → depend on `moonlitt-session` + `moonlitt-audio-io`
- `moonlitt-cli` → depend on `moonlitt-session` + `moonlitt-audio-io`
- `moonlitt-test-suite` → depend on `moonlitt-session` (if it used runtime)

- [ ] **Step 8: Delete moonlitt-runtime**

```bash
rm -rf crates/moonlitt-runtime
```

- [ ] **Step 9: Run full workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "refactor(audio-io): extract moonlitt-audio-io, delete runtime

CpalHost implements AudioHost trait for native audio output.
MidiInput connects MIDI devices to Session.
moonlitt-runtime is fully decomposed and removed."
```

---

### Task 7: Restructure moonlitt-capi

Rename moonlitt-ffi to moonlitt-capi and restructure the API around Session instead of Runtime.

**Files:**
- Rename: `crates/moonlitt-ffi/` → `crates/moonlitt-capi/`
- Modify: `crates/moonlitt-capi/Cargo.toml` — rename, update dependencies
- Modify: `crates/moonlitt-capi/src/lib.rs`
- Rewrite: `crates/moonlitt-capi/src/engine_api.rs` — use factory
- Rewrite: `crates/moonlitt-capi/src/runtime_api.rs` → `session_api.rs` — use Session
- Modify: `crates/moonlitt-capi/src/builtin_api.rs` → `effects_api.rs` — use moonlitt-effects
- Modify: `Cargo.toml` — update workspace member

- [ ] **Step 1: Rename directory**

```bash
mv crates/moonlitt-ffi crates/moonlitt-capi
```

- [ ] **Step 2: Update Cargo.toml**

```toml
[package]
name = "moonlitt-capi"
description = "C ABI bindings for moonlitt audio engine"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
moonlitt-engine = { path = "../moonlitt-engine", features = ["sf2", "vst3", "clap"] }
moonlitt-session = { path = "../moonlitt-session" }
moonlitt-effects = { path = "../moonlitt-effects" }
moonlitt-audio-io = { path = "../moonlitt-audio-io", optional = true }
hound = "3"
oxisynth = { path = "../../deps/oxisynth/oxisynth" }

[features]
default = ["audio-io"]
audio-io = ["dep:moonlitt-audio-io"]

[lib]
crate-type = ["cdylib", "rlib"]
```

- [ ] **Step 3: Rewrite session_api.rs**

Replace all Runtime-based APIs with Session-based APIs. The function signatures change to use `SessionHandle` and `ProcessorHandle` instead of `RuntimeHandle`.

Key mapping:
- `moonlitt_runtime_create()` → `moonlitt_session_new()` (returns session + processor)
- `moonlitt_runtime_start()` → `moonlitt_audio_start()` (starts CpalHost with processor)
- `moonlitt_runtime_note_on()` → `moonlitt_session_note_on()`
- All mixer controls keep the same names but operate on SessionHandle

- [ ] **Step 4: Rename files and update lib.rs**

```bash
mv crates/moonlitt-capi/src/runtime_api.rs crates/moonlitt-capi/src/session_api.rs
mv crates/moonlitt-capi/src/builtin_api.rs crates/moonlitt-capi/src/effects_api.rs
```

Update lib.rs:
```rust
mod engine_api;
mod session_api;
mod effects_api;
mod util;
```

- [ ] **Step 5: Update workspace Cargo.toml**

Replace `"crates/moonlitt-ffi"` with `"crates/moonlitt-capi"` in workspace members.

- [ ] **Step 6: Run tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(capi): rename ffi→capi, restructure around Session

API now uses Session/Processor handles instead of Runtime.
moonlitt-audio-io is optional (game engines manage their own audio).
Effects use moonlitt-effects crate."
```

---

### Task 8: Create moonlitt-node (napi-rs binding)

New crate providing Node.js bindings via napi-rs.

**Files:**
- Create: `crates/moonlitt-node/Cargo.toml`
- Create: `crates/moonlitt-node/src/lib.rs`
- Create: `crates/moonlitt-node/src/engine.rs`
- Create: `crates/moonlitt-node/src/session.rs`
- Create: `crates/moonlitt-node/src/effects.rs`
- Create: `crates/moonlitt-node/src/types.rs`
- Create: `crates/moonlitt-node/build.rs`
- Create: `crates/moonlitt-node/package.json`
- Modify: `Cargo.toml` — add to workspace

- [ ] **Step 1: Install napi-rs CLI**

```bash
npm install -g @napi-rs/cli
```

- [ ] **Step 2: Initialize napi-rs project**

```bash
cd crates && napi new moonlitt-node --platforms darwin-arm64 darwin-x64 linux-x64-gnu win32-x64-msvc
```

Or manually create the structure.

- [ ] **Step 3: Create Cargo.toml**

```toml
[package]
name = "moonlitt-node"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
moonlitt-engine = { path = "../moonlitt-engine", features = ["sf2", "vst3", "clap"] }
moonlitt-session = { path = "../moonlitt-session" }
moonlitt-effects = { path = "../moonlitt-effects" }
moonlitt-audio-io = { path = "../moonlitt-audio-io" }
napi = { version = "3", features = ["async"] }
napi-derive = "3"

[build-dependencies]
napi-build = "2"
```

- [ ] **Step 4: Create build.rs**

```rust
extern crate napi_build;

fn main() {
    napi_build::setup();
}
```

- [ ] **Step 5: Create src/engine.rs — factory functions**

```rust
use napi_derive::napi;
use napi::Result;
use moonlitt_core::AudioBackend;

#[napi]
pub struct Backend {
    pub(crate) inner: Option<Box<dyn AudioBackend>>,
}

#[napi]
pub fn create(path: String, sample_rate: u32, buffer_size: u32) -> Result<Backend> {
    let backend = moonlitt_engine::create(&path, sample_rate, buffer_size)
        .map_err(|e| napi::Error::from_reason(format!("{e}")))?;
    Ok(Backend { inner: Some(backend) })
}

#[napi]
pub fn create_high_quality(path: String, sample_rate: u32, buffer_size: u32) -> Result<Backend> {
    let backend = moonlitt_engine::create_high_quality(&path, sample_rate, buffer_size)
        .map_err(|e| napi::Error::from_reason(format!("{e}")))?;
    Ok(Backend { inner: Some(backend) })
}

#[napi]
pub fn scan_plugins(dirs: Vec<String>) -> Vec<PluginInfo> {
    let dirs_ref: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
    moonlitt_engine::scan_plugins(&dirs_ref)
        .into_iter()
        .map(|p| PluginInfo {
            name: p.name,
            path: p.path,
            format: format!("{:?}", p.format),
        })
        .collect()
}

#[napi]
pub fn supported_formats() -> Vec<String> {
    moonlitt_engine::supported_formats()
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

#[napi(object)]
pub struct PluginInfo {
    pub name: String,
    pub path: String,
    pub format: String,
}
```

- [ ] **Step 6: Create src/session.rs — Session class**

```rust
use napi_derive::napi;
use napi::Result;
use crate::engine::Backend;
use crate::types::TrackLevels;

#[napi]
pub struct Session {
    inner: moonlitt_session::Session,
    processor: Option<moonlitt_session::AudioProcessor>,
    host: Option<moonlitt_audio_io::CpalHost>,
}

#[napi]
impl Session {
    #[napi(constructor)]
    pub fn new(sample_rate: u32, buffer_size: u32) -> Self {
        let (session, processor) = moonlitt_session::Session::new(sample_rate, buffer_size);
        Session {
            inner: session,
            processor: Some(processor),
            host: None,
        }
    }

    #[napi]
    pub fn add_track(&mut self, backend: &mut Backend, path: Option<String>) -> u32 {
        let b = backend.inner.take()
            .expect("Backend already consumed");
        self.inner.add_track(b, path, 0xFFFF)
    }

    #[napi]
    pub fn play(&self) { self.inner.play(); }

    #[napi]
    pub fn pause(&self) { self.inner.pause(); }

    #[napi]
    pub fn stop(&self) { self.inner.stop(); }

    #[napi]
    pub fn note_on(&self, channel: u8, note: u8, velocity: u8) {
        self.inner.note_on(channel, note, velocity);
    }

    #[napi]
    pub fn note_off(&self, channel: u8, note: u8) {
        self.inner.note_off(channel, note);
    }

    #[napi]
    pub fn set_track_volume(&self, track_id: u8, db: f64) {
        self.inner.set_track_volume(track_id, db as f32);
    }

    #[napi]
    pub fn set_track_pan(&self, track_id: u8, pan: f64) {
        self.inner.set_track_pan(track_id, pan as f32);
    }

    #[napi]
    pub fn set_master_volume(&self, db: f64) {
        self.inner.set_master_volume(db as f32);
    }

    #[napi]
    pub fn start_audio(&mut self) -> Result<()> {
        let processor = self.processor.take()
            .ok_or_else(|| napi::Error::from_reason("Audio already started"))?;
        let mut host = moonlitt_audio_io::CpalHost::new(
            self.inner.sample_rate(),
            self.inner.buffer_size(),
        ).map_err(|e| napi::Error::from_reason(format!("{e}")))?;
        use moonlitt_core::AudioHost;
        host.start(Box::new(move |output| {
            processor.process(output);
        })).map_err(|e| napi::Error::from_reason(format!("{e}")))?;
        self.host = Some(host);
        Ok(())
    }
}
```

Note: The Session/AudioProcessor pair ownership needs careful design. The constructor creates both, but the processor needs to be moved into CpalHost when start_audio() is called. One approach: store `Option<AudioProcessor>` in the napi Session and take it on start_audio().

- [ ] **Step 7: Create src/types.rs**

```rust
use napi_derive::napi;

#[napi(object)]
pub struct TrackLevels {
    pub peak_l: f64,
    pub peak_r: f64,
}
```

- [ ] **Step 8: Create src/effects.rs — effect factories**

```rust
use napi_derive::napi;
use crate::engine::Backend;

#[napi]
pub fn create_eq(sample_rate: u32) -> Backend {
    let eq = moonlitt_effects::ParametricEq::new(sample_rate);
    Backend { inner: Some(Box::new(eq)) }
}

#[napi]
pub fn create_compressor(sample_rate: u32) -> Backend {
    let comp = moonlitt_effects::Compressor::new(sample_rate);
    Backend { inner: Some(Box::new(comp)) }
}

#[napi]
pub fn create_reverb(sample_rate: u32) -> Backend {
    let rev = moonlitt_effects::Reverb::new(sample_rate);
    Backend { inner: Some(Box::new(rev)) }
}
```

- [ ] **Step 9: Create src/lib.rs**

```rust
mod engine;
mod effects;
mod session;
mod types;
```

- [ ] **Step 10: Create package.json**

```json
{
  "name": "@moonlitt/node",
  "version": "0.1.0",
  "main": "index.js",
  "types": "index.d.ts",
  "napi": {
    "name": "moonlitt",
    "triples": {
      "defaults": false,
      "additional": [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc"
      ]
    }
  },
  "devDependencies": {
    "@napi-rs/cli": "^3.0.0"
  },
  "scripts": {
    "build": "napi build --release",
    "build:debug": "napi build"
  }
}
```

- [ ] **Step 11: Add to workspace and build**

Add `"crates/moonlitt-node"` to workspace members in root Cargo.toml.

```bash
cargo build -p moonlitt-node 2>&1
```

Fix compilation errors. napi-rs generates TypeScript bindings automatically.

- [ ] **Step 12: Run workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 13: Commit**

```bash
git add -A
git commit -m "feat(node): add moonlitt-node — Node.js binding via napi-rs

Exposes Session, Backend, effects factories, and plugin scanning
to Node.js/Electron/Ink. Auto-generates TypeScript types.
Same API semantics as moonlitt-capi."
```

---

## Phase 4 — Application Layer

### Task 9: Update moonlitt-cli

Update CLI to use moonlitt-session + moonlitt-audio-io instead of moonlitt-runtime.

**Files:**
- Modify: `crates/moonlitt-cli/Cargo.toml`
- Modify: `crates/moonlitt-cli/src/main.rs`

- [ ] **Step 1: Update Cargo.toml**

Replace:
```toml
moonlitt-runtime = { path = "../moonlitt-runtime" }
```

With:
```toml
moonlitt-session = { path = "../moonlitt-session" }
moonlitt-audio-io = { path = "../moonlitt-audio-io" }
moonlitt-effects = { path = "../moonlitt-effects" }
```

- [ ] **Step 2: Update main.rs imports and usage**

Replace all `use moonlitt_runtime::` with appropriate imports from session/audio-io.

Key changes:
- `Runtime::new(engine)` → `Session::new(sample_rate, buffer_size)` + `CpalHost`
- `Engine::new() + engine.load()` → `moonlitt_engine::create()`
- `runtime.note_on()` → `session.note_on()`
- `runtime.start()` → `host.start(callback)`

- [ ] **Step 3: Run CLI commands manually**

```bash
cargo run -p moonlitt-cli -- scan
cargo run -p moonlitt-cli -- midi-devices
```

Verify basic commands work.

- [ ] **Step 4: Run workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(cli): update to use session + audio-io

Replace moonlitt-runtime with moonlitt-session + moonlitt-audio-io.
Same functionality, new API."
```

---

### Task 10: Update moonlitt-test-suite and final verification

Update all test imports and run the full compliance suite.

**Files:**
- Modify: `crates/moonlitt-test-suite/Cargo.toml`
- Modify: All test files in `crates/moonlitt-test-suite/tests/`

- [ ] **Step 1: Update test-suite Cargo.toml**

Ensure all dependencies point to the new crate names. Remove moonlitt-runtime, add moonlitt-session/moonlitt-mixer/moonlitt-audio-io as needed.

- [ ] **Step 2: Update test imports**

Search all test files for references to old crate names and update:
- `moonlitt_runtime::mixer::` → `moonlitt_mixer::mixer::`
- `moonlitt_runtime::transport::` → `moonlitt_session::transport::`
- `moonlitt_runtime::` → `moonlitt_session::`
- `moonlitt_eq::` → `moonlitt_effects::` (if not done in Task 2)
- `moonlitt_compressor::` → `moonlitt_effects::`
- `moonlitt_reverb::` → `moonlitt_effects::`
- `moonlitt_convolver::` → `moonlitt_effects::`

- [ ] **Step 3: Run full test suite**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

**This is the final gate.** All 356+ tests must pass. No regressions. No weakened assertions.

- [ ] **Step 4: Run clippy**

```bash
cargo clippy --workspace -- -D warnings -A clippy::not_unsafe_ptr_arg_deref 2>&1
```

Zero warnings.

- [ ] **Step 5: Verify crate count**

```bash
ls -d crates/moonlitt-*/
```

Expected 14 directories:
```
moonlitt-audio-io
moonlitt-capi
moonlitt-cli
moonlitt-clap
moonlitt-core
moonlitt-effects
moonlitt-engine
moonlitt-mixer
moonlitt-node
moonlitt-resampler
moonlitt-sampler
moonlitt-session
moonlitt-test-suite
moonlitt-vst3
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: complete crate restructuring — 14 crates, all tests pass

Final state:
- Layer 0: core (traits, events, AudioHost)
- Layer 1: sampler, vst3, clap, effects, resampler
- Layer 2: engine (factory), mixer (audio graph)
- Layer 3: session (orchestration)
- Layer 4: audio-io, capi, node (platform bindings)
- Apps: cli, test-suite

All 356+ tests pass. Zero regressions."
```

- [ ] **Step 7: Update CLAUDE.md**

Update the architecture section in `CLAUDE.md` to reflect the new crate structure:
- New dependency graph
- New crate descriptions
- Updated build/test commands
- Remove references to moonlitt-runtime, moonlitt-ffi, and individual effect crates

- [ ] **Step 8: Final commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md for new crate structure"
```
