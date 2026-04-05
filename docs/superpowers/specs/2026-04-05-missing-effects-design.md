# Missing Effects Design Spec

**Date:** 2026-04-05
**Status:** Draft
**Scope:** 5 new effects — saturator, bitcrusher, multiband compressor, auto-filter, pitch shifter

## Motivation

The effects suite now has dynamics (compressor, limiter, gate, de-esser), modulation (delay, chorus, flanger, phaser, tremolo), spatial (reverb, convolver), and utility (gain, stereo width). Missing are distortion/creative effects and advanced dynamics. These 5 effects complete the DAW-standard set.

## Module Structure

```
distortion/                     NEW CATEGORY
  ├── mod.rs
  ├── saturator.rs              5 saturation models + oversampling
  └── bitcrusher.rs             sample rate + bit depth reduction

dynamics/
  └── multiband_compressor.rs   NEW: 1-6 band Linkwitz-Riley crossover + per-band compression

modulation/
  ├── auto_filter.rs            NEW: envelope/LFO → resonant filter
  └── pitch_shifter.rs          NEW: granular + phase vocoder dual mode
```

## Feature Flags

```toml
# New individual flags
saturator = []
bitcrusher = []
multiband-compressor = []
auto-filter = []
pitch-shifter = ["dep:rustfft"]

# New category
distortion = ["saturator", "bitcrusher"]

# Updated categories
dynamics = ["compressor", "limiter", "gate", "deesser", "multiband-compressor"]
modulation = ["delay", "chorus", "flanger", "phaser", "tremolo", "auto-filter", "pitch-shifter"]
all = ["dynamics", "eq", "spatial", "modulation", "utility", "distortion"]
```

`pitch-shifter` requires `rustfft` for the phase vocoder mode (shared with convolver).

---

## Saturator (distortion/saturator.rs)

**Algorithm:** Nonlinear waveshaping with oversampling anti-aliasing.

```
input → [oversampler ↑2x/4x] → [input_gain] → [asymmetry bias] → [waveshaper(mode)]
      → [tone filter] → [output_gain] → [oversampler ↓] → [high_cut] → [dry/wet] → output
```

### 5 Saturation Models

| Mode | Function | Character |
|------|----------|-----------|
| Tube | `x / (1 + abs(x))` | Even harmonics, warm, vintage |
| Tape | `tanh(x) * (1 + 0.5 * exp(-x*x))` + 1-pole LP | Soft compression, natural HF rolloff |
| Transistor | `tanh(x)` | Odd harmonics, symmetric, modern |
| Diode | `sign(x) * (1 - exp(-abs(x)))` | Asymmetric, half-wave harder |
| Fuzz | `sign(x) * (1 - exp(-3 * x * x))` + BPF | Hard clip, narrow band, aggressive |

**Asymmetry:** Before waveshaping, add DC bias: `x_biased = x + asymmetry * abs(x)`. This creates even-order harmonics from any symmetric waveshaper. A DC blocker (1-pole HPF at 5Hz) removes the resulting DC offset after waveshaping.

**Oversampling:** Uses `Oversampler` from `common/oversampler.rs`. Waveshaping generates harmonics that can alias. 2x handles moderate drive, 4x handles extreme drive.

**Tone:** Post-waveshape tilt EQ. 0 = dark (LP emphasis), 0.5 = neutral, 1 = bright (HP emphasis). Implemented as crossfade between LP and HP one-pole filters.

### Parameters (9)

| ID | Name | Range | Default | Display |
|----|------|-------|---------|---------|
| 0 | drive_db | 0..48 | 12 | "{:.1} dB" |
| 1 | mode | 0..4 | 0 | "Tube"/"Tape"/"Transistor"/"Diode"/"Fuzz" (stepped, step_count=4) |
| 2 | tone | 0..1 | 0.5 | "{:.0}%" (×100) |
| 3 | output_db | -24..24 | 0 | "{:.1} dB" |
| 4 | oversampling | 0..2 | 1 | "1x"/"2x"/"4x" (stepped, step_count=2) |
| 5 | asymmetry | -1..1 | 0 | "{:.0}%" (×100) |
| 6 | mix | 0..1 | 1.0 | "{:.0}%" (×100) |
| 7 | high_cut | 200..20000 | 20000 | "{:.0} Hz" |
| 8 | bypass | 0/1 | 0 | "Off"/"On" (stepped) |

