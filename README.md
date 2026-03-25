# Moonlitt

**Pure Rust audio engine for VST3, CLAP, and SF2**

[![CI](https://github.com/moonlitt/engine/actions/workflows/ci.yml/badge.svg)](https://github.com/moonlitt/engine/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)

---

## What is Moonlitt?

Moonlitt is a pure Rust audio engine that hosts VST3 and CLAP plugins and plays SF2 soundfonts. No C++ wrappers, no JUCE dependency -- just Rust talking directly to plugin ABIs. Load Pianoteq, Keyscape, Surge, or any VST3/CLAP instrument and render audio in real time or to WAV.

It ships as a library, a C FFI for language bindings, and a CLI tool.

## Quick Start

### As a Rust library

```rust
use moonlitt_engine::engine::Engine;

let mut engine = Engine::new(44100, 256);
engine.load("Pianoteq 9.vst3")?;
engine.note_on(0, 60, 100);

let mut left = vec![0.0f32; 256];
let mut right = vec![0.0f32; 256];
engine.render(&mut left, &mut right);
```

### As a CLI tool

```bash
# Scan for installed plugins
moonlitt scan

# Render a note to WAV
moonlitt play "Pianoteq 9.vst3" -n 60 -v 100 -d 2.0 -o piano.wav

# Play live through speakers
moonlitt play "Pianoteq 9.vst3" --live -n 60 -d 3

# List MIDI input devices
moonlitt midi-devices
```

### Real-time audio playback

```rust
use moonlitt_engine::engine::Engine;
use moonlitt_runtime::Runtime;

let mut engine = Engine::new(44100, 256);
engine.load("Pianoteq 9.vst3")?;

let mut rt = Runtime::new(engine)?;
rt.start()?;
rt.note_on(0, 60, 100);  // hear it immediately
```

## Architecture

```
moonlitt-vst3       Pure Rust VST3 host (dlopen + COM vtable calls)
moonlitt-clap       Pure Rust CLAP host
moonlitt-engine     Unified engine (auto-detects format by extension)
moonlitt-runtime    Real-time audio output (cpal) + MIDI input + sequencer
moonlitt-ffi        C API for language bindings
moonlitt-cli        Command-line tool
```

## Supported Formats

| Format | Type           | Status |
|--------|----------------|--------|
| VST3   | Plugin hosting | Verified with Pianoteq, Keyscape, Surge, Omnisphere |
| CLAP   | Plugin hosting | Implemented |
| SF2    | Sampler        | Via FluidLite |

## Language Bindings

| Language | Package          | Status  |
|----------|------------------|---------|
| Rust     | `moonlitt-engine`| Available |
| C/C++    | `moonlitt-ffi`   | Available |
| C#/.NET  | `Moonlitt.NET`   | Available |
| Node.js  | `@moonlitt/node` | Planned |
| Python   | `moonlitt-py`    | Planned |

## Building

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT), at your option.
