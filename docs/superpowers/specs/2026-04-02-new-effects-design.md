# New Effects Design Spec

**Date:** 2026-04-02
**Status:** Draft
**Scope:** Add 10 new effects to moonlitt-effects + shared infrastructure

## Motivation

moonlitt-effects currently has 4 effects (compressor, parametric EQ, reverb, convolver). A DAW-quality audio engine needs a complete effects suite. This spec adds 10 new effects across 3 new categories (modulation, utility, expanded dynamics) plus shared infrastructure for parameter smoothing, denormal protection, LFO, and sinc-interpolated delay lines.

## Design Principles

- **DAW-standard quality** — sinc interpolation, parameter smoothing, denormal protection, program-dependent processing. No "good enough" shortcuts.
- **Follow existing patterns** — every effect implements `AudioBackend`, uses the same `param_count/param_info/get_param/set_param` system, bypass is bit-exact copy.
- **Feature flags per effect** — WASM users can include only what they need.
- **Shared building blocks** — LFO, delay line, param smoother, denormal flush are reused across effects, not duplicated.

## Module Structure

```
crates/moonlitt-effects/src/
├── common/                     NEW: cross-category shared infrastructure
│   ├── mod.rs
│   ├── param_smoother.rs       exponential parameter smoothing (5-20ms ramp)
│   └── denormal.rs             denormal flush for feedback paths
│
├── dynamics/
│   ├── compressor.rs           EXISTING
│   ├── envelope.rs             EXISTING
│   ├── limiter.rs              NEW: brickwall, lookahead, auto-release
│   ├── gate.rs                 NEW: noise gate / expander
│   └── deesser.rs              NEW: frequency-band sibilance reduction
│
├── eq/
│   ├── biquad.rs               EXISTING
│   └── parametric.rs           EXISTING
│
├── modulation/                 NEW CATEGORY
│   ├── mod.rs
│   ├── lfo.rs                  shared: multi-waveform LFO + tempo sync
│   ├── delay_line.rs           shared: sinc-interpolated fractional delay
│   ├── delay.rs                stereo delay, ping-pong, tempo sync
│   ├── chorus.rs               4-voice modulated delay
│   ├── flanger.rs              single delay + feedback + through-zero
│   ├── phaser.rs               N-stage allpass + LFO sweep
│   └── tremolo.rs              amplitude modulation + tempo sync
│
├── spatial/
│   ├── reverb.rs               EXISTING
│   ├── dattorro.rs             EXISTING
│   ├── convolver.rs            EXISTING
│   └── (supporting modules)    EXISTING
│
├── utility/                    NEW CATEGORY
│   ├── mod.rs
│   ├── gain.rs                 gain + polarity + mono
│   └── stereo_width.rs         mid/side processing
│
└── lib.rs                      updated feature flags + re-exports
```

## Feature Flags

```toml
[features]
default = ["all"]
all = ["dynamics", "eq", "spatial", "modulation", "utility"]

# Categories
dynamics    = ["compressor", "limiter", "gate", "deesser"]
eq          = ["parametric-eq"]
spatial     = ["reverb", "convolver"]
modulation  = ["delay", "chorus", "flanger", "phaser", "tremolo"]
utility     = ["gain", "stereo-width"]

# Individual effects
compressor     = []
limiter        = []
gate           = []
deesser        = []
parametric-eq  = []
reverb         = []
convolver      = ["dep:rustfft"]
delay          = []
chorus         = []
flanger        = []
phaser         = []
tremolo        = []
gain           = []
stereo-width   = []
```

`common/` has no feature gate — it compiles whenever any effect is enabled.

## Dependencies

```toml
[dependencies]
moonlitt-core = { path = "../moonlitt-core" }
rustfft = { version = "6", optional = true }
```

The sinc interpolation in `delay_line.rs` uses a self-contained Kaiser-windowed sinc table (same math as moonlitt-resampler but extracted as a lookup table, not the block-processing API). No new external dependency needed.

---

## Shared Infrastructure

### common/param_smoother.rs

Exponential parameter smoother. Prevents click/pop when parameters change.

