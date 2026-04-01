# Moonlitt Crate Restructuring Design

**Date:** 2026-04-01
**Status:** Draft
**Scope:** Full crate graph redesign — core, effects, engine, mixer, session, platform bindings

## Motivation

Moonlitt is evolving from a native-only audio engine to a multi-platform engine with bindings for Node.js (Web DAW / Electron / Ink terminal), C ABI (Unity / Unreal / Godot), and potentially WASM. The current crate structure has two problems:

1. **`moonlitt-runtime` mixes platform-agnostic orchestration with platform-specific I/O** — mixer, transport, sequencer, session persistence are bundled with cpal and midir. A Node.js binding cannot reuse the orchestration layer without pulling in cpal.
2. **Four independent effect crates** increase maintenance overhead without binary size benefit — Rust feature flags provide identical compile-time tree-shaking within a single crate.

The restructuring follows the Vue `@vue/runtime-core` / `@vue/runtime-dom` layered model: a platform-agnostic orchestration core (`moonlitt-session`) with thin platform-specific bindings on top.

## Design Principles

- **Dependency arrows point inward only** — platform bindings depend on session, session depends on mixer/core. Never reverse.
- **AudioBackend remains the single trait** — VST3/CLAP plugins can be both instruments and effects; splitting the trait adds complexity without type safety. Use `BackendCaps` bitflags for runtime capability queries.
- **Feature flags are the tree-shaking mechanism** — no need for separate crates to control binary size.
- **API consistency across bindings** — capi, node, and audio-io expose the same semantic operations in platform-idiomatic style.
- **Strict TDD** — every phase must pass all existing tests plus new tests for extracted modules. Tests must not be weakened to pass.

## Crate Inventory: Before and After

### Before (13 crates)

```
moonlitt-core, moonlitt-vst3, moonlitt-clap, moonlitt-sampler, moonlitt-resampler,
moonlitt-engine, moonlitt-runtime, moonlitt-ffi, moonlitt-cli,
moonlitt-eq, moonlitt-compressor, moonlitt-reverb, moonlitt-convolver,
moonlitt-test-suite
```

### After (14 crates)

```
moonlitt-core, moonlitt-vst3, moonlitt-clap, moonlitt-sampler, moonlitt-resampler,
moonlitt-engine, moonlitt-effects, moonlitt-mixer, moonlitt-session,
moonlitt-audio-io, moonlitt-capi, moonlitt-node, moonlitt-cli,
moonlitt-test-suite
```

### Change Summary

| Current | Becomes | Change |
|---------|---------|--------|
| `moonlitt-core` | `moonlitt-core` | Expand: add AudioEvent, BackendCaps, AudioHost trait |
| `moonlitt-eq` | `moonlitt-effects` | Merge 4 effect crates into 1 with feature flags |
| `moonlitt-compressor` | (merged) | |
| `moonlitt-reverb` | (merged) | |
| `moonlitt-convolver` | (merged) | |
| `moonlitt-engine` | `moonlitt-engine` | Slim down: remove Engine struct, keep factory function only |
| `moonlitt-runtime` | Split into 3: | |
| - mixer.rs | `moonlitt-mixer` | Extract as independent crate |
| - transport/sequencer/event/session | `moonlitt-session` | Platform-agnostic orchestration core |
| - audio_output/midi_input | `moonlitt-audio-io` | Platform-specific I/O (cpal + midir) |
| `moonlitt-ffi` | `moonlitt-capi` | Rename + restructure API around Session |
| (new) | `moonlitt-node` | Node.js binding via napi-rs |
| `moonlitt-sampler` | unchanged | |
| `moonlitt-resampler` | unchanged | |
| `moonlitt-vst3` | unchanged | |
| `moonlitt-clap` | unchanged | |
| `moonlitt-cli` | update deps | Depend on session + audio-io instead of runtime |
| `moonlitt-test-suite` | update imports | |

## Complete Dependency Graph

