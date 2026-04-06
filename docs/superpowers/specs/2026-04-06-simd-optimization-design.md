# SIMD Optimization Design Spec

**Date:** 2026-04-06
**Status:** Draft
**Scope:** SIMD optimization of 4 DSP hot paths + benchmark suite

## Motivation

moonlitt has zero SIMD acceleration — all DSP hot paths use scalar Rust. Profiling identifies 4 critical bottlenecks: dB→linear conversion (`powf` at ~100 cycles/sample), FIR convolution in oversampler (25-tap inner product), mixer buffer operations (fill/accumulate/gain on every render), and sinc interpolation (8-tap inner product in delay/chorus/flanger). Combined realistic speedup: 2-3× on the audio thread.

## Design Principles

- **Zero audio quality change** — SIMD computes the same operations, just 4 at a time. All 356 existing tests must pass unchanged.
- **`wide` crate** — stable Rust, cross-platform (x86 SSE + ARM NEON), safe API, no `unsafe` needed.
- **Benchmark before and after** — establish baseline with criterion, measure improvement.
- **Non-invasive** — SIMD functions are helpers called from existing code; no architectural changes.

## Dependencies

```toml
# moonlitt-effects/Cargo.toml — add
wide = "0.7"

# moonlitt-mixer/Cargo.toml — add
wide = "0.7"
moonlitt-effects = { path = "../moonlitt-effects" }  # for db_lut

# New crate: moonlitt-bench/Cargo.toml
criterion = { version = "0.5", features = ["html_reports"] }
```

---

## P0: dB→Linear Lookup Table (db_lut.rs)

**Current bottleneck:** `10.0_f64.powf(gain_db / 20.0)` — ~100 CPU cycles per sample. Called in every compressor, limiter, gate, and mixer trim operation.

### Implementation

Create `crates/moonlitt-effects/src/common/db_lut.rs`:

```rust
pub struct DbLut {
    table: Vec<f32>,   // 4096 entries
    db_min: f32,       // -120.0
    db_max: f32,       // +24.0
    inv_step: f32,     // entries / db_range
}

impl DbLut {
    pub fn new() -> Self {
        // Pre-compute: table[i] = 10^((db_min + i*step) / 20)
        // 4096 entries over 144dB range = 0.035 dB/entry
    }

    /// O(1) lookup with linear interpolation. ~5 cycles.
    #[inline]
    pub fn db_to_linear(&self, db: f32) -> f32;
}
```

**Precision:** 4096 entries over 144dB = 0.035 dB resolution. Linear interpolation between entries gives < 0.001 dB error — far below 24-bit audio noise floor.

**Integration:** Each effect that uses dB→linear creates a `DbLut` in its constructor (16KB, one-time allocation). Replace all `10.0_f64.powf(x / 20.0)` calls with `self.db_lut.db_to_linear(x as f32) as f64`.

**Consumers:**
- `dynamics/compressor.rs` — per-sample gain conversion
- `dynamics/limiter.rs` — per-sample gain conversion
- `dynamics/gate.rs` — range_db to linear
- `dynamics/multiband_compressor.rs` — per-band gain conversion
- `mixer/mixer.rs` — trim_db to linear

---

## P1: Oversampler FIR SIMD

**Current:** Scalar 25-tap FIR convolution in `filter_output()`.

### Implementation

In `crates/moonlitt-effects/src/common/oversampler.rs`, replace the inner product loop:

```rust
use wide::f32x4;

// 25 taps → 6 chunks of 4 + 1 scalar
let mut sum = f32x4::ZERO;
for chunk in 0..6 {
    let taps = f32x4::new(self.taps[chunk*4..chunk*4+4].try_into().unwrap());
    let samples = f32x4::new(self.delay[base+chunk*4..base+chunk*4+4].try_into().unwrap());
    sum = sum + taps * samples;
}
let result = sum.reduce_add() + self.delay[base + 24] * self.taps[24];
```

**Buffer padding:** Delay line needs 25 extra samples at the end to guarantee contiguous reads without modular arithmetic. When write pointer wraps, copy the first 25 samples to the padding region.

---

## P2: Mixer Buffer Operations SIMD

**Current:** Scalar loops for buffer zeroing, gain application, and accumulation.

### Implementation

Create `crates/moonlitt-effects/src/common/simd.rs`:

