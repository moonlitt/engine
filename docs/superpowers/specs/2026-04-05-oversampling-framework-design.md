# Oversampling Framework Design Spec

**Date:** 2026-04-05
**Status:** Draft
**Scope:** Shared oversampling infrastructure for moonlitt-effects + limiter integration

## Motivation

The limiter's `oversampling` parameter is currently a placeholder — it's stored but not used in DSP processing. True peak limiting requires oversampling to detect inter-sample peaks that exceed the ceiling. Future effects (saturator, bitcrusher, distortion) also need oversampling for anti-aliasing. A shared framework avoids duplicating this complex DSP infrastructure.

## Design Principles

- **Shared utility** — one `Oversampler` struct in `common/`, reusable by any effect
- **Half-band FIR** — specialized 2x filter, ~50% fewer multiplies than general sinc
- **Linear phase** — symmetric FIR, zero phase distortion, latency reported via `latency()` for PDC
- **Cascaded architecture** — 4x = two 2x stages, 8x = three 2x stages

## Architecture

### File Structure

```
crates/moonlitt-effects/src/common/
├── mod.rs              — add pub mod oversampler;
├── param_smoother.rs   — existing
├── denormal.rs         — existing
└── oversampler.rs      — NEW: Oversampler + HalfBandStage
```

No new crate dependencies. Pure Rust FIR implementation.

### Oversampler API

```rust
/// Shared oversampling processor.
/// Handles upsample → process → downsample with linear-phase half-band filters.
pub struct Oversampler {
    factor: usize,              // 1, 2, 4, or 8
    stages: Vec<HalfBandStage>, // one per 2x level (0 for 1x, 1 for 2x, 2 for 4x, 3 for 8x)
    work_buffers: Vec<Vec<f32>>, // intermediate buffers for cascade
}

impl Oversampler {
    /// Create an oversampler.
    /// - factor: 1 (bypass), 2, 4, or 8
    /// - max_block_size: maximum input block size in samples
    pub fn new(factor: usize, max_block_size: usize) -> Self;

    /// Process a block: upsample → callback at high rate → downsample.
    ///
    /// The callback receives a mutable slice at the oversampled rate
    /// (length = input.len() * factor). It should process the audio in-place.
    ///
    /// Output length == input length.
    pub fn process<F>(&mut self, input: &[f32], output: &mut [f32], callback: F)
    where
        F: FnMut(&mut [f32]);

    /// Latency in samples at the original sample rate.
    /// Each 2x stage adds order/4 samples of latency.
    pub fn latency(&self) -> usize;

    /// Reset all filter states (call on parameter changes that reset processing).
    pub fn reset(&mut self);

    /// Current oversampling factor.
    pub fn factor(&self) -> usize;
}
```

### Usage Pattern (Effect Integration)

```rust
// In an effect's process_effect():
if self.oversampling > 1 {
    // Process left channel
    self.oversampler_l.process(in_l, out_l, |upsampled| {
        for sample in upsampled.iter_mut() {
            *sample = self.process_sample(*sample);
        }
    });
    // Process right channel
    self.oversampler_r.process(in_r, out_r, |upsampled| {
        for sample in upsampled.iter_mut() {
            *sample = self.process_sample(*sample);
        }
    });
} else {
    // Direct processing at original rate
    for i in 0..in_l.len() {
        out_l[i] = self.process_sample(in_l[i]);
        out_r[i] = self.process_sample(in_r[i]);
    }
}
```

### HalfBandStage — Single 2x Up/Down

```rust
/// A single 2x oversampling stage using a linear-phase half-band FIR filter.
struct HalfBandStage {
    coeffs: Vec<f32>,          // non-zero half-band coefficients (symmetric, store half)
    up_delay: Vec<f32>,        // upsampling filter delay line
    down_delay: Vec<f32>,      // downsampling filter delay line
    order: usize,              // filter order (number of taps)
}

impl HalfBandStage {
    /// Create a stage with the given filter order.
    fn new(order: usize) -> Self;

    /// Upsample: insert zeros between samples, apply lowpass.
    /// Output length = input length × 2.
    fn upsample(&mut self, input: &[f32], output: &mut [f32]);

    /// Downsample: apply lowpass, decimate by 2.
    /// Output length = input length / 2.
    fn downsample(&mut self, input: &[f32], output: &mut [f32]);

    /// Reset delay lines.
    fn reset(&mut self);

    /// Latency in upsampled samples.
    fn latency_samples(&self) -> usize;
}
```

### Half-Band Filter Design

**Order:** 12 (12-tap FIR, 6 non-zero coefficients + center tap)

