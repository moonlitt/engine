# Moonlitt Mixer Architecture Design

## Position

Moonlitt is a **headless DAW** — full audio engine capabilities, no GUI. All sound generation comes from plugins (VST3/CLAP) or sfizz. No built-in synthesizer.

## Current Problems

1. Multiple engines output to separate cpal streams — no shared mixing
2. No master bus (no unified volume, no limiter)
3. No send bus (no shared reverb/effects)
4. No pan control
5. FluidLite is a mediocre SF2 player (7-point sinc)
6. SF2 and SFZ use different engines with different quality ceilings

## Target Architecture

```
                    ┌──────────────────────────────────────────┐
                    │                 Runtime                   │
                    │  ┌────────────────────────────────────┐   │
                    │  │              Mixer                  │   │
                    │  │                                    │   │
                    │  │  Track 0 ─[vol|pan|mute|send]──┐   │   │
                    │  │  Track 1 ─[vol|pan|mute|send]──┤   │   │
                    │  │  Track N ─[vol|pan|mute|send]──┤   │   │
                    │  │                                ↓   │   │
                    │  │              ┌─── Sum Bus ◄────┘   │   │
                    │  │              │                      │   │
                    │  │              ├─→ Send Bus A ────┐   │   │
                    │  │              │   (VST3 reverb)  │   │   │
                    │  │              ├─→ Send Bus B ────┤   │   │
                    │  │              │   (VST3 delay)   │   │   │
                    │  │              │                  ↓   │   │
                    │  │              └──→ Master Bus ◄──┘   │   │
                    │  │                  [vol + limiter]    │   │
                    │  └────────────────────────────────────┘   │
                    │                      ↓                     │
                    │                 cpal stream                │
                    └──────────────────────────────────────────┘
```

**One cpal stream. All mixing in Rust. Zero hardware-level mixing.**

## Component Design

### 1. Track

A track owns one Engine and handles a set of MIDI channels.

```rust
pub struct Track {
    id: u32,
    engine: Engine,
    channel_mask: u16,       // bitmask: which MIDI channels route here
    volume: f32,             // 0.0-1.0
    pan: f32,                // -1.0 (L) to 1.0 (R)
    mute: bool,
    solo: bool,
    send_levels: Vec<f32>,   // per send bus
    left: Vec<f32>,          // pre-allocated render buffer
    right: Vec<f32>,
}
```

### 2. SendBus

A send bus holds an effect plugin (VST3/CLAP) and accumulates audio from tracks.

```rust
pub struct SendBus {
    id: u32,
    engine: Engine,          // loaded with a VST3/CLAP effect plugin
    level: f32,              // return level (how much effect goes to master)
    left: Vec<f32>,          // accumulation buffer
    right: Vec<f32>,
}
```

Effect plugins are loaded the same way as instruments — through our VST3/CLAP hosting. The difference: we feed audio INTO their input buffer (effect mode) instead of sending MIDI (instrument mode).

### 3. MasterBus

```rust
pub struct MasterBus {
    volume: f32,
    limiter_threshold: f32,  // soft limiter to prevent clipping
    left: Vec<f32>,
    right: Vec<f32>,
}
```

### 4. Mixer

```rust
pub struct Mixer {
    tracks: Vec<Track>,
    send_buses: Vec<SendBus>,
    master: MasterBus,
    buffer_size: usize,
    sample_rate: u32,
}
```

### 5. Render Pipeline (per audio callback)

```
for each chunk of buffer_size:
    1. Route events from queue to tracks (by channel_mask)
    2. Render each track:
       - engine.render(track.left, track.right)
       - apply volume
       - apply pan (constant-power: L *= cos(θ), R *= sin(θ))
       - if muted or (any_solo && !this.solo): zero output
    3. Sum all track outputs into master.left/right
    4. For each send bus:
       - accumulate: sum(track.left * track.send_levels[bus_id])
       - process: send_bus.engine.render(input → output)  [effect mode]
       - mix into master: master += send_bus.output * send_bus.level
    5. Apply master volume
    6. Apply soft limiter (tanh for peaks > threshold)
    7. Output to cpal
```