```
                        moonlitt-core
                  (AudioBackend, AudioEvent,
                   BackendCaps, AudioHost)
                            ^
          +---------+-------+-------+----------+
          |         |       |       |          |
      sampler    vst3    clap   effects     mixer
      (+ resampler)              (dynamics,   (tracks, sends,
                                  eq,          groups, PDC,
                                  spatial)     metering, dither)
          ^         ^       ^       |          ^
          +-----+---+-------+       |          |
                |                   |          |
            engine                  |       session
        (factory fn)                |    (transport, sequencer,
                |                   |     event bus, persistence)
                |                   |          ^
                |                   |    +-----+------+
                |                   |    |     |      |
                v                   v    v     v      v
             audio-io            capi        node
          (cpal + midir)    (C ABI)     (napi-rs)
                ^
                |
              cli
```

**Rule: arrows point inward only, never reverse.**

---

## Layer 0: moonlitt-core (Expansion)

### New Types

```rust
// BackendCaps — replaces BackendType enum
bitflags! {
    pub struct BackendCaps: u32 {
        const SOURCE = 0b01;   // can render() (synthesizers, samplers)
        const EFFECT = 0b10;   // can process_effect() (EQ, reverb)
        const BOTH   = 0b11;   // VST3/CLAP plugins may be either
    }
}
```

### AudioEvent (moved down from runtime/event.rs)

All crates need the event type: session (event queue, sequencer), mixer (dispatch), capi/node (construct events). Keeping it in a higher crate would force reverse dependencies.

```rust
#[derive(Clone, Copy, Debug)]
pub enum AudioEvent {
    // MIDI
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    CC { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: i16 },
    ProgramChange { channel: u8, program: u8 },
    AllNotesOff,
    // Mixer control
    SetTrackVolume { track: u16, db: f32 },
    SetTrackPan { track: u16, pan: f32 },
    SetTrackMute { track: u16, muted: bool },
    SetTrackSolo { track: u16, solo: bool },
    SetMasterVolume { db: f32 },
    // Effect control
    SetInsertParam { track: u16, insert: u16, param: u32, value: f64 },
    InsertBypass { track: u16, insert: u16, bypass: bool },
    // Transport
    Stop,
}
```

**Size constraint:** `AudioEvent` must remain <= 16 bytes for cache-friendly ring buffer operation. Assert at compile time with `const_assert!(size_of::<AudioEvent>() <= 16)`. Note: `SetInsertParam` contains `u16 + u16 + u32 + f64` (12 bytes payload + discriminant) which may exceed 16 bytes. If so, downgrade `value` to `f32` or split into a separate structural command channel. Resolve during Phase 1 implementation.

### AudioHost Trait

Defined in core, implemented by platform bindings.

```rust
pub trait AudioHost: Send {
    fn start(&mut self, callback: AudioCallback) -> Result<(), Box<dyn Error>>;
    fn stop(&mut self);
    fn sample_rate(&self) -> u32;
    fn buffer_size(&self) -> u32;
}

pub type AudioCallback = Box<dyn FnMut(&mut [f32]) + Send>;
```

### Dependencies

```toml
[dependencies]
bitflags = "2"
```

No new external dependencies.

---

## Layer 1: moonlitt-effects (Merge)

### Directory Structure

```
crates/moonlitt-effects/
├── Cargo.toml
└── src/
    ├── lib.rs                  # feature-gated re-exports
    ├── dynamics/
    │   ├── mod.rs
    │   └── compressor.rs       # from moonlitt-compressor
    ├── eq/
    │   ├── mod.rs
    │   └── parametric.rs       # from moonlitt-eq
    └── spatial/
        ├── mod.rs
        ├── reverb.rs           # from moonlitt-reverb (Freeverb + Dattorro)
        └── convolver.rs        # from moonlitt-convolver (FFT partitioned)
```

### Cargo.toml

