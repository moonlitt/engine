# New Effects Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 10 new effects + shared infrastructure to moonlitt-effects (limiter, gate, de-esser, delay, chorus, flanger, phaser, tremolo, gain, stereo width).

**Architecture:** Bottom-up by dependency: shared infrastructure (param_smoother, denormal, LFO, delay_line) → dynamics (limiter, gate, de-esser) → modulation (delay, chorus, flanger, phaser, tremolo) → utility (gain, stereo_width) → integration (lib.rs, feature flags, bindings).

**Tech Stack:** Pure Rust, no new external deps. Sinc interpolation via self-contained Kaiser-windowed sinc table. All effects implement `AudioBackend` trait from moonlitt-core.

**Spec:** `docs/superpowers/specs/2026-04-02-new-effects-design.md`

**Baseline:** `cargo test --workspace -- --skip pianoteq --skip keyscape` — all tests pass.

**Existing pattern to follow:** See `crates/moonlitt-effects/src/dynamics/compressor.rs` for the canonical AudioBackend implementation — struct with params + state, `new(sample_rate)` constructor, bypass check in process_effect, param_count/param_info/get_param/set_param/param_display system.

---

## Phase 1 — Shared Infrastructure

### Task 1: ParamSmoother + Denormal

Create `common/` module with parameter smoothing and denormal protection utilities.

**Files:**
- Create: `crates/moonlitt-effects/src/common/mod.rs`
- Create: `crates/moonlitt-effects/src/common/param_smoother.rs`
- Create: `crates/moonlitt-effects/src/common/denormal.rs`
- Modify: `crates/moonlitt-effects/src/lib.rs` — add `pub mod common;`

- [ ] **Step 1: Write failing tests for ParamSmoother**

Create `crates/moonlitt-effects/src/common/param_smoother.rs` with tests at the bottom:

```rust
// ... (implementation placeholder — struct + impl will be empty)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoother_reaches_target() {
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        // After ~10ms at 44100Hz = ~441 samples, should be near target
        for _ in 0..4410 {
            s.next();
        }
        assert!((s.next() - 1.0).abs() < 0.001, "Should reach target after 100ms");
    }

    #[test]
    fn smoother_starts_at_initial() {
        let s = ParamSmoother::new(5.0, 44100.0, 10.0);
        assert_eq!(s.next_value(), 5.0);
    }

    #[test]
    fn smoother_settled_when_at_target() {
        let s = ParamSmoother::new(1.0, 44100.0, 10.0);
        assert!(s.is_settled());
    }

    #[test]
    fn smoother_not_settled_after_target_change() {
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        assert!(!s.is_settled());
    }

    #[test]
    fn smoother_reset_jumps_immediately() {
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        s.reset(5.0);
        assert_eq!(s.next_value(), 5.0);
        assert!(s.is_settled());
    }

    #[test]
    fn smoother_ramp_timing() {
        // After 1 time constant (~10ms), should be at ~63.2% of target
        let mut s = ParamSmoother::new(0.0, 44100.0, 10.0);
        s.set_target(1.0);
        let samples_10ms = (44100.0 * 0.01) as usize;
        for _ in 0..samples_10ms {
            s.next();
        }
        let val = s.next();
        // Should be near 0.632 (1 - e^-1)
        assert!((val - 0.632).abs() < 0.05, "After 1 TC, value={val}, expected ~0.632");
    }
}
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
cargo test -p moonlitt-effects common::param_smoother 2>&1
```

Expected: compilation error — `ParamSmoother` not defined.

- [ ] **Step 3: Implement ParamSmoother**

```rust
/// Exponential parameter smoother — prevents click/pop on parameter changes.
///
/// Use for every continuous parameter that affects the signal path
/// (gain, frequency, delay time, mix). Do NOT use for discrete params
/// (bypass, mode, detection_mode).
pub struct ParamSmoother {
    current: f64,
    target: f64,
    coeff: f64,
    threshold: f64,
}

impl ParamSmoother {
    /// Create a smoother.
    /// - `initial`: starting value
    /// - `sample_rate`: audio sample rate
    /// - `ramp_ms`: smoothing time (5-20ms typical)
    pub fn new(initial: f64, sample_rate: f64, ramp_ms: f64) -> Self {
        let samples = ramp_ms * 0.001 * sample_rate;
        let coeff = if samples > 0.0 {
            (-1.0 / samples).exp()
        } else {
            0.0
        };
        Self {
            current: initial,
            target: initial,
            coeff,
            threshold: 1e-8,
        }
    }

    /// Set new target value. Smoother will ramp toward it.
    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    /// Call once per sample. Returns smoothed value.
    #[inline]
    pub fn next(&mut self) -> f64 {
        if (self.current - self.target).abs() < self.threshold {
            self.current = self.target;
        } else {
            self.current = self.coeff * self.current + (1.0 - self.coeff) * self.target;
        }
        self.current
    }

    /// Peek at the current value without advancing.
    pub fn next_value(&self) -> f64 {
        self.current
    }

    /// True when current == target (within threshold).
    pub fn is_settled(&self) -> bool {
        (self.current - self.target).abs() < self.threshold
    }

    /// Jump immediately to a value (for initialization, not during playback).
    pub fn reset(&mut self, value: f64) {
        self.current = value;
        self.target = value;
    }
}
```

