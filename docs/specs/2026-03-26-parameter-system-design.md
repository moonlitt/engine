# Moonlitt Parameter System Design

## Problem

Each audio backend (SF2, VST3, CLAP) exposes different parameters:
- **SF2/FluidLite**: reverb (roomsize, damp, width, level), chorus (nr, level, speed, depth), gain
- **VST3**: plugin-defined via IEditController (Pianoteq has 200+ params)
- **CLAP**: plugin-defined via clap_plugin_params extension

Currently moonlitt has no unified parameter API. The host/UI/FFI cannot discover or control backend parameters.

## Design

### Core Abstraction

Add parameter discovery and control to the `AudioBackend` trait:

```rust
/// Describes a single controllable parameter.
pub struct ParamInfo {
    pub id: u32,           // unique within this backend instance
    pub name: String,      // display name: "Reverb Room Size"
    pub group: String,     // UI grouping: "Reverb", "Chorus", "Dynamics"
    pub min: f64,          // minimum value (e.g., 0.0)
    pub max: f64,          // maximum value (e.g., 1.0)
    pub default: f64,      // default value
    pub step_count: u32,   // 0 = continuous, >0 = discrete steps
    pub flags: ParamFlags, // hidden, readonly, automatable
}

bitflags! {
    pub struct ParamFlags: u32 {
        const HIDDEN    = 1 << 0;  // don't show in UI
        const READONLY  = 1 << 1;  // display only
        const STEPPED   = 1 << 2;  // discrete values (e.g., on/off)
    }
}

// Added to AudioBackend trait:
fn param_count(&self) -> u32 { 0 }
fn param_info(&self, index: u32) -> Option<ParamInfo> { None }
fn get_param(&self, id: u32) -> Option<f64> { None }
fn set_param(&mut self, id: u32, value: f64) { }
fn param_display(&self, id: u32, value: f64) -> Option<String> { None }
```

All methods have default implementations (return nothing / do nothing), so existing backends compile without changes. Backends opt in by implementing the methods they support.

### SF2 Backend Parameters

Hand-defined parameter set, mapped to FluidLite API calls:

| ID | Name | Group | Min | Max | Default | FluidLite API |
|----|------|-------|-----|-----|---------|---------------|
| 0 | Reverb On | Reverb | 0 | 1 | 1 | `set_reverb_on` |
| 1 | Room Size | Reverb | 0 | 1.2 | 0.2 | `set_reverb_params` roomsize |
| 2 | Damping | Reverb | 0 | 1 | 0.0 | `set_reverb_params` damp |
| 3 | Width | Reverb | 0 | 100 | 0.5 | `set_reverb_params` width |
| 4 | Level | Reverb | 0 | 1 | 0.9 | `set_reverb_params` level |
| 10 | Chorus On | Chorus | 0 | 1 | 1 | `set_chorus_on` |
| 11 | Voices | Chorus | 0 | 99 | 3 | `set_chorus_params` nr |
| 12 | Level | Chorus | 0 | 10 | 2 | `set_chorus_params` level |
| 13 | Speed | Chorus | 0.1 | 5 | 0.3 | `set_chorus_params` speed |
| 14 | Depth | Chorus | 0 | 256 | 8 | `set_chorus_params` depth |
| 20 | Gain | Master | 0 | 5 | 1 | `set_gain` |

### VST3 Backend Parameters

Direct mapping from IEditController:

```rust
impl AudioBackend for Vst3Backend {
    fn param_count(&self) -> u32 {
        // ctrl.getParameterCount()
    }
    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        // ctrl.getParameterInfo(index) → convert ParameterInfo to ParamInfo
        // name: String128 → String
        // group: unitInfo or empty
        // min/max: 0.0/1.0 (VST3 uses normalized values)
        // step_count: from ParameterInfo.stepCount
        // flags: map kIsReadOnly, kIsHidden, etc.
    }
    fn get_param(&self, id: u32) -> Option<f64> {
        // ctrl.getParamNormalized(id)
    }
    fn set_param(&mut self, id: u32, value: f64) {
        // ctrl.setParamNormalized(id, value)
        // For real-time safety: queue parameter change as process event
    }
    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        // ctrl.getParamStringByValue(id, value)
        // Returns "Hall" or "0.5 ms" etc.
    }
}
```

### CLAP Backend Parameters

Direct mapping from clap_plugin_params extension:

```rust
impl AudioBackend for ClapBackend {
    fn param_count(&self) -> u32 {
        // params_ext.count(plugin)
    }
    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        // params_ext.get_info(plugin, index)
        // name: from clap_param_info.name
        // group: from clap_param_info.module (path separator '/')
        // min/max: clap_param_info.min_value / max_value (NOT normalized)
        // step_count: inferred from CLAP_PARAM_IS_STEPPED flag
    }
    fn get_param(&self, id: u32) -> Option<f64> {
        // params_ext.get_value(plugin, id)
    }
    fn set_param(&mut self, id: u32, value: f64) {
        // Queue CLAP_EVENT_PARAM_VALUE in next process call
    }
    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        // params_ext.value_to_text(plugin, id, value)
    }
}
```