```toml
[package]
name = "moonlitt-effects"

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
rustfft = { version = "6", optional = true }

[features]
default = ["all"]
all = ["dynamics", "eq", "spatial"]

# Category level
dynamics    = ["compressor"]
eq          = ["parametric-eq"]
spatial     = ["reverb", "convolver"]

# Individual effect level
compressor     = []
parametric-eq  = []
reverb         = []
convolver      = ["dep:rustfft"]
```

### lib.rs

```rust
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

#[cfg(feature = "reverb")]
pub use spatial::reverb::{Reverb, DattorroReverb};

#[cfg(feature = "convolver")]
pub use spatial::convolver::Convolver;
```

### Migration

Each effect crate's `src/lib.rs` moves to the corresponding module file. Only `use` paths change. All effects implement `AudioBackend` — interface unchanged. Old crates deleted from workspace.

### Adding Future Effects

1. Add `src/dynamics/limiter.rs`
2. Add `limiter = []` feature, add to `dynamics` feature list
3. Add re-export in `lib.rs`
4. Structure is ready — no architecture changes needed

---

## Layer 2: moonlitt-engine (Slim Down)

### Before

`Engine` struct with ~250 lines wrapping `Option<Box<dyn AudioBackend>>`, proxying every method call through to the backend. Stores `sample_rate`, `buffer_size`, `volume`, `loaded_path` — all either duplicated from AudioBackend or metadata that belongs elsewhere.

### After

Pure module with factory function + plugin scanning. ~100 lines.

```
crates/moonlitt-engine/
├── Cargo.toml
└── src/
    ├── lib.rs            # create() + scan_plugins() + supported_formats()
    ├── error.rs          # EngineError
    └── plugin_info.rs    # PluginInfo + PluginFormat
```

### Deleted

- `engine.rs` (Engine struct + all proxy methods)
- `backend.rs` (trait re-export — callers use `moonlitt_core::AudioBackend` directly)
- `backends/` directory (each backend lives in its own crate)

### Core API

```rust
use moonlitt_core::AudioBackend;

pub fn create(
    path: &str,
    sample_rate: u32,
    buffer_size: u32,
) -> Result<Box<dyn AudioBackend>, EngineError> {
    match extension(path) {
        #[cfg(feature = "sf2")]
        "sf2" => { /* OxiSynthBackend or SamplerBackend */ }
        #[cfg(feature = "vst3")]
        "vst3" => { /* Vst3Backend */ }
        #[cfg(feature = "clap")]
        "clap" => { /* ClapBackend */ }
        ext => Err(EngineError::UnsupportedFormat(ext.into())),
    }
}

pub fn scan_plugins(dirs: &[&str]) -> Vec<PluginInfo> { ... }
pub fn supported_formats() -> Vec<&'static str> { ... }
```

### Cargo.toml

```toml
[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
moonlitt-sampler = { path = "../moonlitt-sampler", optional = true }
moonlitt-vst3    = { path = "../moonlitt-vst3", optional = true }
moonlitt-clap    = { path = "../moonlitt-clap", optional = true }
oxisynth         = { path = "../../deps/oxisynth/oxisynth", optional = true }

[features]
default = ["sf2"]
sf2         = ["dep:oxisynth"]
sf2-sampler = ["dep:moonlitt-sampler"]
vst3        = ["dep:moonlitt-vst3"]
clap        = ["dep:moonlitt-clap"]
```

### Impact

Callers change from:

```rust
let mut engine = Engine::new(44100, 512);
engine.load("piano.sf2")?;
engine.note_on(0, 60, 100);
```

To:

```rust
let mut backend = moonlitt_engine::create("piano.sf2", 44100, 512)?;
backend.note_on(0, 60, 100);
```

`loaded_path` metadata moves to `Track::source_path` in moonlitt-mixer.

---

## Layer 3a: moonlitt-mixer (Extract)

### Source

Extracted from `moonlitt-runtime/src/mixer.rs` (1436 lines). The code comment at line 9-10 already states: "If the crate dependency graph ever needs it, Mixer can be extracted to its own `moonlitt-mixer` crate with zero API changes."