- [ ] **Step 4: Write denormal.rs**

```rust
/// Flush denormal float values to zero.
///
/// Call on every sample in feedback paths (delay, flanger, phaser, reverb comb)
/// to prevent CPU spikes from denormalized floating-point arithmetic.
#[inline(always)]
pub fn flush_denormal(x: f32) -> f32 {
    // f32 denormals have exponent bits all zero and non-zero mantissa.
    // Any value with absolute magnitude below ~1.2e-38 is denormal.
    // We use a practical threshold well above the denormal range.
    if x.abs() < 1e-15 {
        0.0
    } else {
        x
    }
}

/// f64 version for internal processing paths.
#[inline(always)]
pub fn flush_denormal_f64(x: f64) -> f64 {
    if x.abs() < 1e-30 {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_values_pass_through() {
        assert_eq!(flush_denormal(1.0), 1.0);
        assert_eq!(flush_denormal(-0.5), -0.5);
        assert_eq!(flush_denormal(1e-10_f32), 1e-10_f32);
    }

    #[test]
    fn tiny_values_become_zero() {
        assert_eq!(flush_denormal(1e-20_f32), 0.0);
        assert_eq!(flush_denormal(-1e-20_f32), 0.0);
        assert_eq!(flush_denormal(0.0), 0.0);
    }

    #[test]
    fn f64_normal_values_pass_through() {
        assert_eq!(flush_denormal_f64(1.0), 1.0);
        assert_eq!(flush_denormal_f64(1e-20), 1e-20);
    }

    #[test]
    fn f64_tiny_values_become_zero() {
        assert_eq!(flush_denormal_f64(1e-35), 0.0);
    }
}
```

- [ ] **Step 5: Create common/mod.rs**

```rust
pub mod denormal;
pub mod param_smoother;

pub use denormal::{flush_denormal, flush_denormal_f64};
pub use param_smoother::ParamSmoother;
```

- [ ] **Step 6: Add common module to lib.rs**

In `crates/moonlitt-effects/src/lib.rs`, add at the top (before the `#[cfg(feature ...)]` blocks):

```rust
pub mod common;
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p moonlitt-effects common 2>&1
```

Expected: all 10 tests pass (6 smoother + 4 denormal).

- [ ] **Step 8: Run workspace tests for regression**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

Expected: all existing tests still pass.

- [ ] **Step 9: Commit**

```bash
git add crates/moonlitt-effects/src/common/
git add crates/moonlitt-effects/src/lib.rs
git commit -m "feat(effects): add param_smoother and denormal utilities

Shared infrastructure for all effects:
- ParamSmoother: exponential parameter smoothing (5-20ms ramp)
- flush_denormal: denormal protection for feedback paths"
```

---

### Task 2: LFO

Multi-waveform LFO with tempo sync. Shared by chorus, flanger, phaser, tremolo, delay.

**Files:**
- Create: `crates/moonlitt-effects/src/modulation/mod.rs`
- Create: `crates/moonlitt-effects/src/modulation/lfo.rs`

- [ ] **Step 1: Write LFO tests**