**Smoothed params:** drive_db, output_db, tone, mix, asymmetry.

**Reuses:** `Oversampler`, `ParamSmoother`, `Biquad` (for tone/high_cut/DC blocker).

---

## Bitcrusher (distortion/bitcrusher.rs)

**Algorithm:** Sample rate reduction + bit depth reduction.

```
input → [sample & hold at reduced rate] → [TPDF dither] → [quantize to N bits] → [dry/wet] → output
```

**Sample rate reduction:** Sample & hold — every `rate_reduction` samples, hold the current value. Non-integer values use the counter approach: accumulate `1/rate_reduction` per sample, trigger new sample when accumulator >= 1.

**Bit depth reduction:** `quantized = floor(sample × 2^(bits-1) + 0.5) / 2^(bits-1)`. At 24 bits, effect is inaudible. At 1 bit, output is ±1 (square wave).

**Dither:** Optional TPDF dither injected before quantization. Reduces harmonic distortion from quantization at the cost of noise floor. Amount 0..1 controls dither amplitude relative to one quantization step.

**Jitter:** Random clock jitter on the sample & hold timing. Adds analog imperfection character. xorshift64 PRNG.

### Parameters (6)

| ID | Name | Range | Default | Display |
|----|------|-------|---------|---------|
| 0 | bit_depth | 1..24 | 8 | "{} bit" (stepped, step_count=23) |
| 1 | rate_reduction | 1..100 | 1 | "{:.1}x" |
| 2 | dither | 0..1 | 0.5 | "{:.0}%" (×100) |
| 3 | dry_wet | 0..1 | 1.0 | "{:.0}%" (×100) |
| 4 | jitter | 0..1 | 0 | "{:.0}%" (×100) |
| 5 | bypass | 0/1 | 0 | "Off"/"On" (stepped) |

**Smoothed params:** dry_wet.

---

## Multiband Compressor (dynamics/multiband_compressor.rs)

**Algorithm:** Linkwitz-Riley crossover → per-band compression → sum.

```
input → [LR4 crossover @ freq_1] → band_1 → [compress_1] → ┐
                                 → rest    → [LR4 @ freq_2] → band_2 → [compress_2] → ┤
                                                             → rest   → [LR4 @ freq_3] → band_3 → [compress_3] → ┤ sum → [output_gain] → output
                                                                                       → ...                      ┘
```

### Linkwitz-Riley 4th Order Crossover

Two cascaded Butterworth 2nd-order filters (LP+HP). LR4 has these properties:
- -6 dB at crossover point (both LP and HP)
- LP + HP sum = unity (flat magnitude response)
- -24 dB/octave slope (steep, minimal overlap)

Each crossover splits the signal into low and high. For N bands, need N-1 crossovers applied cascaded.

### Per-Band Compression

Each band has its own independent compressor with: threshold, ratio, attack, release, makeup gain. Reuses the gain computation from `Compressor::compute_gain_db_static()` and `EnvelopeFollower`.

### Variable Band Count (1-6)

- 1 band = fullband compression (no crossover, degenerates to normal compressor)
- 4 bands = 3 crossover points (default)
- 6 bands = 5 crossover points (maximum)

Crossover frequencies must be in ascending order. Enforce: `crossover[i] < crossover[i+1]` with minimum gap of 1 octave.

### Parameters (38)

**Global (3):**

| ID | Name | Range | Default | Display |
|----|------|-------|---------|---------|
| 0 | band_count | 1..6 | 4 | "{}" (stepped, step_count=5) |
| 1 | output_db | -24..24 | 0 | "{:.1} dB" |
| 2 | bypass | 0/1 | 0 | "Off"/"On" (stepped) |

**Crossover frequencies (5), IDs 3-7:**

| ID | Name | Range | Default | Display |
|----|------|-------|---------|---------|
| 3 | crossover_1 | 20..20000 | 100 | "{:.0} Hz" |
| 4 | crossover_2 | 20..20000 | 500 | "{:.0} Hz" |
| 5 | crossover_3 | 20..20000 | 2000 | "{:.0} Hz" |
| 6 | crossover_4 | 20..20000 | 8000 | "{:.0} Hz" |
| 7 | crossover_5 | 20..20000 | 16000 | "{:.0} Hz" |

Only `band_count - 1` crossovers are active.

**Per-band (6 bands × 5 params = 30), IDs 8-37:**