### Directory Structure

```
crates/moonlitt-mixer/
├── Cargo.toml
└── src/
    ├── lib.rs           # pub mod + re-exports
    ├── mixer.rs         # Mixer struct (core audio graph render)     ~400 lines
    ├── track.rs         # Track struct + insert chain + pan + routing ~350 lines
    ├── send.rs          # SendBus struct + send routing               ~150 lines
    ├── master.rs        # MasterBus struct + limiter                  ~150 lines
    ├── meter.rs         # LevelMeter (peak, RMS, true peak EBU R128) ~150 lines
    ├── pdc.rs           # Plugin Delay Compensation delay lines       ~100 lines
    └── dither.rs        # TPDF dither (from runtime/dither.rs)        ~80 lines
```

Each file 80-400 lines, within coding standards.

### Cargo.toml

```toml
[package]
name = "moonlitt-mixer"

[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
```

Single dependency — only needs `AudioBackend` trait and `AudioEvent`. No cpal, no rtrb, no platform dependencies.

### Core API

```rust
use moonlitt_core::{AudioBackend, AudioEvent};

pub struct Mixer { ... }

impl Mixer {
    pub fn new(sample_rate: u32) -> Self;

    // Track management
    pub fn add_track(&mut self, backend: Box<dyn AudioBackend>,
                     source_path: Option<String>, channel_mask: u16) -> usize;
    pub fn remove_track(&mut self, index: usize);
    pub fn add_insert(&mut self, track: usize, effect: Box<dyn AudioBackend>) -> usize;
    pub fn add_send_bus(&mut self, backend: Box<dyn AudioBackend>) -> usize;
    pub fn set_track_route(&mut self, track: usize, target: RouteTarget);

    // Parameter control
    pub fn set_track_volume(&mut self, track: usize, db: f32);
    pub fn set_track_pan(&mut self, track: usize, pan: f32);
    pub fn set_track_trim(&mut self, track: usize, db: f32);
    pub fn set_track_mute(&mut self, track: usize, muted: bool);
    pub fn set_track_solo(&mut self, track: usize, solo: bool);
    pub fn set_master_volume(&mut self, db: f32);

    // Event dispatch
    pub fn dispatch_event(&mut self, event: AudioEvent);

    // Render
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]);

    // Metering
    pub fn track_levels(&self, track: usize) -> (f32, f32);
    pub fn master_levels(&self) -> (f32, f32);

    // Metadata for session persistence
    pub fn track_source_path(&self, track: usize) -> Option<&str>;
    pub fn track_count(&self) -> usize;
}
```

### Track Metadata

`loaded_path` from the deleted Engine struct moves to Track:

```rust
pub struct Track {
    pub(crate) backend: Box<dyn AudioBackend>,
    pub(crate) source_path: Option<String>,    // formerly Engine::loaded_path
    pub(crate) channel_mask: u16,
    pub(crate) volume_db: f32,
    pub(crate) pan: f32,
    pub(crate) trim_db: f32,
    pub(crate) muted: bool,
    pub(crate) solo: bool,
    pub(crate) inserts: Vec<Insert>,
    pub(crate) sends: Vec<SendConfig>,
    pub(crate) route: RouteTarget,
    pub(crate) meter: LevelMeter,
    pub(crate) pdc_delay: PdcDelay,
}
```

---

## Layer 3b: moonlitt-session (Extract)

### Role

The platform-agnostic orchestration core. Equivalent to Vue's `@vue/runtime-core`. Knows how to orchestrate mixer + transport + sequencer + events, but does not touch any hardware.

### Directory Structure