Write tests in `crates/moonlitt-effects/src/modulation/lfo.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_output_range() {
        let mut lfo = Lfo::new(44100);
        for _ in 0..44100 {
            let v = lfo.next(1.0);
            assert!(v >= -1.0 && v <= 1.0, "Sine out of range: {v}");
        }
    }

    #[test]
    fn sine_completes_one_cycle() {
        let mut lfo = Lfo::new(44100);
        let mut crossed_zero_positive = 0;
        let mut prev = lfo.next(1.0);
        for _ in 1..44100 {
            let v = lfo.next(1.0);
            if prev <= 0.0 && v > 0.0 {
                crossed_zero_positive += 1;
            }
            prev = v;
        }
        assert_eq!(crossed_zero_positive, 1, "1 Hz sine should cross zero upward once per second");
    }

    #[test]
    fn triangle_output_range() {
        let mut lfo = Lfo::new(44100);
        lfo.set_shape(LfoShape::Triangle);
        for _ in 0..44100 {
            let v = lfo.next(2.0);
            assert!(v >= -1.0 && v <= 1.0, "Triangle out of range: {v}");
        }
    }

    #[test]
    fn saw_ramps_up() {
        let mut lfo = Lfo::new(1000);
        lfo.set_shape(LfoShape::Saw);
        let v0 = lfo.next(1.0);
        let v1 = lfo.next(1.0);
        assert!(v1 > v0, "Saw should ramp up: {v0} -> {v1}");
    }

    #[test]
    fn square_is_binary() {
        let mut lfo = Lfo::new(44100);
        lfo.set_shape(LfoShape::Square);
        for _ in 0..44100 {
            let v = lfo.next(1.0);
            assert!(v == 1.0 || v == -1.0, "Square should be ±1, got {v}");
        }
    }

    #[test]
    fn sample_and_hold_holds() {
        let mut lfo = Lfo::new(100); // 100 Hz SR
        lfo.set_shape(LfoShape::SampleAndHold);
        let v0 = lfo.next(1.0); // freq=1Hz, period=100 samples
        let v1 = lfo.next(1.0);
        assert_eq!(v0, v1, "S&H should hold value within same cycle");
    }

    #[test]
    fn note_value_quarter_at_120bpm() {
        let ms = NoteValue::Quarter.to_ms(120.0);
        assert!((ms - 500.0).abs() < 0.01, "1/4 @ 120 BPM = 500ms, got {ms}");
    }

    #[test]
    fn note_value_eighth_at_120bpm() {
        let ms = NoteValue::Eighth.to_ms(120.0);
        assert!((ms - 250.0).abs() < 0.01, "1/8 @ 120 BPM = 250ms, got {ms}");
    }

    #[test]
    fn note_value_dotted_eighth_at_120bpm() {
        let ms = NoteValue::DottedEighth.to_ms(120.0);
        assert!((ms - 375.0).abs() < 0.01, "1/8. @ 120 BPM = 375ms, got {ms}");
    }

    #[test]
    fn note_value_triplet_at_120bpm() {
        let ms = NoteValue::EighthTriplet.to_ms(120.0);
        // 1/8T = 2/3 of 1/8 = 2/3 * 250 = 166.67ms
        assert!((ms - 166.667).abs() < 0.1, "1/8T @ 120 BPM = 166.67ms, got {ms}");
    }

    #[test]
    fn tempo_sync_matches_free() {
        let mut lfo = Lfo::new(44100);
        let synced = lfo.next_synced(120.0, NoteValue::Quarter);
        // Quarter at 120bpm = 500ms = 2Hz
        let mut lfo2 = Lfo::new(44100);
        let free = lfo2.next(2.0);
        assert_eq!(synced, free, "Synced should produce same as free at equivalent Hz");
    }

    #[test]
    fn reset_phase_resets() {
        let mut lfo = Lfo::new(44100);
        for _ in 0..1000 {
            lfo.next(10.0);
        }
        lfo.reset_phase();
        let mut lfo2 = Lfo::new(44100);
        assert_eq!(lfo.next(10.0), lfo2.next(10.0));
    }
}
```

- [ ] **Step 2: Implement LFO**

Implement `NoteValue` enum with `to_ms()` and `to_hz()`, `LfoShape` enum, and `Lfo` struct in `crates/moonlitt-effects/src/modulation/lfo.rs`.

Key implementation details:
- Phase accumulator: `phase += freq_hz / sample_rate`, wrap at 1.0
- Sine: `(phase * 2π).sin()`
- Triangle: `4 * |phase - 0.5| - 1` (shifted sawtooth folded)
- Saw: `2 * phase - 1`
- Square: `if phase < 0.5 { 1.0 } else { -1.0 }`
- S&H: xorshift64 PRNG triggered on phase wrap, output scaled to -1..1

`NoteValue::to_ms(bpm)`:
- Base: `beat_ms = 60000.0 / bpm`
- Quarter = 1× beat_ms
- Eighth = 0.5× beat_ms
- Triplet variants = 2/3 of their straight equivalent
- Dotted variants = 1.5× their straight equivalent
- ThirtySecond = 1/8 × beat_ms
- Whole = 4× beat_ms
- TwoBar = 8× beat_ms, FourBar = 16× beat_ms

`NoteValue::to_hz(bpm)` = `1000.0 / to_ms(bpm)`

`next_synced(bpm, note)` = `next(note.to_hz(bpm))`

- [ ] **Step 3: Create modulation/mod.rs**

```rust
pub mod lfo;
// delay_line will be added in Task 3
```