```rust
use wide::f32x4;

/// Multiply every sample in buf by gain.
#[inline]
pub fn apply_gain(buf: &mut [f32], gain: f32) {
    let gain_v = f32x4::splat(gain);
    let chunks = buf.chunks_exact_mut(4);
    let remainder = chunks.into_remainder();
    for chunk in chunks {
        let v = f32x4::new(chunk.try_into().unwrap());
        (v * gain_v).to_array().iter().enumerate().for_each(|(i, &val)| chunk[i] = val);
    }
    for s in remainder {
        *s *= gain;
    }
}

/// Add src into dst element-wise.
#[inline]
pub fn accumulate(dst: &mut [f32], src: &[f32]) {
    let len = dst.len().min(src.len());
    let chunks = len / 4;
    for i in 0..chunks {
        let d = f32x4::new(dst[i*4..i*4+4].try_into().unwrap());
        let s = f32x4::new(src[i*4..i*4+4].try_into().unwrap());
        let r = d + s;
        dst[i*4..i*4+4].copy_from_slice(&r.to_array());
    }
    for i in chunks*4..len {
        dst[i] += src[i];
    }
}
```

**Integration in mixer.rs:**
- Replace `for s in &mut left[..frames] { *s *= trim_gain; }` with `simd::apply_gain(&mut left[..frames], trim_gain)`
- Replace manual accumulation loops with `simd::accumulate(&mut master_l, &track_l)`

---

## P3: Sinc Interpolation SIMD

**Current:** 8-tap scalar inner product in `FractionalDelayLine::read()`.

### Implementation

In `delay_line.rs`, add a SIMD fast path:

```rust
use wide::f32x4;

// 8 taps = 2 × f32x4
let k0 = f32x4::new(kernel[0..4].try_into().unwrap());
let k1 = f32x4::new(kernel[4..8].try_into().unwrap());
let s0 = f32x4::new(self.buffer[base..base+4].try_into().unwrap());
let s1 = f32x4::new(self.buffer[base+4..base+8].try_into().unwrap());
(k0 * s0 + k1 * s1).reduce_add()
```

**Buffer padding:** Same approach as oversampler — 8 extra samples at buffer end for contiguous reads.

---

## Benchmark Suite (moonlitt-bench)

New workspace member: `crates/moonlitt-bench/`

### Structure

```
crates/moonlitt-bench/
├── Cargo.toml
└── benches/
    ├── mixer_bench.rs
    ├── effects_bench.rs
    └── resampler_bench.rs
```

Not added to workspace `members` (benchmark crates are typically run manually). Added to workspace `exclude` to prevent `cargo test --workspace` from building it.

### Benchmarks

**mixer_bench.rs:**
- `mixer_render_4tracks_512` — 4 tracks, 512 samples
- `mixer_render_16tracks_512` — 16 tracks with 2 inserts each
- `mixer_accumulate_512` — isolated buffer accumulation
- `mixer_apply_gain_512` — isolated gain application

**effects_bench.rs:**
- `compressor_process_512` — compressor on 512 samples
- `limiter_2x_512` — limiter with 2× oversampling
- `oversampler_2x_512` — oversampler alone
- `chorus_4voice_512` — 4-voice chorus
- `db_to_linear_powf_1000` — 1000× `powf` (baseline)
- `db_to_linear_lut_1000` — 1000× LUT lookup (optimized)

**resampler_bench.rs:**
- `sinc8_read_1000` — 1000 fractional delay reads
- `sinc8_read_simd_1000` — SIMD version

### Usage

```bash
cargo bench -p moonlitt-bench
```

Generates HTML reports in `target/criterion/`.

---

## Testing Strategy

### Correctness (most important)

All 356 existing tests (179 compliance + 177 unit) must pass unchanged after SIMD optimization. This guarantees zero audio quality regression.

### SIMD-specific tests

Add to `moonlitt-effects` unit tests:

| Test | Verifies |
|------|----------|
| `db_lut_precision` | LUT error < 0.001 dB vs `powf` across -120 to +24 dB |
| `db_lut_boundary` | -120 dB → ~0.0, +24 dB → ~15.85, 0 dB → 1.0 |
| `simd_apply_gain_matches_scalar` | SIMD result == scalar result for various gains |
| `simd_accumulate_matches_scalar` | SIMD result == scalar result |
| `simd_inner_product_matches_scalar` | SIMD FIR == scalar FIR |
| `simd_sinc_matches_scalar` | SIMD sinc read == scalar sinc read |

---

## Implementation Order

```
(1) Create moonlitt-bench crate + baseline benchmarks (measure BEFORE)
(2) common/db_lut.rs + integrate into compressor/limiter/gate/multiband/mixer
(3) common/simd.rs (apply_gain, accumulate) + integrate into mixer
(4) Oversampler FIR SIMD (modify oversampler.rs)
(5) Delay line sinc SIMD (modify delay_line.rs)
(6) Final benchmarks (measure AFTER) + comparison report
```

Each step must pass `cargo test --workspace -- --skip pianoteq --skip keyscape`.
