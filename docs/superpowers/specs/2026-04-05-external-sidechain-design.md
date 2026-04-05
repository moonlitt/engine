# External Sidechain Routing Design Spec

**Date:** 2026-04-05
**Status:** Draft
**Scope:** External sidechain input for insert effects (compressor, gate, de-esser)

## Motivation

The compressor and gate currently use their own input signal for sidechain detection. In a DAW, the standard workflow is to route another track's audio as the sidechain source — for example, using a kick drum to trigger compression on a bass track. This requires changes across 3 crates: the AudioBackend trait, the mixer's insert chain processing, and the dynamics effects.

## Design Principles

- **Backward compatible** — `set_sidechain()` has a default empty implementation, existing effects unchanged
- **Pre-fader signal** — sidechain source uses engine output + trim, unaffected by source track's fader/mute
- **Dependency-aware rendering** — sidechain dependencies are included in the mixer's topological sort, same mechanism as group routing
- **Cycle detection** — circular sidechain dependencies are rejected, reusing existing cycle detection logic

## Changes by Crate

### moonlitt-core — AudioBackend trait extension

Add two default methods to the AudioBackend trait:

```rust
pub trait AudioBackend: Send {
    // ... existing methods unchanged ...

    /// Provide external sidechain audio for this effect.
    /// Called by the mixer before process_effect() each render cycle.
    /// Effects that support sidechain override this to store the buffers internally.
    /// Default: ignore (use internal sidechain).
    fn set_sidechain(&mut self, _left: &[f32], _right: &[f32]) {}

    /// Whether this effect supports external sidechain input.
    fn supports_sidechain(&self) -> bool { false }
}
```

No existing AudioBackend implementations need changes — the defaults provide the current behavior.

### moonlitt-core — AudioEvent extension

Add a new variant to the AudioEvent enum:

```rust
pub enum AudioEvent {
    // ... existing variants ...

    /// Set external sidechain source for an insert effect.
    /// source_track_id = 0xFF means None (revert to internal sidechain).
    SetInsertSidechain { track_id: u8, insert_id: u8, source_track_id: u8 },
}
```

This variant uses 3 × u8 = 3 bytes + discriminant. AudioEvent remains ≤ 16 bytes. Verify with compile-time assertion.

### moonlitt-mixer — InsertEffect extension

```rust
pub struct InsertEffect {
    pub id: u32,
    pub backend: Box<dyn AudioBackend>,
    pub bypass: bool,
    pub source_path: Option<String>,
    pub sidechain_source: Option<u32>,  // NEW: source track ID, None = internal
}
```

### moonlitt-mixer — Track extension

```rust
pub struct Track {
    // ... existing fields ...
    
    // NEW: temporary buffer for sidechain signal (pre-fader from source track)
    sidechain_buf_l: Vec<f32>,
    sidechain_buf_r: Vec<f32>,
}
```

These buffers are allocated at track creation (same size as other track buffers) and reused each render cycle.

### moonlitt-mixer — New API

```rust
impl Mixer {
    /// Set external sidechain source for an insert effect.
    /// Returns false if this would create a circular dependency.
    pub fn set_insert_sidechain(
        &mut self,
        track_id: u32,
        insert_id: u32,
        source_track_id: Option<u32>,
    ) -> bool {
        // 1. Validate track_id, insert_id exist
        // 2. If source_track_id is Some, validate it exists and != track_id
        // 3. Check for cycles (same logic as group routing cycle detection)
        // 4. Set insert.sidechain_source = source_track_id
        // 5. Recompute render_order
        // 6. Return true on success
    }
}
```

### moonlitt-mixer — Render loop changes

#### Topological sort extension

`compute_render_order()` currently builds a dependency graph from group routing (`output_target`). Extend to also include sidechain dependencies:

```rust
fn compute_render_order(&self) -> Vec<usize> {
    // Build adjacency: for each track, collect dependencies:
    // 1. Group routing: if track routes to Group(id), track depends on nothing extra,
    //    but group track depends on all tracks routed to it (already handled)
    // 2. Sidechain: if track has an insert with sidechain_source = Some(src_id),
    //    then this track depends on src_id (src must render first)
    //
    // Topological sort with cycle detection (existing algorithm)
}
```

#### Insert chain processing

Before calling `process_effect()` on each insert, inject the sidechain signal if configured:

```rust
// In the render loop, for each track:
// 1. Render engine output (existing)
// 2. Apply trim (existing)
// 3. For each insert in chain:
//    a. If insert.sidechain_source is Some(src_id):
//       - Copy source track's pre-fader signal (left/right after engine+trim)
//         into current track's sidechain_buf_l/r
//       - Call insert.backend.set_sidechain(&sidechain_buf_l, &sidechain_buf_r)
//    b. Call insert.backend.process_effect(in_l, in_r, out_l, out_r) (existing)
```

The borrow checker challenge: we need to read from source track's buffers while mutating the current track. Solution: copy source track's pre-fader signal into the sidechain buffer before processing the insert chain. Since the topological sort guarantees the source track has already been rendered, its `left/right` buffers contain valid pre-fader audio.

Implementation approach for the borrow checker:
```rust
// Before processing track's insert chain:
// Collect sidechain data while we can borrow other tracks immutably
for insert in &self.tracks[track_idx].inserts {
    if let Some(src_id) = insert.sidechain_source {
        let src_idx = self.track_index(src_id);
        // Copy source's pre-fader audio to a temporary buffer
        // (This copy happens outside the mutable borrow of the current track)
    }
}
// Then process the insert chain with mutable borrow of current track
```