- [ ] **Step 4: Add modulation module to lib.rs** (behind a temporary always-on gate since it's infrastructure)

In `crates/moonlitt-effects/src/lib.rs`, add:

```rust
#[cfg(any(
    feature = "delay", feature = "chorus", feature = "flanger",
    feature = "phaser", feature = "tremolo"
))]
pub mod modulation;
```

For now during development, the `modulation` module needs to compile. Since `default = ["all"]` includes all features, this works.

- [ ] **Step 5: Run tests**

```bash
cargo test -p moonlitt-effects modulation::lfo 2>&1
```

Expected: all 12 LFO tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/moonlitt-effects/src/modulation/
git add crates/moonlitt-effects/src/lib.rs
git commit -m "feat(effects): add LFO with 5 waveforms and tempo sync

Sine, Triangle, Saw, Square, Sample&Hold.
NoteValue enum with 17 musical divisions (1/32 to 4 bars).
Tempo sync via next_synced(bpm, note)."
```

---

### Task 3: FractionalDelayLine (sinc interpolation)

Self-contained Kaiser-windowed sinc interpolation delay line. The core DSP building block for chorus, flanger, and delay effects.

**Files:**
- Create: `crates/moonlitt-effects/src/modulation/delay_line.rs`
- Modify: `crates/moonlitt-effects/src/modulation/mod.rs` — add `pub mod delay_line;`

- [ ] **Step 1: Write tests**

Add tests at bottom of `delay_line.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_delay_is_exact() {
        let mut dl = FractionalDelayLine::new(100.0, 44100, 8);
        // Write impulse
        dl.write(1.0);
        for _ in 1..100 {
            dl.write(0.0);
        }
        // Read at integer delay of 50 samples
        // Need to rewind — actually, the read is relative to current write_pos
        // So write 50 zeros, then read at delay=50
        let mut dl2 = FractionalDelayLine::new(100.0, 44100, 8);
        dl2.write(1.0);
        for _ in 0..49 {
            dl2.write(0.0);
        }
        let val = dl2.read(50.0);
        assert!((val - 1.0).abs() < 1e-6, "Integer delay should be exact, got {val}");
    }

    #[test]
    fn zero_delay_returns_current() {
        let mut dl = FractionalDelayLine::new(10.0, 44100, 8);
        dl.write(0.5);
        // delay of 0 should return most recent sample
        let val = dl.read(0.0);
        assert!((val - 0.5).abs() < 1e-6, "Zero delay should return current sample, got {val}");
    }

    #[test]
    fn fractional_delay_interpolates() {
        let mut dl = FractionalDelayLine::new(100.0, 44100, 8);
        // Write a simple ramp
        for i in 0..100 {
            dl.write(i as f32 / 100.0);
        }
        let val = dl.read(10.5);
        // Should be between the values at delay=10 and delay=11
        let v10 = dl.read(10.0);
        let v11 = dl.read(11.0);
        assert!(val > v11.min(v10) && val < v11.max(v10),
            "Fractional delay should interpolate: {v11} < {val} < {v10}");
    }

    #[test]
    fn sinc_quality_exceeds_linear() {
        // Test with a sine wave — sinc should preserve amplitude better
        let sr = 44100;
        let freq = 1000.0; // 1kHz test tone
        let mut dl = FractionalDelayLine::new(50.0, sr, 8);
        let mut dl_linear = FractionalDelayLine::new(50.0, sr, 8);

        let num_samples = 4410; // 100ms
        let delay_frac = 20.7; // fractional delay

        let mut sinc_error = 0.0f64;
        let mut linear_error = 0.0f64;

        for i in 0..num_samples {
            let sample = (2.0 * std::f64::consts::PI * freq * i as f64 / sr as f64).sin() as f32;
            dl.write(sample);
            dl_linear.write(sample);

            if i >= delay_frac as usize + 10 {
                let expected = (2.0 * std::f64::consts::PI * freq * (i as f64 - delay_frac) / sr as f64).sin() as f32;
                let sinc_val = dl.read(delay_frac);
                let lin_val = dl_linear.read_linear(delay_frac);
                sinc_error += (sinc_val - expected).powi(2) as f64;
                linear_error += (lin_val - expected).powi(2) as f64;
            }
        }
        assert!(sinc_error < linear_error,
            "Sinc error ({sinc_error:.6}) should be less than linear error ({linear_error:.6})");
    }

    #[test]
    fn clear_resets_buffer() {
        let mut dl = FractionalDelayLine::new(10.0, 44100, 8);
        dl.write(1.0);
        dl.clear();
        let val = dl.read(0.0);
        assert_eq!(val, 0.0, "After clear, all values should be zero");
    }

    #[test]
    fn max_delay_respected() {
        let dl = FractionalDelayLine::new(10.0, 44100, 8);
        let max = dl.max_delay_samples();
        assert!(max >= (44100.0 * 0.01) as usize, "Max delay should be at least 10ms worth of samples");
    }
}
```

- [ ] **Step 2: Implement FractionalDelayLine**

Key implementation:
- `SincTable`: pre-compute `num_points × oversample` entries. For each fractional offset `frac` (0..1, quantized to `oversample` steps), store `num_points` sinc kernel values windowed with Kaiser.
- `write(sample)`: circular buffer write at `write_pos`, increment
- `read(delay_samples)`: decompose into integer part + fractional part, look up sinc kernel for fractional offset, convolve with `num_points` surrounding samples
- `read_linear(delay_samples)`: simple 2-point linear interpolation for non-critical paths
- Kaiser window: `kaiser(n, half_len, beta) = I₀(beta * sqrt(1 - (n/half_len)²)) / I₀(beta)`, beta=6.2 for 8-point
- Bessel I₀ approximation: series expansion to 20 terms
- Buffer size = `max_delay_samples + num_points + 1` (extra for interpolation kernel)

```rust
use std::f64::consts::PI;

pub struct FractionalDelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    max_delay_samples: usize,
    sinc_table: SincTable,
}

struct SincTable {
    table: Vec<f32>,
    num_points: usize,
    oversample: usize,
}
```

- [ ] **Step 3: Add to modulation/mod.rs**

```rust
pub mod delay_line;
pub mod lfo;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p moonlitt-effects modulation::delay_line 2>&1
```

Expected: all 6 delay_line tests pass.

- [ ] **Step 5: Run workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add crates/moonlitt-effects/src/modulation/delay_line.rs
git add crates/moonlitt-effects/src/modulation/mod.rs
git commit -m "feat(effects): add sinc-interpolated fractional delay line

Kaiser-windowed sinc table (8-point, beta=6.2, 256x oversampling).
Per-sample fractional read for modulation effects.
Also provides linear interpolation fallback for non-critical paths."
```