```rust
pub struct ParamSmoother {
    current: f64,
    target: f64,
    coeff: f64,
    threshold: f64,
}

impl ParamSmoother {
    /// Create a smoother with given ramp time.
    /// ramp_ms: typical 5-20ms. Longer = smoother but less responsive.
    pub fn new(initial: f64, sample_rate: f64, ramp_ms: f64) -> Self;

    /// Set new target value. Smoother will ramp toward it.
    pub fn set_target(&mut self, target: f64);

    /// Call once per sample. Returns smoothed value.
    pub fn next(&mut self) -> f64;

    /// True when current == target (within threshold).
    pub fn is_settled(&self) -> bool;

    /// Jump immediately to target (for initialization).
    pub fn reset(&mut self, value: f64);
}
```

Used by: every effect, for every continuous parameter that affects the signal path (gain, frequency, delay time, mix, etc.). Discrete parameters (bypass, mode, detection_mode) do not use smoothing.

### common/denormal.rs

```rust
/// Flush denormal float values to zero.
/// Call on every sample in feedback paths to prevent CPU spikes.
#[inline(always)]
pub fn flush_denormal(x: f32) -> f32 {
    if x.abs() < 1e-15 { 0.0 } else { x }
}
```

Used by: delay (feedback), flanger (feedback), phaser (feedback), comb filter (existing reverb internals should also adopt this).

### modulation/lfo.rs

Multi-waveform LFO with optional tempo sync.

```rust
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleAndHold,
}

pub enum NoteValue {
    ThirtySecond,       // 1/32
    SixteenthTriplet,   // 1/16T
    Sixteenth,          // 1/16
    DottedSixteenth,    // 1/16.
    EighthTriplet,      // 1/8T
    Eighth,             // 1/8
    DottedEighth,       // 1/8.
    QuarterTriplet,     // 1/4T
    Quarter,            // 1/4
    DottedQuarter,      // 1/4.
    HalfTriplet,        // 1/2T
    Half,               // 1/2
    DottedHalf,         // 1/2.
    WholeTriplet,       // 1/1T
    Whole,              // 1/1
    TwoBar,             // 2/1
    FourBar,            // 4/1
}

impl NoteValue {
    /// Convert to milliseconds at the given BPM.
    pub fn to_ms(&self, bpm: f64) -> f64;

    /// Convert to Hz at the given BPM.
    pub fn to_hz(&self, bpm: f64) -> f64;
}

pub struct Lfo {
    phase: f64,          // 0..1
    sample_rate: f64,
    shape: LfoShape,
    rng_state: u64,      // for S&H
    sh_value: f64,       // held S&H value
}

impl Lfo {
    pub fn new(sample_rate: u32) -> Self;
    pub fn set_shape(&mut self, shape: LfoShape);
    pub fn reset_phase(&mut self);

    /// Advance by one sample at the given frequency. Returns value in -1..1.
    pub fn next(&mut self, freq_hz: f64) -> f64;

    /// Advance with tempo sync.
    pub fn next_synced(&mut self, bpm: f64, note: NoteValue) -> f64;
}
```

### modulation/delay_line.rs

Per-sample sinc-interpolated fractional delay line.

```rust
pub struct FractionalDelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    max_delay_samples: usize,
    sinc_table: SincTable,
}

/// Pre-computed Kaiser-windowed sinc interpolation table.
struct SincTable {
    table: Vec<f32>,       // flattened [num_points × oversampling_factor]
    num_points: usize,     // sinc kernel width (8 = high quality, 16 = pristine)
    oversample: usize,     // sub-sample resolution (typically 256-1024)
}

impl FractionalDelayLine {
    /// max_delay_ms: maximum delay in milliseconds.
    /// sinc_points: interpolation kernel width (8 or 16).
    pub fn new(max_delay_ms: f64, sample_rate: u32, sinc_points: usize) -> Self;

    /// Write one sample into the delay line.
    pub fn write(&mut self, sample: f32);

    /// Read at a fractional delay position (in samples). Uses sinc interpolation.
    pub fn read(&self, delay_samples: f64) -> f32;

    /// Read with linear interpolation (cheaper, for non-critical paths like feedback filters).
    pub fn read_linear(&self, delay_samples: f64) -> f32;

    /// Clear the buffer.
    pub fn clear(&mut self);
}
```

The `SincTable` is generated once at construction using the same Kaiser window formula as moonlitt-resampler (beta=6.2, matching Sinc8 quality). This is self-contained — no runtime dependency on moonlitt-resampler.