Half-band filters have a special property: every other coefficient (except the center) is exactly zero. For a 12th-order half-band:
```
coefficients: [c0, 0, c2, 0, c4, 0, 0.5, 0, c4, 0, c2, 0, c0]
                                       ^center
```

Only c0, c2, c4 (and their symmetric mirrors) + center tap 0.5 need computation. This halves the multiply count compared to a general FIR.

**Design method:** Equiripple (Parks-McClellan / Remez) targeting:
- Passband: 0 to 0.45 × Nyquist (ripple < 0.01 dB)
- Stopband: 0.55 × Nyquist to Nyquist (attenuation > 96 dB)
- Transition band: 0.45 to 0.55 × Nyquist

96 dB stopband attenuation suppresses aliasing/imaging below 16-bit noise floor.

**Pre-computed coefficients:** The filter coefficients are computed offline and stored as constants. No runtime filter design needed. One set of coefficients works for all sample rates (half-band filters are sample-rate independent by design).

### Cascade Structure

```
4x oversampling (2 stages):

input (fs)
  → [Stage 0: ↑2x] → (2fs) work_buffer[0]
  → [Stage 1: ↑2x] → (4fs) work_buffer[1]
  → [callback: process at 4fs]
  → [Stage 1: ↓2x] → (2fs) work_buffer[0]
  → [Stage 0: ↓2x] → (fs)
output

8x oversampling (3 stages):
  Same pattern with 3 stages.
```

Each stage doubles/halves the rate. Work buffers are pre-allocated at construction.

### Latency

Each 2x half-band stage introduces `order / 2` samples of delay at the upsampled rate. After downsampling, this becomes `order / 4` samples at the original rate.

With 12th-order filters:
- 1x: 0 samples
- 2x: 3 samples (1 stage × 12/4)
- 4x: 6 samples (2 stages × 12/4)
- 8x: 9 samples (3 stages × 12/4)

Reported via `Oversampler::latency()`. The mixer's PDC mechanism compensates automatically.

### Factor = 1 Bypass

When `factor == 1`, `process()` copies input to output and calls the callback directly on the output buffer. Zero overhead, zero latency. No filter stages allocated.

---

## Limiter Integration

### Changes to dynamics/limiter.rs

The `oversampling` parameter (ID 5) becomes functional:

1. **Construction:** Create two `Oversampler` instances (L/R) based on `oversampling` value
2. **process_effect():** Wrap the peak detection + gain computation in `oversampler.process()`
3. **latency():** Return `lookahead_samples + oversampler.latency()`
4. **set_param(5, value):** When oversampling changes, recreate the Oversampler instances and reset state

### True Peak Detection Benefit

At 1x (44.1kHz), inter-sample peaks between two consecutive samples can exceed both samples' values by up to +3 dB. At 2x (88.2kHz), the interpolated samples reveal these peaks. At 4x, detection is effectively perfect for broadcast compliance (ITU-R BS.1770).

---

## Testing Strategy

### Oversampler Unit Tests

| Test | Description |
|------|-------------|
| `upsample_preserves_dc` | DC signal (all 1.0) upsampled 2x should produce all 1.0 (after filter settling) |
| `downsample_recovers_original` | Low-freq sine (100Hz @ 44.1kHz) through up→callback(identity)→down should match original. SNR > 120 dB for integer-ratio conversion. |
| `alias_rejection` | Generate a tone at 0.4 × Nyquist, upsample, add a tone at 0.8 × original Nyquist (which is 0.4 × upsampled Nyquist, i.e., in the image band), downsample. The added tone should be attenuated > 90 dB. |
| `latency_correct` | Feed impulse, measure delay in output. Should match `latency()` return value. |
| `cascade_2x_equals_4x` | Process same signal through factor=4 and through two sequential factor=2. Results should be identical (bit-exact or within floating-point tolerance). |
| `reset_clears_state` | After processing, reset(), process silence — output should be all zeros. |
| `factor_one_is_passthrough` | factor=1 should produce output == input (bit-exact). |

### Limiter Integration Test

| Test | Description |
|------|-------------|
| `limiter_oversampling_2x_catches_intersample_peaks` | Generate a signal with known inter-sample peaks. At 1x, some peaks slip through ceiling. At 2x, ceiling is respected. |

---

## Implementation Order

```
(1) Oversampler + HalfBandStage implementation + unit tests
(2) Integrate into Limiter (make oversampling param functional)
(3) Update latency reporting
(4) Run full workspace tests
```

Compact scope — one new file, one modified file, ~300 lines total.