---

## Phase 2 — Dynamics

### Task 4: Limiter

Lookahead brickwall limiter with auto-release.

**Files:**
- Create: `crates/moonlitt-effects/src/dynamics/limiter.rs`
- Modify: `crates/moonlitt-effects/src/dynamics/mod.rs` — add `pub mod limiter;`

The implementer should:
1. Read `compressor.rs` as the pattern to follow for AudioBackend impl
2. Read `envelope.rs` to understand the envelope follower being reused
3. Implement the limiter with:
   - Lookahead delay buffer (circular, size = lookahead_ms × sample_rate / 1000)
   - Peak detection on un-delayed input
   - Gain computation: `gain_db = min(0, threshold_db - peak_db)`
   - Two envelope followers for auto-release (fast=2ms, slow=100ms)
   - Crest factor blend: `blend = fast_env / (fast_env + slow_env + 1e-10)`
   - Final gain envelope = `fast_gain * blend + slow_gain * (1-blend)`
   - Ceiling hard clip at output: `output.clamp(-ceiling_linear, ceiling_linear)`
   - `latency()` returns lookahead in samples
4. 8 parameters as defined in spec
5. Tests: bypass_is_bitexact, param_round_trip, peak_never_exceeds_ceiling (feed 2.0 amplitude sine, verify output never exceeds ceiling), lookahead_latency_correct, auto_release_adapts

- [ ] **Step 1: Write tests (bypass, param_round_trip, peak ceiling, latency, auto-release)**
- [ ] **Step 2: Run tests — verify they fail**
- [ ] **Step 3: Implement Limiter struct + AudioBackend**
- [ ] **Step 4: Run tests — verify they pass**
- [ ] **Step 5: Run workspace tests**
- [ ] **Step 6: Commit**

```bash
git commit -m "feat(effects): add brickwall limiter with lookahead and auto-release

8 params: threshold, ceiling, release, lookahead, attack, oversampling,
auto_release, bypass. Reports latency for PDC compensation."
```

---

### Task 5: Gate

Noise gate with hysteresis, hold timer, and dual sidechain filters.

**Files:**
- Create: `crates/moonlitt-effects/src/dynamics/gate.rs`
- Modify: `crates/moonlitt-effects/src/dynamics/mod.rs` — add `pub mod gate;`

The implementer should:
1. Follow compressor.rs pattern
2. Reuse `EnvelopeFollower` for attack/release smoothing
3. Reuse `Biquad` (from eq module) for sidechain HPF and LPF
4. Gate state machine: Open → Hold → Release → Closed, with hysteresis
5. 10 parameters as defined in spec
6. Tests: bypass_is_bitexact, param_round_trip, below_threshold_attenuates (verify output is reduced by range_db), hold_prevents_chattering (rapid on/off input should not cause rapid gating), hysteresis_works (close threshold is lower than open threshold)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement Gate struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add noise gate with hysteresis and hold timer

10 params: threshold, range, attack, hold, release, hysteresis,
sidechain HPF/LPF, detection mode, bypass."
```

---

### Task 6: De-esser

Split-band sibilance reduction.

**Files:**
- Create: `crates/moonlitt-effects/src/dynamics/deesser.rs`
- Modify: `crates/moonlitt-effects/src/dynamics/mod.rs` — add `pub mod deesser;`

The implementer should:
1. Follow compressor.rs pattern
2. Use `Biquad` bandpass for sibilance detection
3. Use `Biquad` bandpass + inverse for split-band mode (band = BPF output, rest = input - band)
4. Use `EnvelopeFollower` for detection smoothing
5. Gain reduction uses compressor's `compute_gain_db_static` formula (import or inline)
6. Listen mode: output = sidechain band signal only
7. 7 parameters as defined in spec
8. Tests: bypass_is_bitexact, param_round_trip, sibilance_attenuated (feed 6kHz sine above threshold, verify attenuation), non_sibilant_passes (feed 200Hz sine, verify passes unchanged in split mode), listen_mode_outputs_band (verify output equals bandpass-filtered input)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement DeEsser struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add de-esser with split-band and listen mode

7 params: threshold, frequency, bandwidth, ratio, mode, listen, bypass.
Wideband and split-band modes."
```

---

## Phase 3 — Modulation

### Task 7: Delay

Stereo delay with tempo sync, ping-pong, and filtered feedback.

**Files:**
- Create: `crates/moonlitt-effects/src/modulation/delay.rs`
- Modify: `crates/moonlitt-effects/src/modulation/mod.rs` — add `pub mod delay;`

