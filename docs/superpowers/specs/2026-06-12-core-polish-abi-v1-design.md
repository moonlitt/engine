# Core Polish → ABI v1 Freeze — Design

**Date:** 2026-06-12
**Status:** Approved by user (design dialogue, this session)
**Scope:** 14 engine crates + moonlitt-capi. Game integration explicitly deferred to a later round.

## 1. Background & Goal

Moonlitt is a headless DAW engine (pure Rust; hosts VST3/CLAP plugins and SF2 soundfonts). The long-term vision is embedding into games (Stardew Valley piano-block, Minecraft custom blocks). The user's direction for this round: **polish the game-agnostic core to completion first; game integration comes after.** Required sound sources: SF2, Keyscape (VST3 sample streamer), PianoTeq (VST3 modeled piano).

"Complete" is defined on three dimensions (all required, executed in phases):

1. **Functional**: source × capability matrix tests green.
2. **API freeze**: C ABI v1 finalized — consistent semantics, 100% documented, generated header, conformance testbed.
3. **Engineering quality**: RT-safety clean, CI matrix hardened, stress/long-run coverage.

### Audit grounding (2026-06-12, three parallel audits)

Functional status today:

| Source | Status | Gaps |
|--------|--------|------|
| VST3 PianoTeq | complete | none |
| SF2 oxisynth | complete | no state story (acceptable: presets are addressable) |
| SF2 sampler (pure Rust) | partial | CC/pitch-bend/params are TODOs; voicepool channel tracking; mono_buf allocation |
| VST3 Keyscape | works via state replay | CLI offline render skips `warm_up()` → renders silence (real bug); no automated smoke test story |