---

## New Effects — Dynamics

### Limiter (dynamics/limiter.rs)

**Algorithm:** Lookahead brickwall limiter with program-dependent release.

```
input → [lookahead delay] → [apply gain envelope] → [ceiling clip] → output
              ↓
         [oversample 2x/4x] → [true peak detect] → [gain compute]
              ↓
         [auto-release: fast(2ms) + slow(100ms) blend] → gain envelope
```

**Parameters (8):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | threshold_db | -30..0 | -1.0 |
| 1 | ceiling_db | -30..0 | -0.3 |
| 2 | release_ms | 10..1000 | 100 |
| 3 | lookahead_ms | 0.5..5.0 | 1.0 |
| 4 | attack_ms | 0.01..5.0 | 0.1 |
| 5 | oversampling | 1/2/4 | 1 |
| 6 | auto_release | 0/1 | 1 |
| 7 | bypass | 0/1 | 0 |

**auto_release algorithm:**
- Two envelope followers: fast (2ms release) and slow (100ms release)
- Blend ratio determined by input crest factor (peak/RMS): transient-heavy material favors fast, sustained material favors slow
- Prevents pumping on sustained signals while maintaining transient transparency

**latency():** Returns `(lookahead_ms * sample_rate / 1000) as u32`. Triggers mixer PDC.

**Smoothed params:** threshold_db, ceiling_db (via ParamSmoother).

### Gate (dynamics/gate.rs)

**Algorithm:** Noise gate with hysteresis and hold timer.

```
input → [sidechain HPF → LPF] → [peak/RMS detect] → [open/close with hysteresis]
  ↓                                                            ↓
  └──→ [gain envelope: attack/hold/release] ← ─────────────────┘
  ↓              ↓
  └──→ [input × gain] → output
```

**Parameters (10):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | threshold_db | -80..0 | -40 |
| 1 | range_db | -80..0 | -80 |
| 2 | attack_ms | 0.01..100 | 0.5 |
| 3 | hold_ms | 0..500 | 50 |
| 4 | release_ms | 5..2000 | 200 |
| 5 | hysteresis_db | 0..20 | 3 |
| 6 | sidechain_hpf | 20..2000 | 20 |
| 7 | sidechain_lpf | 200..20000 | 20000 |
| 8 | detection_mode | 0/1 | 0 |
| 9 | bypass | 0/1 | 0 |

**Hysteresis:** Open threshold = threshold_db. Close threshold = threshold_db - hysteresis_db. Prevents rapid on/off chattering.

**Hold timer:** After signal drops below close threshold, gate stays open for hold_ms before starting release. Prevents premature cutoff of note tails.

**Smoothed params:** threshold_db, range_db.

Reuses: `EnvelopeFollower` (from compressor), `Biquad` (from EQ, for sidechain filters).

### De-esser (dynamics/deesser.rs)

**Algorithm:** Split-band sibilance reduction.

```
input → [crossover: band = BPF at frequency/Q] → [detect band level]
  ↓                                                       ↓
  ├──[wideband mode: input × gain_reduction]──→ output     │
  └──[split mode: band × GR + rest passthrough]──→ output  │
                                                           ↓
                                              [threshold compare → GR]
```

**Parameters (7):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | threshold_db | -40..0 | -20 |
| 1 | frequency | 2000..12000 | 6000 |
| 2 | bandwidth_q | 0.5..8 | 2.0 |
| 3 | ratio | 1..20 | 4.0 |
| 4 | mode | 0/1 | 1 |
| 5 | listen_mode | 0/1 | 0 |
| 6 | bypass | 0/1 | 0 |

**Mode 0 (Wideband):** Gain reduction applied to full signal when sibilance detected.
**Mode 1 (Split-band):** Only the detected frequency band is compressed; everything else passes through untouched.
**Listen mode:** Outputs the sidechain signal (the isolated sibilance band) so the user can tune frequency/Q by ear.

**Smoothed params:** threshold_db, frequency, bandwidth_q.

Reuses: `Biquad` (bandpass for detection, crossover for split), `EnvelopeFollower`.

---

## New Effects — Modulation

### Delay (modulation/delay.rs)

**Algorithm:** Stereo delay with tempo sync, ping-pong, and filtered feedback.