The implementer should:
1. Use `FractionalDelayLine` (from Task 3) for each channel — max delay 5000ms
2. Use `Biquad` for feedback LP and HP filters
3. Use `ParamSmoother` for delay time, feedback, filter freqs, dry_wet
4. Use `flush_denormal` in feedback path
5. Ping-pong: cross-feed delay outputs (L→R, R→L) instead of self-feedback
6. Tempo sync: when sync_mode=1, compute delay from `NoteValue::to_ms(bpm)`
7. 12 parameters as defined in spec
8. Tests: bypass_is_bitexact, param_round_trip, delay_time_correct (verify impulse appears at correct sample offset), feedback_decays (each repeat is quieter by feedback factor), ping_pong_alternates (impulse in L appears in R on first repeat), tempo_sync_correct (verify delay matches note value at given BPM)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement Delay struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add stereo delay with tempo sync and ping-pong

12 params: time L/R, sync mode, sync notes, BPM, feedback,
ping-pong, filter LP/HP, dry/wet, bypass.
Sinc-interpolated delay line, denormal-protected feedback."
```

---

### Task 8: Chorus

4-voice modulated delay with sinc interpolation.

**Files:**
- Create: `crates/moonlitt-effects/src/modulation/chorus.rs`
- Modify: `crates/moonlitt-effects/src/modulation/mod.rs` — add `pub mod chorus;`

The implementer should:
1. 4 voices, each with own `FractionalDelayLine` (max ~60ms) and `Lfo` (phase offset = i/n)
2. Use `Biquad` lowpass for high_cut on wet signal
3. Use `ParamSmoother` for rate, depth, delay_ms, dry_wet, high_cut
4. Stereo spread: voice i pans to `(i as f32 / (voices-1) - 0.5) * 2 * spread`
5. Modulation depth: `actual_delay = delay_ms + lfo_value * depth * delay_ms * 0.5`
6. 8 parameters as defined in spec
7. Tests: bypass_is_bitexact, param_round_trip, multi_voice_thicker (4 voices produces more energy than 1 voice), stereo_spread_distributes (spread=1 puts voices in different channels), depth_zero_no_modulation (output is constant delay, no pitch variation)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement Chorus struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add 4-voice chorus with sinc interpolation

8 params: rate, depth, delay, voices, stereo spread, high cut,
dry/wet, bypass. Per-voice LFO phase distribution."
```

---

### Task 9: Flanger

Single delay line + feedback + through-zero + soft saturation.

**Files:**
- Create: `crates/moonlitt-effects/src/modulation/flanger.rs`
- Modify: `crates/moonlitt-effects/src/modulation/mod.rs` — add `pub mod flanger;`

The implementer should:
1. Use `FractionalDelayLine` (max ~20ms) per channel
2. Use `Lfo` per channel with stereo phase offset
3. Use `ParamSmoother` for rate, depth, delay_ms, feedback, dry_wet
4. Feedback path: `feedback_sample = flush_denormal(tanh(delayed * feedback))`
5. Negative feedback = through-zero: polarity of wet signal flips
6. LFO shapes: all 5 via `lfo_shape` parameter
7. 11 parameters as defined in spec (including sync_mode, sync_note, bpm)
8. Tests: bypass_is_bitexact, param_round_trip, through_zero_flips_polarity (negative feedback produces inverted wet signal), saturation_bounds_output (high feedback + loud input stays bounded), feedback_zero_no_resonance (zero feedback = no resonance, single delay tap only)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement Flanger struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add flanger with through-zero and soft saturation

11 params: rate, depth, delay, feedback (neg=through-zero),
stereo phase, LFO shape, dry/wet, bypass, sync mode/note/BPM.
tanh saturation in feedback path."
```

---

### Task 10: Phaser

N-stage allpass cascade with LFO-modulated center frequencies.

**Files:**
- Create: `crates/moonlitt-effects/src/modulation/phaser.rs`
- Modify: `crates/moonlitt-effects/src/modulation/mod.rs` — add `pub mod phaser;`

The implementer should:
1. First-order allpass stages (NOT biquad): `y = -a*x + z; z = a*y + x` where `a = (tan(π*f/sr) - 1) / (tan(π*f/sr) + 1)`
2. Up to 12 stages per channel (allocate max, use `stages` param to control active count)
3. Use `Lfo` per channel with stereo phase offset
4. Exponential frequency sweep: `freq = min_freq * (max_freq/min_freq).powf(lfo_unipolar)`
5. Use `ParamSmoother` for rate, depth, min_freq, max_freq, feedback
6. Denormal protection in feedback path
7. Dry/wet mix: `output = input + wet_signal` (standard phaser mix, not crossfade — dry is always full, wet adds the phase-cancelled signal)
8. 11 parameters as defined in spec
9. Tests: bypass_is_bitexact, param_round_trip, more_stages_more_notches (compare FFT of 4-stage vs 8-stage output — 8 should have more spectral dips), feedback_deepens_notches (higher feedback = deeper notches), frequency_sweep_range (verify allpass frequencies stay within min/max)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement Phaser struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add phaser with N-stage allpass and exponential sweep

11 params: rate, depth, stages (2-12), feedback, min/max freq,
stereo phase, sync mode/note/BPM, bypass.
First-order allpass cascade, exponential frequency mapping."
```

---