C API status today: 91 symbols; no `catch_unwind` at FFI boundary; widespread silent failures (void returns on fallible ops); three coexisting error-reporting patterns; engine-level state save/load not exported (only session-level bundles); `moonlitt_session_save` is a stub that always fails; no generated header (hand-written C# bindings, drift unprotected); no ABI version; runtime API ~53% documented; param value types split f32/f64.

Engineering status today: RT-safety A− (two `eprintln!` in vst3 render error paths; GUI `set_state` can hold the plugin mutex up to ~1 s blocking render); 338 tests with a strong DSP compliance suite; no fuzzing; benches not in CI; CI macOS-only, no fmt/doc/feature-matrix checks; `mixer.rs` at 1861 lines (repo cap is 800); ~1650 uncommitted lines audited as complete and cleanly separable (engine side vs UI side).

## 2. Non-Goals

- Game integration (Stardew mod adaptation, Minecraft/JNI binding) — next round.
- desktop/web/node polish — keep compiling; mechanical sync only when the C API changes.
- New DSP features, Windows CI (Linux CI is in scope; Windows deferred post-v1).
- Backward compatibility of the C ABI — user explicitly waived it; renames are clean breaks with no aliases. The Stardew mod re-adapts in the game round. (Session file format keeps its existing `#[serde(default)]` v2 compatibility policy — cheap and already in place.)

In-repo consumers `bindings/dotnet` and `examples/ffi-testbed-csharp` ARE updated in the same PR as any API change — the testbed is the ABI gatekeeper.

## 3. Phase 0 — Land in-flight work + stop-the-bleed fix

1. Commit the ~1650 uncommitted lines as four separate commits (full test run first):
   1. `feat(session)`: metronome (new `metronome.rs`, processor integration, persistence v2 fields `metronome_enabled`/`color`, audio-io atomic toggle)
   2. `feat(vst3)`: shared plugin handle (`Arc<parking_lot::Mutex<Vst3Plugin>>`) + `tests/vst3_shared_handle.rs`
   3. `feat(desktop,web)`: Tier-1 DAW UI (meters, master section, send-bus rack, metronome toggle, plugin window rewrite)
   4. `feat(examples,ci)`: FFI testbed (C#, 63 checks) + bevy-piano-tiles demo + CI wiring (`--no-sf2` job)
2. Fix the Keyscape offline-render bug: `cmd_midi_render` in `moonlitt-cli` must call `backend.warm_up()` after `load_state()` (mirror `persistence.rs` restore path). Add a regression test (fixture-gated, skips without Keyscape).

**Deliverable:** clean working tree, CI green including the FFI testbed job.

## 4. Phase 1 — API skeleton (conventions across all 91 symbols)

Do conventions before adding functions, so Phase 2 additions land conformant once.

### 4.1 Error model

- All fallible functions return `MoonlittStatus` (`int32_t`): `0` = OK; negative = error class:
  `-1 INVALID_ARG, -2 NOT_LOADED, -3 QUEUE_FULL, -4 IO, -5 PLUGIN, -6 STATE, -7 PANIC, -8 UNSUPPORTED`.
- Human-readable detail: single thread-local accessor `moonlitt_last_error_message() → const char*` (borrowed; valid on the calling thread until the next moonlitt call). This replaces both the per-handle `moonlitt_engine_get_error` and the session thread-local accessor — one pattern everywhere, and it works for creation functions that have no handle yet.
  - *Alternative considered:* per-handle error storage. Rejected: thread-local is simpler to bind via P/Invoke, covers handle-less creation failures, and is correct under the documented single-control-thread contract.
- Hot-path producers (`note_on` etc., called from a game/control thread, not the audio thread) return cheap status ints and set only **static** message strings (no formatting) on failure, e.g. `QUEUE_FULL`.
- `void` return is reserved for genuinely infallible operations (`destroy`, `free_*`).
- Silent failures are abolished: queue overflow → `QUEUE_FULL`; bad track/insert/bus id → `INVALID_ARG`.

### 4.2 Panic safety

- `ffi_guard!` macro wraps every `extern "C"` body: `catch_unwind` → set thread-local message → return `MOONLITT_ERR_PANIC` (or null/sentinel for pointer/value returns).
- Debug-only `moonlitt_test_panic()` export so the testbed can prove the guard works end-to-end through P/Invoke.

### 4.3 ABI version

- `moonlitt_abi_version() → uint32_t` packed `(major<<16)|(minor<<8)|patch`, plus the same constant in the generated header. Loaders check at startup.

### 4.4 Generated header

- cbindgen → `include/moonlitt.h`, committed. CI regenerates and fails on drift (`git diff --exit-code`).
- C# bindings annotated to correspond 1:1 with header sections; testbed gains a symbol-presence check against the dylib export table.

### 4.5 Type & naming unification

- Parameter values are `double` (f64) end-to-end (runtime `set_param` migrates from f32; the SPSC event stays Copy and small — verify size budget in implementation).
- Naming rule: **family prefix == handle type**. Functions taking `RuntimeHandle` are `moonlitt_runtime_*`; pre-build `MixerHandle` functions are `moonlitt_mixer_*`. Known violations to rename (clean break, exact sweep done at implementation):
  `moonlitt_set_param_for_track`, `moonlitt_set_insert_param`, `moonlitt_set_insert_sidechain`, and the `moonlitt_mixer_set_*` family that actually takes a RuntimeHandle.
  Stream-vs-transport split: audio stream control becomes `moonlitt_runtime_start_audio`/`stop_audio`; transport becomes `moonlitt_runtime_play`/`pause`/`stop`.

### 4.6 Ownership rules

- Every `create`/`destroy` pair documented; every returned string/buffer marked **owned** (must `moonlitt_free_string`/`moonlitt_free_buffer`) or **borrowed** (must not free).
- `add_track` consuming an engine's backend becomes explicit: second consumption attempt returns `NOT_LOADED` with a clear message instead of UB; documented as ownership transfer.

### 4.7 Documentation

- 100% of exported symbols get `///` docs: parameter semantics, units, valid ranges, threading contract, ownership. `cargo doc` joins CI (Phase 3 enforces `-D warnings`).

**Deliverable:** ABI draft v0.9 — header + conventions doc; all symbols conformant; testbed + dotnet bindings updated in lockstep.

## 5. Phase 2 — Functional completion (under the new conventions)

1. **Engine-level state API** (the key to the Keyscape single-patch workflow):
   ```c
   MoonlittStatus moonlitt_engine_save_state(EngineHandle*, uint8_t** out_data, uintptr_t* out_len);
   MoonlittStatus moonlitt_engine_load_state(EngineHandle*, const uint8_t* data, uintptr_t len);
   void           moonlitt_free_buffer(uint8_t* data, uintptr_t len);
   int32_t        moonlitt_engine_recommended_warmup_blocks(EngineHandle*);
   MoonlittStatus moonlitt_engine_warm_up(EngineHandle*, int32_t blocks);
   ```
   Binary-safe (raw bytes + length, no base64). Wraps the existing `AudioBackend::save_state`/`load_state`/`warm_up` and the MLST container.
2. **Real `moonlitt_session_save`**: control thread briefly locks each track's shared plugin handle (the Phase-0 `Arc<Mutex<Vst3Plugin>>` architecture enables exactly this), captures backend states, merges with a control-side shadow of mixer state kept on the RuntimeHandle (extended as needed — every mixer mutation already flows through the C API, so the shadow is maintainable without audio-thread reads), writes v2 JSON. Lock windows are bounded; render-side `try_lock` fallback arrives in Phase 3.
3. **Sampler parity**: CC7 volume, CC11 expression, **CC64 sustain**, pitch bend; voicepool channel tracking fix; pre-allocate `mono_buf` (removes an RT allocation hazard).
4. **Keyscape workflow closure**: MLST container versioning; fixture-driven smoke test (auto-skip without fixture); a `docs/` guide for "capture patch in desktop app → replay headless".
5. **Runtime queries**: `moonlitt_runtime_is_running`, master peak/RMS atomic reads (mixer metering already exists; expose it).
6. **Deeper `moonlitt_session_validate_file`**: verify referenced plugin/SF2 paths exist + format sniff, so validate-then-load can't diverge.

**Deliverable:** functional matrix complete for all sources; testbed checks for every new symbol.

## 6. Phase 3 — Engineering hardening

1. **RT zero-defect**: vst3 `render()` switches to `try_lock` + silence fill on contention; the two `eprintln!` calls become an atomic error counter + a control-thread-readable last-error slot.
2. **CI matrix**: `cargo fmt --check`; `cargo doc --no-deps -D warnings`; feature-combination builds (`--no-default-features`, `sf2-sampler`, `vst3`, `clap`, all-features); Linux job (SF2/CLAP path; audio-device tests already skip gracefully); ABI header drift check; existing macOS job + FFI testbed retained.
3. **Test hardening**: source × capability matrix suite (`moonlitt-test-suite/tests/source_matrix.rs`) with tiered gating (CI subset ungated; PianoTeq/Keyscape/fixture/device tests skip with a printed coverage manifest); 10-minute long-run render with an allocation-counter assertion on the audio path; SF2-loader fuzz target (cargo-fuzz, run locally, not in CI); testbed extended to error paths + panic-guard verification.
4. **mixer.rs split**: 1861 lines → focused modules (tracks / sends / metering / routing), each ≤800 lines; purely mechanical, tests unchanged.

## 7. Phase 4 — v1 freeze

- Final naming/semantics review pass over the full surface.
- CHANGELOG; tag `moonlitt-capi` v1.0; publish `include/moonlitt.h`.
- `docs/`: embedding guide + Keyscape headless workflow guide.
- FFI testbed pinned as the ABI conformance suite.

## 8. Acceptance criteria (Definition of Done)

**Functional** — matrix green: {SF2-oxisynth, SF2-sampler, VST3-PianoTeq*, VST3-Keyscape*} × {load, preset/program, params, state save/load, realtime*, offline render, session roundtrip, warmup where applicable}. (*) = gated by local plugin/fixture/device availability; CI runs the ungated subset; the suite prints what was skipped so coverage is never silently overstated.

**API** — every exported symbol: status-code conformant, panic-guarded, documented, present in the generated header, covered by ≥1 testbed check. ABI version exported. Drift check green. `moonlitt_session_save` actually works.

**Engineering** — audio path verified alloc/lock/IO/format-free (review checklist + long-run allocation assertion); CI matrix green on macOS + Linux incl. feature combos; mixer.rs split done; fuzz target runs clean locally (one-time ≥1M execs).

## 9. Risks & mitigations

- **session_save consistency** (mixer lives on the audio thread): solved via shared plugin handles + control-side shadow state; lock windows bounded; Phase 3's `try_lock` render fallback turns worst-case contention into momentary silence instead of a stall.
- **Off-thread `getState` plugin quirks**: Spectrasonics can take ~1 s; bounded and documented; warmup advisory exposed via API.
- **Linux CI flakiness**: no audio device in runners — existing graceful-skip machinery covers it; Linux job scoped to SF2/CLAP.
- **Scope**: mixer split and fuzzing are bounded, mechanical items and are part of the Definition of Done (§8). If schedule pressure appears, descoping either one is a user decision surfaced explicitly — never a silent slip.

## 10. Decisions log

- Error detail via **thread-local accessor**, not per-handle storage (binding simplicity; covers creation failures; matches documented threading model).
- State blobs as **raw bytes + length**, not base64 (binary-safe, zero-copy on the C side; base64 stays an internal session-file encoding detail).
- **No ABI aliases** for renames — user waived backward compatibility; in-repo consumers updated atomically; out-of-repo consumers (Stardew mod) adapt in the game round.
- Param values standardize on **f64**.
- Windows CI deferred post-v1; Linux included now.