```
crates/moonlitt-session/
├── Cargo.toml
└── src/
    ├── lib.rs              # pub mod + re-exports
    ├── session.rs          # Session struct (control plane, main thread)
    ├── processor.rs        # AudioProcessor (process plane, audio thread)
    ├── sequencer.rs        # MIDI file playback (from runtime)
    ├── transport.rs        # Play/Pause/Stop state machine (from runtime)
    ├── event_bus.rs        # Event queue management (rtrb Producer/Consumer)
    └── persistence.rs      # Session save/load JSON serialization (from runtime/session.rs)
```

### Cargo.toml

```toml
[package]
name = "moonlitt-session"

[dependencies]
moonlitt-core  = { path = "../moonlitt-core" }
moonlitt-mixer = { path = "../moonlitt-mixer" }
rtrb           = "0.3"
midly          = "0.5"
serde          = { version = "1", features = ["derive"] }
serde_json     = "1"
base64         = "0.22"
```

No cpal, no midir — pure platform-agnostic.

### Core Design: Session + AudioProcessor Split

The fundamental constraint of audio systems: control thread and audio thread cannot share mutable state.

```rust
/// Control plane (main thread holds this, sends commands)
pub struct Session {
    producer: rtrb::Producer<AudioEvent>,
    transport: Transport,           // shared via Arc<Atomic*>
    shared_meters: Arc<SharedMeters>,
}

/// Process plane (audio thread holds this, executes rendering)
pub struct AudioProcessor {
    mixer: Mixer,
    sequencer: Sequencer,
    transport: Transport,           // shared with Session
    consumer: rtrb::Consumer<AudioEvent>,
    shared_meters: Arc<SharedMeters>,
}
```

`Session::new()` returns both as a pair:

```rust
impl Session {
    pub fn new(sample_rate: u32, buffer_size: u32) -> (Session, AudioProcessor);
}
```

The `AudioProcessor` is moved into the platform's audio callback. The `Session` stays on the main thread for control.

### Session Public API

```rust
impl Session {
    pub fn new(sample_rate: u32, buffer_size: u32) -> (Session, AudioProcessor);

    // Track management — structural mutations.
    // Since Mixer lives in AudioProcessor (audio thread), these must be sent via
    // a dedicated structural command channel (separate from AudioEvent queue).
    // Uses a bounded crossbeam channel or second rtrb for Box<dyn AudioBackend> transfer.
    // AudioProcessor drains structural commands at the start of each process() call,
    // before event dispatch, to ensure the mixer is in a consistent state.
    pub fn add_track(&mut self, backend: Box<dyn AudioBackend>,
                     source_path: Option<&str>, channel_mask: u16) -> usize;
    pub fn remove_track(&mut self, track: usize);
    pub fn add_insert(&mut self, track: usize, effect: Box<dyn AudioBackend>) -> usize;
    pub fn add_send_bus(&mut self, backend: Box<dyn AudioBackend>) -> usize;

    // Transport
    pub fn play(&self);
    pub fn pause(&self);
    pub fn stop(&self);
    pub fn set_tempo(&self, bpm: f64);
    pub fn set_loop(&self, enabled: bool);

    // MIDI (via event bus)
    pub fn note_on(&self, track: usize, channel: u8, note: u8, velocity: u8);
    pub fn note_off(&self, track: usize, channel: u8, note: u8);
    pub fn cc(&self, track: usize, channel: u8, cc: u8, value: u8);

    // Mixer control (via event bus)
    pub fn set_track_volume(&self, track: usize, db: f32);
    pub fn set_track_pan(&self, track: usize, pan: f32);
    pub fn set_track_mute(&self, track: usize, muted: bool);
    pub fn set_track_solo(&self, track: usize, solo: bool);
    pub fn set_master_volume(&self, db: f32);

    // Sequencer
    pub fn load_midi(&mut self, path: &str) -> Result<(), Error>;
    pub fn load_midi_bytes(&mut self, bytes: &[u8]) -> Result<(), Error>;

    // Metering (atomic read, lock-free)
    pub fn track_levels(&self, track: usize) -> (f32, f32);
    pub fn master_levels(&self) -> (f32, f32);

    // Persistence
    pub fn save(&self, path: &str) -> Result<(), Error>;
    /// Returns (Session, AudioProcessor) pair, same as new().
    /// Restores mixer state, track routing, effects, and engine state from saved session.
    pub fn load(path: &str, engine_loader: &dyn Fn(&str) -> Result<Box<dyn AudioBackend>, Error>)
        -> Result<(Session, AudioProcessor), Error>;
}
```