### Task 11: Tremolo

Amplitude modulation with tempo sync and stereo auto-pan.

**Files:**
- Create: `crates/moonlitt-effects/src/modulation/tremolo.rs`
- Modify: `crates/moonlitt-effects/src/modulation/mod.rs` — add `pub mod tremolo;`

The implementer should:
1. Use `Lfo` — one per channel for stereo mode
2. Use `ParamSmoother` for rate and depth
3. Gain formula: `gain = 1.0 - depth + depth * (lfo * 0.5 + 0.5)`
4. Stereo mode: R channel LFO is 180° out of phase with L
5. All 5 LFO shapes supported
6. Tempo sync via sync_mode/sync_note/bpm params
7. 8 parameters as defined in spec
8. Tests: bypass_is_bitexact, param_round_trip, depth_zero_passthrough (depth=0 should be identity), depth_one_reaches_silence (at LFO minimum, output should be zero), stereo_mode_opposite (L and R amplitudes should be inversely related)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement Tremolo struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add tremolo with tempo sync and stereo auto-pan

8 params: rate, depth, LFO shape, stereo mode, sync mode/note/BPM,
bypass. Mono and auto-pan modes."
```

---

## Phase 4 — Utility

### Task 12: Gain

Simple gain + polarity + mono.

**Files:**
- Create: `crates/moonlitt-effects/src/utility/mod.rs`
- Create: `crates/moonlitt-effects/src/utility/gain.rs`

The implementer should:
1. Follow compressor.rs AudioBackend pattern
2. Use `ParamSmoother` for gain_db
3. `gain_linear = 10.0_f64.powf(gain_db / 20.0)`, clamp gain_db at -120
4. Polarity: multiply by -1 when enabled
5. Mono: `(L + R) / 2` to both channels
6. 4 parameters as defined in spec
7. Tests: bypass_is_bitexact, param_round_trip, gain_maps_correctly (0dB=unity, 6dB≈2×, -6dB≈0.5×), polarity_inverts (output is negative of input), mono_sums (L=1,R=0 → both=0.5)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement Gain struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Add utility module to lib.rs**

```rust
#[cfg(any(feature = "gain", feature = "stereo-width"))]
pub mod utility;
```

- [ ] **Step 5: Run workspace tests**
- [ ] **Step 6: Commit**

```bash
git commit -m "feat(effects): add gain utility (gain + polarity + mono)

4 params: gain_db, polarity, mono, bypass.
Smoothed gain to prevent clicks."
```

---

### Task 13: Stereo Width

Mid/side processing for stereo width control.

**Files:**
- Create: `crates/moonlitt-effects/src/utility/stereo_width.rs`
- Modify: `crates/moonlitt-effects/src/utility/mod.rs` — add `pub mod stereo_width;`

The implementer should:
1. Follow compressor.rs AudioBackend pattern
2. Use `ParamSmoother` for width, mid_gain_db, side_gain_db
3. M/S encode: `mid = (L + R) / 2`, `side = (L - R) / 2`
4. M/S process: `mid *= mid_gain`, `side *= side_gain * width`
5. M/S decode: `L = mid + side`, `R = mid - side`
6. 4 parameters as defined in spec
7. Tests: bypass_is_bitexact, param_round_trip, width_zero_mono (width=0 should produce identical L and R), width_one_passthrough (width=1, gains=0dB should be identity), mid_side_independent (boosting mid with side=0 should only affect center-panned content)

- [ ] **Step 1: Write tests**
- [ ] **Step 2: Implement StereoWidth struct + AudioBackend**
- [ ] **Step 3: Run tests — verify they pass**
- [ ] **Step 4: Run workspace tests**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat(effects): add stereo width (mid/side processing)

4 params: width, mid_gain_db, side_gain_db, bypass.
M/S encode → process → decode."
```

---

## Phase 5 — Integration

### Task 14: Update lib.rs and Cargo.toml

Wire all new effects into the crate's public API with feature flags.

**Files:**
- Modify: `crates/moonlitt-effects/Cargo.toml`
- Modify: `crates/moonlitt-effects/src/lib.rs`
- Modify: `crates/moonlitt-effects/src/dynamics/mod.rs`
- Modify: `crates/moonlitt-effects/src/modulation/mod.rs`
- Modify: `crates/moonlitt-effects/src/utility/mod.rs`

- [ ] **Step 1: Update Cargo.toml features**

Replace the features section with:

```toml
[features]
default = ["all"]
all = ["dynamics", "eq", "spatial", "modulation", "utility"]
dynamics = ["compressor", "limiter", "gate", "deesser"]
eq = ["parametric-eq"]
spatial = ["reverb", "convolver"]
modulation = ["delay", "chorus", "flanger", "phaser", "tremolo"]
utility = ["gain", "stereo-width"]
compressor = []
limiter = []
gate = []
deesser = []
parametric-eq = []
reverb = []
convolver = ["dep:rustfft"]
delay = []
chorus = []
flanger = []
phaser = []
tremolo = []
gain = []
stereo-width = []
```

- [ ] **Step 2: Update lib.rs**

