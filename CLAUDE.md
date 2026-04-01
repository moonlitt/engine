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
moonlitt-runtime       ← Real-time audio I/O (cpal), MIDI input (midir), sequencer, mixer
moonlitt-ffi           ← C API for language bindings (.NET, future Python/Node.js)
moonlitt-cli           ← CLI tool: scan, play, live, midi-devices

Effects (independent, no cross-deps):
  moonlitt-eq          ← 8-band parametric EQ (biquad cascade)
  moonlitt-compressor  ← Log-domain dynamics compressor
  moonlitt-reverb      ← Freeverb (8 comb + 4 allpass) + Dattorro plate reverb
  moonlitt-convolver   ← FFT partitioned convolution (overlap-add, rustfft)

moonlitt-test-suite    ← DSP compliance tests (SF2 2.04, MIDI 1.0, EBU R128, AES17)
```

### Key Abstractions

- **`AudioBackend` trait** (`moonlitt-core`): Every audio source implements this — MIDI I/O, audio rendering, parameters, presets, state save/load. Community-extensible.
- **`Engine`** (`moonlitt-engine`): Wraps a `Box<dyn AudioBackend>`. Routes `.sf2` → sampler, `.vst3` → VST3 host, `.clap` → CLAP host.
- **`Runtime`** (`moonlitt-runtime`): Owns an `Engine`, connects it to audio hardware via `cpal`. Lock-free SPSC ring buffer (`rtrb`) passes events between control thread and audio thread.
- **Mixer** (`moonlitt-runtime/src/mixer.rs`): Multi-track with send buses, panning, and metering. Tracks → Send Buses → Master Bus → Output.

### Real-time Safety

The audio thread (`audio_thread.rs`) must never allocate or block. All communication uses the `rtrb` lock-free SPSC queue. Keep this invariant when modifying runtime code.

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

## FFI Layer

`moonlitt-ffi` exposes the full API as `extern "C"` functions. It builds as both `cdylib` and `rlib`. The C# bindings live in `bindings/dotnet/`. When modifying FFI: the `moonlitt_runtime_create` function consumes the engine even on failure — this is a known contract issue documented in `docs/2026-03-27-deep-review-findings.md`.