### Engine Layer

Engine passes through to the active backend:

```rust
impl Engine {
    pub fn param_count(&self) -> u32 { ... }
    pub fn param_info(&self, index: u32) -> Option<ParamInfo> { ... }
    pub fn get_param(&self, id: u32) -> Option<f64> { ... }
    pub fn set_param(&mut self, id: u32, value: f64) { ... }
    pub fn param_display(&self, id: u32, value: f64) -> Option<String> { ... }
}
```

### Runtime Layer

Parameters need to reach the audio thread. Two approaches:

- **set_param**: Send via ring buffer as `AudioEvent::SetParam { id, value }` — audio thread applies on next callback. Real-time safe.
- **get_param / param_info**: Read-only, can be called directly since backend state is immutable for reads. But Engine is owned by AudioThread...

Solution: `param_count`, `param_info`, and `param_display` are static after load — cache them at load time. `get_param` sends a request and gets a response (or we cache last-set values on the caller side).

Simpler approach for v1: cache ParamInfo list at load time. `set_param` goes through ring buffer. `get_param` returns the last value set by the caller (shadow state). This avoids cross-thread reads entirely.

```rust
pub struct Runtime {
    // ...
    param_cache: Vec<ParamInfo>,        // cached at load time
    param_values: HashMap<u32, f64>,    // shadow state for get_param
}

impl Runtime {
    pub fn param_count(&self) -> u32 { self.param_cache.len() as u32 }
    pub fn param_info(&self, index: u32) -> Option<&ParamInfo> { ... }
    pub fn set_param(&mut self, id: u32, value: f64) {
        self.param_values.insert(id, value);
        self.send(AudioEvent::SetParam { id, value });
    }
    pub fn get_param(&self, id: u32) -> Option<f64> {
        self.param_values.get(&id).copied()
    }
}
```

### FFI Layer

```c
// Discovery
int moonlitt_engine_param_count(EngineHandle* e);
char* moonlitt_engine_param_info_json(EngineHandle* e);
// Returns: [{"id":0,"name":"Room Size","group":"Reverb","min":0,"max":1.2,"default":0.2,"step_count":0,"flags":0}, ...]

// Get/Set
double moonlitt_engine_get_param(EngineHandle* e, int id);
void moonlitt_engine_set_param(EngineHandle* e, int id, double value);
char* moonlitt_engine_param_display(EngineHandle* e, int id, double value);

// Runtime versions (thread-safe via ring buffer)
int moonlitt_runtime_param_count(RuntimeHandle* rt);
char* moonlitt_runtime_param_info_json(RuntimeHandle* rt);
void moonlitt_runtime_set_param(RuntimeHandle* rt, int id, double value);
double moonlitt_runtime_get_param(RuntimeHandle* rt, int id);
```

### AudioEvent Extension

```rust
pub enum AudioEvent {
    // ... existing variants ...
    SetParam { id: u32, value: f64 },  // NEW: 12 bytes
}
```

Note: AudioEvent must stay small for rtrb efficiency. SetParam adds one variant with 12 bytes (u32 + f64). TimedEvent remains Copy. Total TimedEvent size ~20 bytes, acceptable.

### C# Bindings (Piano Block mod)

```csharp
// NativeEngine
public int ParamCount();
public string? GetParamInfoJson();
public double GetParam(int id);
public void SetParam(int id, double value);
public string? GetParamDisplay(int id, double value);
```

### Value Conventions

| Backend | Value Range | Normalization |
|---------|-------------|---------------|
| SF2 | Physical units (Hz, seconds, etc.) | None — values are direct |
| VST3 | 0.0 - 1.0 (normalized) | Plugin handles conversion |
| CLAP | Physical units (min_value - max_value) | None — values are direct |

The UI normalizes for display (slider position = (value - min) / (max - min)).

## Implementation Order

1. **ParamInfo struct + AudioBackend trait** — add to moonlitt-engine
2. **SF2 backend** — implement the 11 hand-defined parameters
3. **VST3 backend** — map IEditController parameter API
4. **CLAP backend** — map clap_plugin_params extension
5. **AudioEvent::SetParam** — add variant, handle in audio_thread
6. **Engine passthrough** — Engine delegates to backend
7. **Runtime caching** — param_cache + shadow values
8. **FFI** — expose all functions
9. **CLI** — `moonlitt params <plugin>` command
10. **C# bindings** — NativeEngine methods

## Testing

- SF2: verify reverb/chorus params change audibly (WAV comparison)
- VST3: load Pianoteq, enumerate params, set/get round-trip
- CLAP: same with available CLAP plugins
- FFI: JSON roundtrip, null safety
- Edge cases: set_param on invalid ID, get_param before load

## Non-Goals (v1)

- Parameter automation (sample-accurate param changes) — future via TimedEvent
- Parameter state save/restore (already in AudioBackend as save_state/load_state)
- GUI rendering of plugin's native editor window
- MIDI learn / parameter mapping