Add feature-gated module declarations and re-exports for all new types. Follow existing pattern.

New re-exports:
```rust
// Dynamics
#[cfg(feature = "limiter")]
pub use dynamics::limiter::Limiter;
#[cfg(feature = "gate")]
pub use dynamics::gate::Gate;
#[cfg(feature = "deesser")]
pub use dynamics::deesser::DeEsser;

// Modulation
#[cfg(feature = "delay")]
pub use modulation::delay::Delay;
#[cfg(feature = "chorus")]
pub use modulation::chorus::Chorus;
#[cfg(feature = "flanger")]
pub use modulation::flanger::Flanger;
#[cfg(feature = "phaser")]
pub use modulation::phaser::Phaser;
#[cfg(feature = "tremolo")]
pub use modulation::tremolo::Tremolo;

// Utility
#[cfg(feature = "gain")]
pub use utility::gain::Gain;
#[cfg(feature = "stereo-width")]
pub use utility::stereo_width::StereoWidth;
```

- [ ] **Step 3: Update dynamics/mod.rs**

Add feature-gated modules:
```rust
#[cfg(feature = "limiter")]
pub mod limiter;
#[cfg(feature = "gate")]
pub mod gate;
#[cfg(feature = "deesser")]
pub mod deesser;
```

- [ ] **Step 4: Update modulation/mod.rs**

Add feature-gated modules:
```rust
#[cfg(feature = "delay")]
pub mod delay;
#[cfg(feature = "chorus")]
pub mod chorus;
#[cfg(feature = "flanger")]
pub mod flanger;
#[cfg(feature = "phaser")]
pub mod phaser;
#[cfg(feature = "tremolo")]
pub mod tremolo;
```

- [ ] **Step 5: Verify feature-gated compilation**

Test that individual features compile:
```bash
cargo build -p moonlitt-effects --no-default-features --features limiter 2>&1
cargo build -p moonlitt-effects --no-default-features --features delay 2>&1
cargo build -p moonlitt-effects --no-default-features --features gain 2>&1
cargo build -p moonlitt-effects 2>&1  # default = all
```

- [ ] **Step 6: Run full workspace tests**

```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 7: Commit**

```bash
git commit -m "feat(effects): wire all new effects into feature flags and re-exports

14 individual feature flags across 5 categories.
All effects accessible via moonlitt_effects::EffectName."
```

---

### Task 15: Update Node.js and C API bindings

Add factory functions for all new effects in moonlitt-node and moonlitt-capi.

**Files:**
- Modify: `crates/moonlitt-node/src/effects.rs`
- Modify: `crates/moonlitt-capi/src/builtin_api.rs`

- [ ] **Step 1: Update moonlitt-node effects.rs**

Add factory functions for each new effect:

```rust
#[napi]
pub fn create_limiter(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Limiter::new(sample_rate))) }
}

#[napi]
pub fn create_gate(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Gate::new(sample_rate))) }
}

#[napi]
pub fn create_deesser(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::DeEsser::new(sample_rate))) }
}

#[napi]
pub fn create_delay(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Delay::new(sample_rate))) }
}

#[napi]
pub fn create_chorus(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Chorus::new(sample_rate))) }
}

#[napi]
pub fn create_flanger(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Flanger::new(sample_rate))) }
}

#[napi]
pub fn create_phaser(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Phaser::new(sample_rate))) }
}

#[napi]
pub fn create_tremolo(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Tremolo::new(sample_rate))) }
}

#[napi]
pub fn create_gain(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::Gain::new(sample_rate))) }
}

#[napi]
pub fn create_stereo_width(sample_rate: u32) -> Backend {
    Backend { inner: Some(Box::new(moonlitt_effects::StereoWidth::new(sample_rate))) }
}
```

- [ ] **Step 2: Update moonlitt-capi builtin_api.rs**

Add `extern "C"` factory functions following the existing pattern (each returns `*mut EngineHandle`):

```rust
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_limiter(sample_rate: c_int) -> *mut EngineHandle { ... }
#[no_mangle]
pub extern "C" fn moonlitt_builtin_create_gate(sample_rate: c_int) -> *mut EngineHandle { ... }
// ... (same pattern for all 10 effects)
```

- [ ] **Step 3: Build and test**

```bash
cargo build --workspace 2>&1
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(node,capi): add factory functions for all new effects

10 new effects available in both Node.js and C API bindings."
```

---

### Task 16: Update CLAUDE.md

Update the architecture documentation.

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update effects section in CLAUDE.md**

Update the moonlitt-effects entry in the crate dependency graph:

```
moonlitt-effects       ← Built-in audio effects (feature-gated modules):
  ↑                      dynamics/ — compressor, limiter, gate, de-esser
  ↑                      eq/       — 8-band parametric EQ (biquad cascade)
  ↑                      spatial/  — Freeverb, Dattorro plate reverb, FFT convolver
  ↑                      modulation/ — delay, chorus, flanger, phaser, tremolo
  ↑                      utility/  — gain, stereo width (mid/side)
  ↑                      common/   — param smoother, denormal protection
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md with new effects categories"
```