### 6. Pan Law

Constant-power panning (industry standard):

```rust
fn apply_pan(left: &mut [f32], right: &mut [f32], pan: f32) {
    // pan: -1.0 (full left) to 1.0 (full right)
    let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI; // 0 to π/2
    let gain_l = angle.cos();
    let gain_r = angle.sin();
    for s in left.iter_mut() { *s *= gain_l; }
    for s in right.iter_mut() { *s *= gain_r; }
}
```

### 7. Soft Limiter

```rust
fn soft_limit(sample: f32, threshold: f32) -> f32 {
    if sample.abs() <= threshold {
        sample
    } else {
        threshold * (sample / sample.abs()) * (1.0 + ((sample.abs() - threshold) / (1.0 - threshold)).tanh() * (1.0 - threshold) / threshold)
    }
    // Simplified: tanh(sample) for |sample| > threshold
}
```

## SF2 → SFZ Conversion

### Why

sfizz is SFZ-only. SF2 users should get sfizz quality (Sinc 72) without manual conversion.

### Converter: moonlitt-sf2-import

A new crate or module that:

1. Parses SF2 file (using `soundfont-rs` or custom parser)
2. Extracts PCM samples → writes as WAV files to a cache directory
3. Generates SFZ mapping file from SF2 instrument/preset definitions:
   - Key ranges (`lokey`, `hikey`)
   - Velocity layers (`lovel`, `hivel`)
   - Loop points (`loop_mode`, `loop_start`, `loop_end`)
   - Tuning (`tune`, `pitch_keycenter`)
   - Volume envelope (`ampeg_attack`, `ampeg_decay`, `ampeg_sustain`, `ampeg_release`)
   - Filter (`fil_type`, `cutoff`, `resonance`)
4. Returns path to generated SFZ

### Cache Strategy

```
~/.moonlitt/sf2-cache/
  <sha256-of-sf2>/
    preset_000_Acoustic_Grand_Piano.sfz
    preset_001_Bright_Acoustic_Piano.sfz
    samples/
      sample_0001.wav
      sample_0002.wav
      ...
```

SHA-256 of the SF2 file prevents re-conversion. Cache is persistent across sessions.

### Integration

```rust
// engine.load() auto-detects:
fn load(&mut self, path: &str) -> Result<(), EngineError> {
    match extension {
        "sf2" => {
            let sfz_path = sf2_import::convert_or_cache(path)?;
            self.load_sfizz(&sfz_path)
        }
        "sfz" => self.load_sfizz(path),
        "vst3" => self.load_vst3(path),
        "clap" => self.load_clap(path),
    }
}
```

## sfizz Integration

### Approach: libsfizz via C FFI

Link libsfizz as a C library (BSD-2 license, redistributable).

```rust
// New backend: SfizzBackend
pub struct SfizzBackend {
    synth: *mut sfizz_synth_t,
    sample_rate: u32,
    buffer_size: u32,
}

impl AudioBackend for SfizzBackend {
    fn load(&mut self, path: &str) {
        sfizz_load_file(self.synth, path);
    }
    fn note_on(&mut self, ch: u8, note: u8, vel: u8) {
        sfizz_send_note_on(self.synth, 0, note, vel);
    }
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        sfizz_render_block(self.synth, channels.as_ptr(), 2, left.len());
    }
    // ... params map to sfizz_set_sample_quality, etc.
}
```

### Build

sfizz requires: C++17 compiler, CMake, libsndfile. Use `cmake` crate in build.rs to compile from source (vendored submodule).

### Sample Quality Mapping

sfizz `sample_quality` 0-10:
- 0: nearest (fastest)
- 1: linear
- 2: polynomial (default)
- 3-10: sinc with increasing points

We set **quality 10** (Sinc 72) by default. Users can adjust via parameter API.

## Removing FluidLite

### What gets removed

- `moonlitt-engine/src/backends/sf2.rs` — entire file
- `fluidlite` dependency from Cargo.toml
- `sf2` feature flag
- All FluidLite-specific parameter definitions

### What replaces it

