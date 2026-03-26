# Moonlitt Mixer Architecture Design (v2)

## Position

Moonlitt is a **headless DAW** — full audio engine capabilities, no GUI. Sound generation from plugins (VST3/CLAP) and OxiSynth (pure Rust SF2). Future: moonlitt-sampler (Sinc 72).

## Architecture

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

One cpal stream. All mixing in Rust. Zero hardware-level mixing.

## Completed (Steps 1-3)

### ✅ Mixer Core (mixer.rs)
- Track: engine + channel_mask + volume + pan + mute/solo + send_levels
- SendBus: effect engine + accumulation buffers + return level
- MasterBus: master volume + soft limiter (tanh)
- Constant-power pan law (cos/sin)
- 9 unit tests passing

### ✅ AudioThread Integration
- AudioThread owns Mixer (replaces single Engine)
- Event routing by channel_mask bitmask
- Sample-accurate delayed events with split rendering
- Runtime::new() wraps single Engine in Mixer (backward compat)
- Runtime::with_mixer() for pre-configured setups

### ✅ VST3/CLAP Effect Mode
- AudioBackend::process_effect(in_l, in_r, out_l, out_r)
- Vst3Plugin::process_effect — feeds audio into VST3 process() input bus
- ClapPlugin::process_effect — same for CLAP
- SendBus uses engine.process_effect() for shared effects

### ✅ SF2 Backend: OxiSynth
- Pure Rust, replaces FluidLite (kept as "sf2-legacy" feature)
- SeventhOrder interpolation (highest OxiSynth offers)
- Full parameter support (reverb, chorus, gain)
- Future: replace with moonlitt-sampler (Sinc 72)

### ✅ Sinc 72 Resampler (moonlitt-resampler)
- Pure Rust, zero dependencies
- Quality levels: Linear, Sinc8, Sinc16, Sinc36, Sinc48, Sinc72
- Kaiser window + pre-computed sinc tables
- 8 tests passing
- Ready for integration into moonlitt-sampler

## Remaining Steps

### Step 4: Mixer FFI

Expose mixer controls through C FFI for C#/game integration.

```c
// Track management (via AudioEvent ring buffer for thread safety)
int   moonlitt_mixer_add_track(RuntimeHandle* rt, EngineHandle* engine) → track_id
void  moonlitt_mixer_remove_track(RuntimeHandle* rt, int track_id)
void  moonlitt_mixer_set_track_volume(RuntimeHandle* rt, int track_id, float vol)
void  moonlitt_mixer_set_track_pan(RuntimeHandle* rt, int track_id, float pan)
void  moonlitt_mixer_set_track_mute(RuntimeHandle* rt, int track_id, int mute)
void  moonlitt_mixer_set_track_solo(RuntimeHandle* rt, int track_id, int solo)
void  moonlitt_mixer_set_track_channels(RuntimeHandle* rt, int track_id, int mask)
void  moonlitt_mixer_set_track_send(RuntimeHandle* rt, int track_id, int bus_id, float level)

// Send bus management
int   moonlitt_mixer_add_send(RuntimeHandle* rt, EngineHandle* effect) → bus_id
void  moonlitt_mixer_remove_send(RuntimeHandle* rt, int bus_id)
void  moonlitt_mixer_set_send_level(RuntimeHandle* rt, int bus_id, float level)

// Master
void  moonlitt_mixer_set_master_volume(RuntimeHandle* rt, float vol)

// Query
char* moonlitt_mixer_info_json(RuntimeHandle* rt)
```

**Thread safety**: Track volume/pan/mute/solo changes go through the ring buffer as MixerCommand events. Track add/remove use a separate mpsc channel (rare operations, non-realtime safe is OK).

### Step 5: C# Bindings Update

Replace the manual multi-NativeEngine routing in AudioManager with proper mixer FFI calls.

```csharp
// NativeEngine adds mixer methods:
public int MixerAddTrack(IntPtr engineHandle);
public void MixerSetTrackVolume(int trackId, float vol);
public void MixerSetTrackPan(int trackId, float pan);
public void MixerSetTrackMute(int trackId, bool mute);
public void MixerSetTrackSolo(int trackId, bool solo);
public void MixerSetTrackSend(int trackId, int busId, float level);
public int MixerAddSend(IntPtr effectHandle);
public void MixerSetMasterVolume(float vol);
public string? MixerInfoJson();
```

AudioManager changes:
- Remove `_channelEngines` dictionary (manual multi-engine routing)
- Remove per-engine cpal streams
- Use single Runtime with Mixer
- `SetChannelEngine()` → creates Engine, adds as Mixer track
- MixerMenu reads from `MixerInfoJson()` instead of TrackScanner

### Step 6: Tests

- Mixer FFI null safety tests
- C# round-trip: add track → set volume → query info
- E2E: multi-track rendering through single cpal stream
- Verify: piano_track command works through new mixer FFI

## Future (Not This Session)

### moonlitt-sampler (Sinc 72 SF2 Synth)
- World's first Sinc 72 pure Rust SF2 synthesizer
- soundfont-rs parser + moonlitt-resampler + ADSR/Filter/LFO/VoicePool
- Replace OxiSynth as default SF2 backend
- Reference: SpessaSynth (spec compliance), OxiSynth (Rust patterns), FluidSynth (algorithms)

### sfizz VST3 Integration
- setState file loading needs debugging (IBStream works, file not loaded)
- Alternative: explore sfizz's IConnectionPoint messaging protocol
- Goal: load SFZ files programmatically through sfizz VST3

### Parameter Automation in Mixer
- Per-track parameter automation lanes
- Automation events in process loop (sample-accurate parameter changes)

## Non-Goals (v1)

- Plugin delay compensation (PDC)
- Sidechain routing
- Multi-output plugins (>2 channels)
- Insert effects (per-track effect chain) — only send effects
- Undo/redo for mixer state