### AudioProcessor

```rust
impl AudioProcessor {
    /// Called by platform binding in audio callback
    pub fn process(&mut self, output: &mut [f32]);
}
```

Logic: drain events -> advance sequencer -> render mixer -> interleave stereo output.

### Metering Cross-Thread

```rust
pub struct SharedMeters {
    tracks: Vec<SharedMeter>,
    master: SharedMeter,
}

pub struct SharedMeter {
    peak_l: AtomicU32,    // f32::to_bits / from_bits
    peak_r: AtomicU32,
}
```

AudioProcessor writes after each render. Session reads at any time. No locks.

---

## Layer 4: Platform Bindings

### moonlitt-audio-io (Native Audio)

```
crates/moonlitt-audio-io/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── audio_output.rs    # cpal output stream (from runtime)
    └── midi_input.rs      # midir MIDI input (from runtime)
```

```toml
[dependencies]
moonlitt-core    = { path = "../moonlitt-core" }
moonlitt-session = { path = "../moonlitt-session" }
cpal = "0.15"
midir = "0.10"
```

```rust
pub struct CpalHost { ... }

impl AudioHost for CpalHost {
    fn start(&mut self, callback: AudioCallback) -> Result<(), Error> {
        // Build cpal output stream
        // Move callback into cpal closure
    }
    fn stop(&mut self) { ... }
    fn sample_rate(&self) -> u32 { ... }
    fn buffer_size(&self) -> u32 { ... }
}

pub struct MidiInput { ... }

impl MidiInput {
    pub fn list_devices() -> Vec<MidiDevice>;
    pub fn connect(device: &MidiDevice, session: &Session) -> Result<Self, Error>;
}
```

### moonlitt-capi (C ABI Binding)

```
crates/moonlitt-capi/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── engine_api.rs      # moonlitt_create() etc.
    ├── session_api.rs     # moonlitt_session_*() series
    ├── effects_api.rs     # moonlitt_effects_*() series
    └── util.rs            # C string conversion, error handling
```

```toml
[dependencies]
moonlitt-core     = { path = "../moonlitt-core" }
moonlitt-engine   = { path = "../moonlitt-engine" }
moonlitt-session  = { path = "../moonlitt-session" }
moonlitt-effects  = { path = "../moonlitt-effects" }
moonlitt-audio-io = { path = "../moonlitt-audio-io", optional = true }

[features]
default = ["audio-io"]
audio-io = ["dep:moonlitt-audio-io"]   # Game engines may manage their own audio thread

[lib]
crate-type = ["cdylib", "rlib"]
```

Key design: `audio-io` is optional. Unity/Unreal have their own audio threads — they only need `Session` + `AudioProcessor` and call `processor.process()` themselves.

```c
// C API usage
MoonlittBackend* b = moonlitt_create("piano.sf2", 44100, 512);
MoonlittSession* s = NULL;
MoonlittProcessor* p = NULL;
moonlitt_session_new(44100, 512, &s, &p);
int track = moonlitt_session_add_track(s, b, "piano.sf2", 0xFFFF);

// Option A: built-in audio (standalone C program)
moonlitt_audio_start(p);

// Option B: game engine drives audio (Unity/Unreal)
moonlitt_processor_process(p, output_buffer, buffer_len);

moonlitt_session_play(s);
moonlitt_session_note_on(s, track, 0, 60, 100);
moonlitt_session_set_track_volume(s, track, -3.0f);
```

### moonlitt-node (Node.js Binding)

