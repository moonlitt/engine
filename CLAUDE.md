# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build --workspace                  # Build all 14 crates
cargo build --release                    # Release build
cargo test --workspace                   # Run all tests
cargo test --workspace -- --skip pianoteq --skip keyscape  # Skip plugin-specific tests (CI default)
cargo test -p moonlitt-sampler           # Test a single crate
cargo test -p moonlitt-test-suite test_name  # Run a single test
cargo clippy --workspace -- -D warnings -A clippy::not_unsafe_ptr_arg_deref  # Lint (CI strictness)
```

CI runs on macOS only. Tests that require audio devices skip gracefully when hardware is unavailable.

## Architecture

Moonlitt is a pure Rust audio engine — no C++ wrappers or JUCE. It hosts VST3/CLAP plugins and synthesizes SF2 soundfonts.

### Crate Dependency Graph

```
moonlitt-core          ← Central trait: AudioBackend (all engines implement this)
  ↑
moonlitt-vst3          ← VST3 hosting via dlopen + COM vtable
moonlitt-clap          ← CLAP hosting via dlopen
moonlitt-sampler       ← Pure Rust SF2 synthesis (sinc interpolation)
  ↑ uses moonlitt-resampler (Kaiser-windowed sinc tables)
  ↑
moonlitt-engine        ← Unified entry point; auto-detects format by file extension
  ↑                      Feature-gated backends: sf2, sf2-sampler, sf2-legacy, vst3, clap
  ↑
moonlitt-mixer         ← Multi-track mixer: tracks, send buses, inserts, panning, metering, dither
moonlitt-session       ← Transport, sequencer, persistence, audio thread processor
moonlitt-audio-io      ← Platform audio I/O (cpal output, midir MIDI input, Runtime orchestrator)
moonlitt-capi          ← C API for language bindings (.NET, future Python)
moonlitt-node          ← Node.js binding via napi-rs (Web DAW, Electron, Ink terminal)
moonlitt-cli           ← CLI tool: scan, play, live, midi-devices

moonlitt-effects       ← Built-in audio effects (feature-gated modules):
  ↑                      dynamics/ — compressor (log-domain, soft knee)
  ↑                      eq/       — 8-band parametric EQ (biquad cascade)
  ↑                      spatial/  — Freeverb, Dattorro plate reverb, FFT convolver

moonlitt-test-suite    ← DSP compliance tests (SF2 2.04, MIDI 1.0, EBU R128, AES17)
```

### Key Abstractions

- **`AudioBackend` trait** (`moonlitt-core`): Every audio source implements this — MIDI I/O, audio rendering, parameters, presets, state save/load. Community-extensible.
- **`create()`** (`moonlitt-engine`): Factory function — routes `.sf2` → sampler, `.vst3` → VST3 host, `.clap` → CLAP host. Returns `Box<dyn AudioBackend>` directly.
- **`Runtime`** (`moonlitt-audio-io`): Owns a `Mixer`, connects it to audio hardware via `cpal`. Lock-free SPSC ring buffer (`rtrb`) passes events between control thread and audio thread.
- **Mixer** (`moonlitt-mixer`): Multi-track with send buses, inserts, panning, dither, and metering. Tracks → Send Buses → Master Bus → Output.
- **Session** (`moonlitt-session`): Transport (play/pause/stop), sequencer (MIDI file playback), persistence (save/load), and audio thread processor.

### Real-time Safety

The audio thread processor (`moonlitt-session/src/processor.rs`) must never allocate or block. All communication uses the `rtrb` lock-free SPSC queue. Keep this invariant when modifying audio processing code.

### External Dependencies in `deps/`

`deps/oxisynth` is a git submodule (excluded from workspace via `Cargo.toml`). It provides `oxisynth` (pure Rust SF2 synth) and `soundfont-rs` (SF2 parsing). Referenced by path in crate dependencies.

## Feature Flags (moonlitt-engine)

```
default = ["sf2"]           # oxisynth-based SF2
sf2-sampler                 # Pure Rust sampler (moonlitt-sampler, sinc interpolation)
sf2-legacy                  # FluidLite-based SF2 (C binding, needs bindgen)
vst3                        # VST3 plugin hosting
clap                        # CLAP plugin hosting
```

## Platform Bindings

**C API** (`moonlitt-capi`): Exposes the full API as `extern "C"` functions. Builds as both `cdylib` and `rlib`. C# bindings in `bindings/dotnet/`.

**Node.js** (`moonlitt-node`): napi-rs binding exposing Session, Backend, effects factories, and plugin scanning. For Web DAW (react-dom), desktop (Electron), and terminal UI (Ink).

Both bindings wrap the same `moonlitt-audio-io::Runtime` and provide the same semantic API in platform-idiomatic style.