- `moonlitt-engine/src/backends/sfizz.rs` — SfizzBackend
- `moonlitt-sf2-import/` — new crate for SF2→SFZ conversion
- `sfizz-sys/` — new crate for sfizz C FFI bindings

## Mixer FFI

```c
// Track management
int   moonlitt_mixer_add_track(RuntimeHandle* rt, EngineHandle* engine) → track_id
void  moonlitt_mixer_remove_track(RuntimeHandle* rt, int track_id)
void  moonlitt_mixer_set_track_volume(RuntimeHandle* rt, int track_id, float vol)
void  moonlitt_mixer_set_track_pan(RuntimeHandle* rt, int track_id, float pan)
void  moonlitt_mixer_set_track_mute(RuntimeHandle* rt, int track_id, bool mute)
void  moonlitt_mixer_set_track_solo(RuntimeHandle* rt, int track_id, bool solo)
void  moonlitt_mixer_set_track_channels(RuntimeHandle* rt, int track_id, int mask)
void  moonlitt_mixer_set_track_send(RuntimeHandle* rt, int track_id, int bus, float level)

// Send bus management
int   moonlitt_mixer_add_send(RuntimeHandle* rt, EngineHandle* effect) → bus_id
void  moonlitt_mixer_remove_send(RuntimeHandle* rt, int bus_id)
void  moonlitt_mixer_set_send_level(RuntimeHandle* rt, int bus_id, float level)

// Master
void  moonlitt_mixer_set_master_volume(RuntimeHandle* rt, float vol)

// Query
char* moonlitt_mixer_info_json(RuntimeHandle* rt)  // track list + bus list
```

## VST3/CLAP Effect Mode

Currently our hosting only supports instrument mode (MIDI → audio out). Effect mode needs:

### VST3 Effect Mode

The VST3 `process()` already accepts both audio inputs and outputs. For instruments, we pass empty inputs. For effects, we pass the send bus accumulation buffer as input:

```rust
// Current (instrument):
process_data.audio_inputs = null
process_data.audio_outputs = &output_buffer

// Effect mode:
process_data.audio_inputs = &send_accumulation_buffer  // audio IN
process_data.audio_outputs = &processed_buffer          // audio OUT
```

Both Vst3Plugin and ClapPlugin need a new `process_replacing(input, output)` method alongside the existing `render(output)`.

### CLAP Effect Mode

Same principle — `clap_process` already has `audio_inputs` and `audio_outputs` fields.

## Implementation Order

1. **Mixer core** (`moonlitt-mixer` or in `moonlitt-runtime`)
   - Mixer, Track, SendBus, MasterBus structs
   - Render pipeline with pan, volume, mute/solo, send routing
   - Soft limiter

2. **AudioThread integration**
   - Replace single Engine with Mixer
   - Event routing by channel_mask

3. **VST3/CLAP effect mode**
   - `process_replacing(input_l, input_r, output_l, output_r)`
   - Wire into SendBus

4. **sfizz backend**
   - sfizz-sys crate (bindgen from sfizz.h)
   - SfizzBackend implementing AudioBackend
   - Build sfizz from source via cmake

5. **SF2→SFZ converter**
   - Parse SF2 (soundfont-rs)
   - Extract samples to WAV
   - Generate SFZ mapping
   - SHA-256 cache

6. **Remove FluidLite**
   - Delete sf2.rs backend
   - Remove fluidlite dependency
   - Update feature flags

7. **Mixer FFI**
   - All mixer control functions
   - JSON info query

8. **C# bindings update**
   - NativeEngine uses mixer FFI
   - AudioManager routes through mixer API instead of manual multi-engine

9. **Tests**
   - Mixer unit tests (pan law, volume, mute/solo, send routing)
   - sfizz integration test
   - SF2→SFZ conversion test
   - E2E: SF2 file → sfizz → mixer → cpal

## Non-Goals (v1)

- Plugin delay compensation (PDC) — all plugins assumed zero latency
- Sidechain routing
- Multi-output plugins (>2 channels)
- Insert effects (per-track effect chain) — only send effects
- Undo/redo for mixer state