```
crates/moonlitt-node/
├── Cargo.toml
├── build.rs           # napi-rs build config
├── npm/               # prebuilt packages per platform
│   ├── darwin-arm64/
│   ├── darwin-x64/
│   ├── win32-x64/
│   └── linux-x64/
└── src/
    ├── lib.rs
    ├── engine.rs      # create(), scanPlugins()
    ├── session.rs     # Session class
    ├── effects.rs     # effect factories
    └── types.rs       # TypeScript type exports
```

```toml
[dependencies]
moonlitt-core     = { path = "../moonlitt-core" }
moonlitt-engine   = { path = "../moonlitt-engine", features = ["sf2", "vst3", "clap"] }
moonlitt-session  = { path = "../moonlitt-session" }
moonlitt-effects  = { path = "../moonlitt-effects" }
moonlitt-audio-io = { path = "../moonlitt-audio-io" }
napi = { version = "3", features = ["async"] }
napi-derive = "3"

[lib]
crate-type = ["cdylib"]
```

```rust
use napi_derive::napi;

#[napi]
pub fn create(path: String, sample_rate: u32, buffer_size: u32) -> Result<Backend>;

#[napi]
pub fn scan_plugins(dirs: Vec<String>) -> Vec<PluginInfo>;

#[napi]
pub struct Session { inner: moonlitt_session::Session }

#[napi]
impl Session {
    #[napi(constructor)]
    pub fn new(sample_rate: u32, buffer_size: u32) -> Self;
    #[napi]
    pub fn add_track(&mut self, backend: &mut Backend, path: Option<String>) -> u32;
    #[napi]
    pub fn play(&self);
    #[napi]
    pub fn pause(&self);
    #[napi]
    pub fn stop(&self);
    #[napi]
    pub fn note_on(&self, track: u32, channel: u8, note: u8, velocity: u8);
    #[napi]
    pub fn note_off(&self, track: u32, channel: u8, note: u8);
    #[napi]
    pub fn set_track_volume(&self, track: u32, db: f64);
    #[napi]
    pub fn set_track_pan(&self, track: u32, pan: f64);
    #[napi]
    pub fn track_levels(&self, track: u32) -> TrackLevels;
    #[napi]
    pub fn master_levels(&self) -> TrackLevels;
    #[napi]
    pub fn save(&self, path: String) -> Result<()>;
    #[napi]
    pub fn start_audio(&mut self) -> Result<()>;
}
```

napi-rs auto-generates TypeScript type definitions (`index.d.ts`).

### Cross-Binding API Consistency

| Operation | audio-io (Rust) | capi (C) | node (JS) |
|-----------|----------------|----------|-----------|
| Create backend | `engine::create(p,sr,bs)` | `moonlitt_create(p,sr,bs)` | `create(p,sr,bs)` |
| Create session | `Session::new(sr,bs)` | `moonlitt_session_new(sr,bs,&s,&p)` | `new Session(sr,bs)` |
| Add track | `s.add_track(b,path,mask)` | `moonlitt_session_add_track(s,b,p,m)` | `s.addTrack(b,path)` |
| Play | `s.play()` | `moonlitt_session_play(s)` | `s.play()` |
| Note on | `s.note_on(t,c,n,v)` | `moonlitt_session_note_on(s,t,c,n,v)` | `s.noteOn(t,c,n,v)` |
| Volume | `s.set_track_volume(t,db)` | `moonlitt_session_set_track_volume(s,t,db)` | `s.setTrackVolume(t,db)` |
| Meter | `s.track_levels(t)` | `moonlitt_session_track_levels(s,t,&l,&r)` | `s.trackLevels(t)` |
| Start audio | `host.start(processor)` | `moonlitt_audio_start(p)` | `s.startAudio()` |

Same semantics, platform-idiomatic style.

---

## Application Layer

### moonlitt-cli