```
input_L → [delay_line_L] → [feedback_filter LP+HP] → [feedback × coeff] → back to delay_L
input_R → [delay_line_R] → [feedback_filter LP+HP] → [feedback × coeff] → back to delay_R
                                     ↓ (ping-pong: cross-feed L↔R)
                           [dry/wet mix] → output
```

**Parameters (12):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | time_left_ms | 1..5000 | 500 |
| 1 | time_right_ms | 1..5000 | 500 |
| 2 | sync_mode | 0/1 | 0 |
| 3 | sync_note_left | 0..16 | 4 |
| 4 | sync_note_right | 0..16 | 4 |
| 5 | bpm | 20..300 | 120 |
| 6 | feedback | 0..0.95 | 0.3 |
| 7 | ping_pong | 0/1 | 0 |
| 8 | filter_lp | 200..20000 | 8000 |
| 9 | filter_hp | 20..2000 | 20 |
| 10 | dry_wet | 0..1 | 0.3 |
| 11 | bypass | 0/1 | 0 |

**Sync note values** (shared `NoteValue` enum, index 0-16):
```
0=1/32, 1=1/16T, 2=1/16, 3=1/16., 4=1/8T, 5=1/8, 6=1/8.,
7=1/4T, 8=1/4, 9=1/4., 10=1/2T, 11=1/2, 12=1/2.,
13=1/1T, 14=1/1, 15=2/1, 16=4/1
```

When `sync_mode=1`: `delay_ms = NoteValue::to_ms(bpm)`, ignoring `time_*_ms`.

**Feedback cap at 0.95** to prevent infinite self-oscillation.

**Smoothed params:** time_left_ms, time_right_ms, feedback, filter_lp, filter_hp, dry_wet.

**Denormal protection:** Applied in feedback path before writing back to delay line.

Reuses: `FractionalDelayLine`, `Biquad` (for feedback LP/HP).

### Chorus (modulation/chorus.rs)

**Algorithm:** 4-voice modulated delay with sinc interpolation.

```
input → [voice 1: delay + LFO₁ (0°)]   → ┐
      → [voice 2: delay + LFO₂ (90°)]  → ┤ gain-scaled mix
      → [voice 3: delay + LFO₃ (180°)] → ┤     → [high_cut] → [dry/wet] → output
      → [voice 4: delay + LFO₄ (270°)] → ┘
```

Each voice has its own `FractionalDelayLine`. LFO phases are evenly distributed: `phase_offset = voice_index / num_voices`.

**Parameters (8):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | rate_hz | 0.05..5.0 | 0.8 |
| 1 | depth | 0..1 | 0.5 |
| 2 | delay_ms | 5..30 | 12 |
| 3 | voices | 1..4 | 4 |
| 4 | stereo_spread | 0..1 | 0.7 |
| 5 | high_cut | 200..20000 | 12000 |
| 6 | dry_wet | 0..1 | 0.5 |
| 7 | bypass | 0/1 | 0 |

**Stereo spread:** Distributes voices across the stereo field. 0 = all center, 1 = alternating hard L/R.

**depth semantics:** Modulation excursion = `depth * delay_ms * 0.5`. At depth=1, delay_ms=12ms, each voice sweeps ±6ms around the center.

**Smoothed params:** rate_hz, depth, delay_ms, dry_wet, high_cut.

### Flanger (modulation/flanger.rs)

**Algorithm:** Single delay line + feedback + through-zero + soft saturation.

```
input → [delay_line + LFO] → ┐
                               ├→ [dry/wet mix] → output
input (dry) ──────────────→ ┘
                    ↑
         [soft_saturate(tanh)] ← [feedback × coeff] ← delayed output
         [polarity flip when feedback < 0: through-zero]
```

**Parameters (11):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | rate_hz | 0.05..10 | 0.5 |
| 1 | depth | 0..1 | 0.7 |
| 2 | delay_ms | 0.1..10 | 2.0 |
| 3 | feedback | -0.95..0.95 | 0.5 |
| 4 | stereo_phase | 0..180 | 90 |
| 5 | lfo_shape | 0..4 | 0 |
| 6 | dry_wet | 0..1 | 0.5 |
| 7 | bypass | 0/1 | 0 |
| 8 | sync_mode | 0/1 | 0 |
| 9 | sync_note | 0..16 | 4 |
| 10 | bpm | 20..300 | 120 |