### moonlitt-mixer — Event dispatch

Add handling for the new `SetInsertSidechain` event in the mixer's event dispatch:

```rust
AudioEvent::SetInsertSidechain { track_id, insert_id, source_track_id } => {
    let source = if source_track_id == 0xFF { None } else { Some(source_track_id as u32) };
    self.set_insert_sidechain(track_id as u32, insert_id as u32, source);
}
```

### moonlitt-effects — Compressor sidechain support

```rust
pub struct Compressor {
    // ... existing fields ...
    
    // NEW: external sidechain storage
    sidechain_ext_l: Vec<f32>,
    sidechain_ext_r: Vec<f32>,
    use_external_sidechain: bool,
}

impl AudioBackend for Compressor {
    fn set_sidechain(&mut self, left: &[f32], right: &[f32]) {
        self.sidechain_ext_l.resize(left.len(), 0.0);
        self.sidechain_ext_r.resize(right.len(), 0.0);
        self.sidechain_ext_l.copy_from_slice(left);
        self.sidechain_ext_r.copy_from_slice(right);
        self.use_external_sidechain = true;
    }

    fn supports_sidechain(&self) -> bool { true }

    fn process_effect(&mut self, in_l: &[f32], in_r: &[f32],
                      out_l: &mut [f32], out_r: &mut [f32]) {
        // Detection path:
        let (det_l, det_r) = if self.use_external_sidechain {
            (self.sidechain_ext_l.as_slice(), self.sidechain_ext_r.as_slice())
        } else {
            (in_l, in_r)
        };
        
        // Per-sample: sidechain HPF applied to det_l/det_r (not in_l/in_r)
        // Gain reduction computed from det signal
        // Gain applied to in_l/in_r (audio path unchanged)
        
        self.use_external_sidechain = false; // Reset each cycle
    }
}
```

### moonlitt-effects — Gate sidechain support

Same pattern as compressor. The gate's sidechain HPF+LPF filters are applied to the external sidechain signal (or internal input if no external source).

### moonlitt-effects — De-esser sidechain support

Optional — lower priority. The de-esser's bandpass detection could also accept an external sidechain, but this is a less common use case.

---

## Session/Runtime API

### moonlitt-session — Session control plane

Add a method to Session for setting sidechain routing:

```rust
impl Session {
    pub fn set_insert_sidechain(
        &self,
        track_id: u8,
        insert_id: u8,
        source_track_id: Option<u8>,
    ) {
        let src = source_track_id.unwrap_or(0xFF);
        let _ = self.producer.push(TimedEvent {
            event: AudioEvent::SetInsertSidechain {
                track_id, insert_id, source_track_id: src,
            },
            delay_samples: 0,
        });
    }
}
```

### moonlitt-audio-io — Runtime

Add corresponding method that delegates to the event producer (same pattern as other mixer control methods).

### moonlitt-node — Node.js binding

```rust
#[napi]
pub fn set_insert_sidechain(&self, track_id: u8, insert_id: u8, 
                             source_track_id: Option<u8>) -> Result<()> {
    // Delegate to runtime
}
```

### moonlitt-capi — C API

```rust
#[no_mangle]
pub extern "C" fn moonlitt_set_insert_sidechain(
    rt: *mut RuntimeHandle,
    track_id: c_int,
    insert_id: c_int,
    source_track_id: c_int,  // -1 = None (internal sidechain)
) -> c_int;
```

---

## Testing Strategy

| Test | Location | Description |
|------|----------|-------------|
| `sidechain_default_is_internal` | moonlitt-effects | Compressor without set_sidechain() behaves identically to current |
| `sidechain_external_triggers_compression` | moonlitt-mixer | Track A (loud signal) sidechains Track B's compressor — B's output is compressed when A is loud |
| `sidechain_cycle_rejected` | moonlitt-mixer | A sidechains B + B sidechains A → set_insert_sidechain returns false |
| `sidechain_pre_fader` | moonlitt-mixer | Source track muted → sidechain signal still flows (pre-fader) |
| `sidechain_none_reverts` | moonlitt-mixer | Setting source to None reverts to internal sidechain |
| `render_order_respects_sidechain` | moonlitt-mixer | Source track always renders before dependent track |
| `gate_external_sidechain` | moonlitt-effects | Gate triggered by external signal instead of own input |
| `audio_event_size_unchanged` | moonlitt-core | AudioEvent still ≤ 16 bytes after adding new variant |

---

## Implementation Order

```
(1) moonlitt-core: add set_sidechain() + supports_sidechain() to AudioBackend trait
(2) moonlitt-core: add SetInsertSidechain to AudioEvent enum
(3) moonlitt-mixer: extend InsertEffect + Track, add sidechain buffers
(4) moonlitt-mixer: extend topological sort for sidechain dependencies
(5) moonlitt-mixer: inject sidechain in render loop + dispatch event
(6) moonlitt-mixer: add set_insert_sidechain() API + cycle detection
(7) moonlitt-effects: compressor set_sidechain() implementation
(8) moonlitt-effects: gate set_sidechain() implementation
(9) moonlitt-session/audio-io/node/capi: API surface for sidechain routing
(10) Tests across all crates
```

Estimated scope: ~400 lines across 6 crates. Moderate complexity due to borrow checker considerations in the render loop.