```toml
[dependencies]
moonlitt-engine   = { path = "../moonlitt-engine", features = ["sf2", "vst3", "clap"] }
moonlitt-session  = { path = "../moonlitt-session" }
moonlitt-audio-io = { path = "../moonlitt-audio-io" }
moonlitt-effects  = { path = "../moonlitt-effects" }
clap = "4"
midly = "0.5"
hound = "3"
```

Change `Runtime::new(engine)` to `Session::new()` + `CpalHost`. API names map nearly 1:1.

### moonlitt-test-suite

Update import paths. Tests must not be weakened — all existing test assertions remain at the same strictness level. New tests added for extracted modules.

---

## Testing Strategy

**Strict TDD throughout.** Every phase must pass all existing tests before and after refactoring.

### Per-Crate Test Requirements

| Crate | Test Focus |
|-------|-----------|
| `moonlitt-core` | AudioEvent size assertion (<=16 bytes), BackendCaps bitflag operations |
| `moonlitt-effects` | All existing effect tests migrated 1:1, per-feature compilation verification |
| `moonlitt-engine` | Factory function returns correct backend type per extension, unsupported format error |
| `moonlitt-mixer` | All existing mixer tests from runtime, new: track routing, PDC compensation, metering accuracy, group submix cycle detection |
| `moonlitt-session` | Transport state machine, sequencer MIDI parsing, event bus producer/consumer, persistence round-trip, Session+AudioProcessor split correctness |
| `moonlitt-audio-io` | CpalHost start/stop lifecycle, MidiInput device enumeration |
| `moonlitt-capi` | All existing FFI tests migrated, new Session-based API tests |
| `moonlitt-node` | napi binding smoke tests, TypeScript type generation verification |
| `moonlitt-test-suite` | All existing compliance tests pass unchanged (SF2 2.04, MIDI 1.0, EBU R128, AES17) |

### Test Non-Negotiables

1. **No test weakening** — if a test expects 0.001 tolerance, it stays at 0.001
2. **No `#[ignore]` to pass** — fix the code, not the test
3. **Compliance suite unchanged** — SF2 2.04 S1-S20, EBU R128 metering, AES17 signal quality tests must all pass with identical thresholds
4. **Coverage** — new code requires 80%+ test coverage

---

## Migration Plan

Execute bottom-up by dependency order. Each phase ends with `cargo build --workspace && cargo test --workspace` passing.

```
Phase 1 — Foundation
  (1) moonlitt-core: add AudioEvent, BackendCaps, AudioHost trait
  (2) moonlitt-effects: merge 4 effect crates, delete old crates

Phase 2 — Core Split
  (3) moonlitt-mixer: extract from runtime
  (4) moonlitt-engine: slim down (delete Engine struct, keep factory)
  (5) moonlitt-session: extract from runtime

Phase 3 — Platform Bindings
  (6) moonlitt-audio-io: extract from runtime (runtime deleted after this)
  (7) moonlitt-capi: restructure from ffi
  (8) moonlitt-node: new crate (napi-rs)

Phase 4 — Application
  (9) moonlitt-cli: update dependencies
  (10) moonlitt-test-suite: update imports, verify all tests pass
```

### Phase Dependencies

- Phase 2 depends on Phase 1 (mixer and session need AudioEvent from core)
- Phase 3 depends on Phase 2 (bindings need session and mixer)
- Phase 4 depends on Phase 3 (CLI needs audio-io)
- Within each phase, steps are sequential (numbered order)

---

## Future Extensions

This architecture naturally accommodates planned additions:

| Addition | How It Fits |
|----------|------------|
| AU plugin hosting | New `moonlitt-au` crate implementing AudioBackend + `au` feature in engine |
| WASM binding | New `moonlitt-wasm` crate depending on session (no cpal) |
| New effects (limiter, delay, chorus) | Add to appropriate category in moonlitt-effects with feature flag |
| React/TS UI packages | `packages/` directory alongside `crates/`, consuming moonlitt-node |
| Python binding | New crate via PyO3, depending on session |

No architectural changes needed for any of these.