**bpm** is used when sync_mode=1. Same pattern as Delay.

**Negative feedback** = through-zero mode. Signal polarity flips when LFO crosses zero, producing the classic "jet sweep" sound.

**Soft saturation** in feedback path: `tanh(feedback_sample)`. Internal quality measure — not exposed as a parameter. Prevents harsh metallic artifacts at high feedback values.

**Smoothed params:** rate_hz, depth, delay_ms, feedback, dry_wet.

**Denormal protection:** Applied in feedback path.

### Phaser (modulation/phaser.rs)

**Algorithm:** N-stage allpass cascade with LFO-modulated center frequencies.

```
input → [allpass₁(f)] → [allpass₂(f)] → ... → [allpassₙ(f)] → ┐
  ↑                           ↑ LFO sweeps f between min_freq..max_freq
  └── [feedback × coeff] ←────────────────────────────────────────┘
  ↓
  [dry + wet] → output
```

Each allpass stage is a first-order allpass: `y = -a*x + z; z = a*y + x` where `a = (tan(pi*f/sr) - 1) / (tan(pi*f/sr) + 1)`.

More stages = more notches in frequency response. 4 stages = 2 notches, 8 stages = 4 notches, 12 stages = 6 notches.

**Parameters (11):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | rate_hz | 0.05..10 | 0.4 |
| 1 | depth | 0..1 | 0.6 |
| 2 | stages | 2/4/6/8/12 | 4 |
| 3 | feedback | -0.95..0.95 | 0.3 |
| 4 | min_freq | 20..5000 | 100 |
| 5 | max_freq | 200..20000 | 5000 |
| 6 | stereo_phase | 0..180 | 90 |
| 7 | sync_mode | 0/1 | 0 |
| 8 | sync_note | 0..16 | 4 |
| 9 | bpm | 20..300 | 120 |
| 10 | bypass | 0/1 | 0 |

**bpm** is used when sync_mode=1. Same pattern as Delay.

**LFO sweep:** Frequency computed as `freq = min_freq * (max_freq/min_freq)^lfo_value` (exponential mapping for perceptually uniform sweep).

**Negative feedback:** Creates different notch patterns (inverted comb). Standard in high-end phasers.

**Smoothed params:** rate_hz, depth, min_freq, max_freq, feedback.

**Denormal protection:** Applied in feedback path.

### Tremolo (modulation/tremolo.rs)

**Algorithm:** Amplitude modulation.

```
input × [1 - depth + depth * (lfo_value * 0.5 + 0.5)] → output
```

LFO output (-1..1) is mapped to gain (0..1). At depth=0, gain=1 (no effect). At depth=1, gain swings from 0 to 1.

**Parameters (8):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | rate_hz | 0.1..20 | 4.0 |
| 1 | depth | 0..1 | 0.5 |
| 2 | lfo_shape | 0..4 | 0 |
| 3 | stereo_mode | 0/1 | 0 |
| 4 | sync_mode | 0/1 | 0 |
| 5 | sync_note | 0..16 | 4 |
| 6 | bpm | 20..300 | 120 |
| 7 | bypass | 0/1 | 0 |

**bpm** is used when sync_mode=1. Same pattern as Delay.

**Stereo mode 0 (Mono):** Same LFO applied to both channels.
**Stereo mode 1 (Auto-pan):** L and R use opposite LFO phase (180° offset). Creates panning effect.

**Smoothed params:** rate_hz, depth.

---

## New Effects — Utility

### Gain (utility/gain.rs)

**Algorithm:** Pure gain + polarity + mono sum.

```
input × (gain_linear × polarity) → [mono: (L+R)/2 if enabled] → output
```

**Parameters (4):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | gain_db | -inf..24 | 0.0 |
| 1 | polarity | 0/1 | 0 |
| 2 | mono | 0/1 | 0 |
| 3 | bypass | 0/1 | 0 |

**gain_db = -inf** means silence (linear gain = 0). In practice, clamp at -120 dB.

**Smoothed params:** gain_db (via ParamSmoother).

### Stereo Width (utility/stereo_width.rs)

**Algorithm:** Mid/Side encoding + independent gain + decode.

