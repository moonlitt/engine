# Changelog

## moonlitt-capi ABI 1.0.0 — 2026-06-12

The first frozen C ABI. `libmoonlitt.{dylib,so,dll}` + `include/moonlitt.h`
(cbindgen-generated, CI drift-checked). 101 symbols, every one
status-code conformant, panic-guarded, documented, and covered by the
FFI conformance testbed (122 full-mode checks, 0 uncovered symbols).

### The contract

- Fallible functions return `MoonlittStatus` (0 = OK, negative = error
  class); detail via thread-local `moonlitt_last_error_message()`.
  No silent failures: queue overflow → `QUEUE_FULL`, out-of-range
  arguments rejected (never clamped), consumed handles → `NOT_LOADED`.
- Every `extern "C"` body is panic-guarded — a Rust panic becomes
  `MOONLITT_ERR_PANIC`, never an unwind into the host process
  (`moonlitt_debug_trigger_panic()` lets bindings verify this).
- Family prefix == handle type; backend parameter values are f64
  end-to-end; ownership (owned vs borrowed returns, backend-consuming
  transfers) documented per symbol.
- `moonlitt_abi_version()` → `(1<<16)|(0<<8)|0`. Additions bump MINOR,
  breaks bump MAJOR.

### Highlights since the pre-ABI surface

- **Patch state API**: `moonlitt_engine_save_state` / `load_state` /
  `supports_state` / `warm_up` / `recommended_warm_up_blocks` — the
  capture-once-replay-forever workflow for commercial samplers
  (Keyscape verified end-to-end through the C ABI).
- **Session save from a live runtime**: `moonlitt_runtime_save_session`
  (control-side shadow + shared-handle state capture); deep
  `moonlitt_session_validate_file` (referenced files must exist).
- **Runtime queries**: `is_running`, `master_peak`, `master_rms`
  (atomic reads).
- **SF2 sampler expressiveness parity**: per-channel CC7/CC11/CC64
  sustain + pitch bend, channel-tracked voices, allocation-free render.
- **RT hardening**: the audio path never blocks on the GUI's plugin
  lock (silence / pass-through fallback + miss counters, replacing
  audio-thread `eprintln!`); the from-scratch render path measures
  **zero** steady-state heap allocations over a soak test.
- **Engine fixes**: offline render warm-up for sample streamers (was
  rendering silent files), f64→f32 parameter truncation in node and
  desktop bindings, a load-dependent scan-cache test race.

### Renames (clean breaks; no aliases — pre-1.0 consumers re-bind)

- per-handle `moonlitt_engine_get_error` and the session-local accessor
  → one `moonlitt_last_error_message()`
- `moonlitt_runtime_start/stop` → `start_audio`/`stop_audio`;
  transport `stop_playback` → `moonlitt_runtime_stop`
- `moonlitt_mixer_set_*` taking a RuntimeHandle →
  `moonlitt_runtime_set_*`; `moonlitt_set_param_for_track` →
  `moonlitt_runtime_set_track_param` (and siblings)
- `moonlitt_session_load_file` → `moonlitt_session_read_json`
- `moonlitt_multitrack_create` → `moonlitt_runtime_create_multitrack_sf2`
- `moonlitt_engine_recommended_warmup_blocks` →
  `…_recommended_warm_up_blocks`
- cdylib artifact renamed **`libmoonlitt`** (was `libmoonlitt_capi`)

### Docs

- `docs/embedding-guide.md` — integrating the engine into a game mod
- `docs/keyscape-headless-workflow.md` — commercial-sampler patches
  without a GUI