Band N (0-indexed) parameters start at ID `8 + N*5`:

| Offset | Name | Range | Default | Display |
|--------|------|-------|---------|---------|
| +0 | threshold_db | -60..0 | -20 | "{:.1} dB" |
| +1 | ratio | 1..100 | 4.0 | "{:.1}:1" |
| +2 | attack_ms | 0.1..100 | 10 | "{:.1} ms" |
| +3 | release_ms | 10..1000 | 100 | "{:.0} ms" |
| +4 | makeup_db | -12..24 | 0 | "{:.1} dB" |

**Total: 38 parameters.**

**Smoothed params:** crossover frequencies, output_db, per-band threshold and makeup.

**Reuses:** `EnvelopeFollower`, `Biquad`/`BiquadCoeffs` (for LR4 crossover), `ParamSmoother`, `Compressor::compute_gain_db_static` (or inline equivalent).

**latency():** 0 (IIR crossover filters, no latency).

---

## Auto-Filter (modulation/auto_filter.rs)

**Algorithm:** Envelope follower or LFO modulates a resonant filter's cutoff frequency.

```
source=Envelope:
  input → [envelope follower] → modulation signal (0..1)
  
source=LFO:
  [LFO] → modulation signal (-1..1, mapped to 0..1)

modulation → [exponential freq mapping] → filter_freq
input → [resonant biquad filter at filter_freq] → [dry/wet] → output
```

### Envelope Mode

`EnvelopeFollower` tracks input amplitude. Sensitivity scales the envelope value. Higher sensitivity = more responsive to dynamics.

### LFO Mode

Uses shared `Lfo` from modulation. All 5 waveform shapes. No tempo sync in this effect (auto-filter is typically free-running).

### Frequency Mapping

Exponential: `freq = min_freq × (max_freq/min_freq)^(modulation × sensitivity)`. Same as phaser. Perceptually uniform sweep.

### Filter

Biquad with per-sample coefficient update (frequency changes every sample due to modulation). Filter types: Lowpass (0), Highpass (1), Bandpass (2).

High resonance (Q > 10) creates a sharp peak — the classic wah/auto-wah sound.

### Parameters (12)

| ID | Name | Range | Default | Display |
|----|------|-------|---------|---------|
| 0 | source | 0/1 | 0 | "Envelope"/"LFO" (stepped) |
| 1 | filter_type | 0..2 | 0 | "LP"/"HP"/"BP" (stepped, step_count=2) |
| 2 | min_freq | 20..5000 | 100 | "{:.0} Hz" |
| 3 | max_freq | 200..20000 | 5000 | "{:.0} Hz" |
| 4 | resonance | 0.5..20 | 2.0 | "{:.1}" |
| 5 | sensitivity | 0..1 | 0.5 | "{:.0}%" (×100) |
| 6 | attack_ms | 0.1..100 | 5 | "{:.1} ms" |
| 7 | release_ms | 5..1000 | 50 | "{:.0} ms" |
| 8 | lfo_rate | 0.05..20 | 1.0 | "{:.2} Hz" |
| 9 | lfo_shape | 0..4 | 0 | "Sine"/"Tri"/"Saw"/"Sq"/"S&H" (stepped, step_count=4) |
| 10 | dry_wet | 0..1 | 1.0 | "{:.0}%" (×100) |
| 11 | bypass | 0/1 | 0 | "Off"/"On" (stepped) |

**Smoothed params:** min_freq, max_freq, resonance, dry_wet.

**Reuses:** `EnvelopeFollower`, `Lfo`, `Biquad`/`BiquadCoeffs`, `ParamSmoother`.

---

## Pitch Shifter (modulation/pitch_shifter.rs)

Dual-mode pitch shifter: Granular for creative/large shifts, Phase Vocoder for transparent/small shifts.

### Granular Mode

```
input → [circular buffer] → [extract overlapping grains with Hann window]
      → [resample grains at playback_rate] → [overlap-add] → output
```

- **Grain size:** 5-50ms (user parameter)
- **Overlap factor:** 4 (grains overlap by 75%)
- **Playback rate:** `2^(semitones/12)` — faster = higher pitch
- **Randomization:** ±20% random grain position offset to prevent periodicity artifacts
- **Window:** Hann window for smooth grain boundaries

### Phase Vocoder Mode