```
input L,R → encode: M = (L+R)/2, S = (L-R)/2
          → M × mid_gain, S × (side_gain × width)
          → decode: L = M+S, R = M-S
          → output
```

**Parameters (4):**

| ID | Name | Range | Default |
|----|------|-------|---------|
| 0 | width | 0..2 | 1.0 |
| 1 | mid_gain_db | -24..24 | 0 |
| 2 | side_gain_db | -24..24 | 0 |
| 3 | bypass | 0/1 | 0 |

**width semantics:** 0 = mono (S=0), 1 = original stereo, 2 = exaggerated width (S doubled).

**Smoothed params:** width, mid_gain_db, side_gain_db.

---

## Testing Strategy

### Pattern

Every effect follows the same test structure (matching existing compressor/reverb/EQ tests):

1. **bypass_is_bitexact** — bypass mode copies input to output with zero modification
2. **param_round_trip** — set/get all parameters, verify values match
3. **param_info_complete** — param_count matches actual params, all have valid info
4. **latency_correct** — latency() returns expected value (0 for IIR effects, lookahead for limiter)
5. **effect-specific behavior tests** — at least 3 per effect, testing core DSP logic

### Per-Effect Test Requirements

| Effect | Specific Tests |
|--------|---------------|
| Limiter | peak never exceeds ceiling, lookahead delay correct, auto-release adapts to crest factor |
| Gate | below-threshold attenuation matches range_db, hold timer prevents chattering, hysteresis works |
| De-esser | sibilance band attenuated while non-sibilant passes through, listen mode outputs sidechain only |
| Delay | delay time matches ms/sync setting, feedback decays correctly, ping-pong alternates channels |
| Chorus | multiple voices produce thicker output than single, stereo spread distributes L/R |
| Flanger | through-zero (negative feedback) produces polarity flip, soft saturation bounds output |
| Phaser | more stages produce more notches (verify via FFT), feedback deepens notches |
| Tremolo | depth=0 is passthrough, depth=1 reaches silence, stereo mode produces L/R opposition |
| Gain | gain_db maps correctly to linear, polarity inverts sign, mono sums channels |
| Stereo Width | width=0 produces mono, width=1 is passthrough, mid/side gains are independent |

### Shared Infrastructure Tests

| Module | Tests |
|--------|-------|
| ParamSmoother | ramp timing matches specified ms, settled detection works, reset jumps immediately |
| denormal | values below threshold become zero, normal values pass through |
| LFO | each waveform shape produces correct output, tempo sync matches BPM, S&H holds between triggers |
| FractionalDelayLine | integer delay is bit-exact, fractional delay interpolates correctly, sinc quality exceeds linear |

### Test Non-Negotiables

- No test weakening — existing 52 effect tests must continue to pass unchanged
- All new effects must have bypass_is_bitexact and param_round_trip tests
- Sinc interpolation quality: verify SNR > 120dB for integer delays, > 96dB for fractional delays

---

## Implementation Order

Bottom-up by dependency:

```
Phase 1 — Shared infrastructure (no new effects yet)
  (1) common/param_smoother.rs + tests
  (2) common/denormal.rs + tests
  (3) modulation/lfo.rs + tests
  (4) modulation/delay_line.rs + tests (sinc table + fractional read)

Phase 2 — Dynamics (reuse existing envelope/biquad)
  (5) dynamics/limiter.rs + tests
  (6) dynamics/gate.rs + tests
  (7) dynamics/deesser.rs + tests

Phase 3 — Modulation (depend on LFO + delay_line)
  (8) modulation/delay.rs + tests
  (9) modulation/chorus.rs + tests
  (10) modulation/flanger.rs + tests
  (11) modulation/phaser.rs + tests
  (12) modulation/tremolo.rs + tests

Phase 4 — Utility (simplest, no dependencies beyond core)
  (13) utility/gain.rs + tests
  (14) utility/stereo_width.rs + tests

Phase 5 — Integration
  (15) Update lib.rs, Cargo.toml feature flags, re-exports
  (16) Update moonlitt-node effects.rs (add factory functions)
  (17) Update moonlitt-capi builtin_api.rs (add C functions)
  (18) Update CLAUDE.md
```

Each step ends with `cargo test --workspace -- --skip pianoteq --skip keyscape` passing.