```
input → [STFT analysis (FFT + windowing)]
      → [phase accumulation + frequency estimation]
      → [frequency shifting (bin resampling)]
      → [ISTFT synthesis (IFFT + overlap-add)]
      → output
```

- **FFT sizes:** 1024, 2048, 4096 (parameter selectable)
- **Overlap:** 4x (hop = fft_size/4)
- **Analysis window:** Hann
- **Phase processing:**
  1. Compute instantaneous frequency per bin from phase difference
  2. Shift frequencies by the pitch ratio
  3. Accumulate synthesis phase from shifted frequencies
- **Bin resampling:** For pitch ratio `r`, bin `k` in output reads from bin `k/r` in input (with linear interpolation between bins)

### Formant Preservation (Phase Vocoder only)

Without formant preservation, shifting up makes vocals sound like chipmunks. The algorithm:
1. Estimate spectral envelope (peak interpolation or cepstral method)
2. Shift the fine structure (harmonics) by the pitch ratio
3. Reapply the original spectral envelope

Simple approach: LPC-based envelope estimation (8-12 coefficients). Apply envelope correction as a gain curve per FFT bin.

For this first implementation, use the simpler approach: spectral envelope via peak picking + interpolation. Full LPC can be added later.

### Parameters (8)

| ID | Name | Range | Default | Display |
|----|------|-------|---------|---------|
| 0 | semitones | -24..24 | 0 | "{:+}" (stepped, step_count=48) |
| 1 | cents | -100..100 | 0 | "{:+} ct" |
| 2 | mode | 0/1 | 0 | "Granular"/"Vocoder" (stepped) |
| 3 | grain_size_ms | 5..50 | 20 | "{:.0} ms" (granular only) |
| 4 | fft_size | 0..2 | 1 | "1024"/"2048"/"4096" (stepped, step_count=2, vocoder only) |
| 5 | dry_wet | 0..1 | 1.0 | "{:.0}%" (×100) |
| 6 | formant_preserve | 0/1 | 0 | "Off"/"On" (stepped, vocoder only) |
| 7 | bypass | 0/1 | 0 | "Off"/"On" (stepped) |

**Total shift:** `semitones + cents/100.0` semitones. Range: -24.99 to +24.99.

**latency():**
- Granular: `(grain_size_ms * sample_rate / 1000 / 2) as u32`
- Phase Vocoder: `fft_size_samples / 2`

Reports via AudioBackend::latency() for PDC.

**Smoothed params:** dry_wet.

**Dependencies:** `rustfft` (for phase vocoder FFT/IFFT).

---

## Testing Strategy

### Per-Effect Tests

| Effect | Tests |
|--------|-------|
| Saturator | bypass_bitexact, param_round_trip, drive_zero_near_unity, each_mode_produces_different_output, oversampling_reduces_aliasing, asymmetry_creates_even_harmonics |
| Bitcrusher | bypass_bitexact, param_round_trip, bit_depth_24_near_passthrough, rate_reduction_1_passthrough, low_bits_quantizes_audibly |
| Multiband Comp | bypass_bitexact, param_round_trip, crossover_sum_is_flat (verify LP+HP=unity), single_band_equals_fullband, per_band_independent_compression |
| Auto-filter | bypass_bitexact, param_round_trip, envelope_responds_to_amplitude, lfo_sweeps_periodically, high_resonance_creates_peak |
| Pitch Shifter | bypass_bitexact, param_round_trip, semitones_zero_near_passthrough, plus_12_doubles_frequency, granular_and_vocoder_differ |

### Test Non-Negotiables

- Existing 151 effect tests must pass unchanged
- All new effects have bypass_bitexact and param_round_trip
- Multiband crossover flatness: deviation < 0.5 dB across 20Hz-20kHz

---

## Implementation Order

```
Phase 1 — Distortion category
  (1) distortion/mod.rs + saturator.rs + tests
  (2) distortion/bitcrusher.rs + tests

Phase 2 — Advanced dynamics
  (3) dynamics/multiband_compressor.rs + tests

Phase 3 — Advanced modulation
  (4) modulation/auto_filter.rs + tests
  (5) modulation/pitch_shifter.rs + tests (most complex)

Phase 4 — Integration
  (6) Update Cargo.toml feature flags
  (7) Update lib.rs re-exports + mod.rs files
  (8) Update moonlitt-node + moonlitt-capi bindings
  (9) Update CLAUDE.md
```
