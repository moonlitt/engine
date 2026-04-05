# Test Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 64 DSP compliance tests across 10 new test files in moonlitt-test-suite for the 16 new effects + oversampler.

**Architecture:** Each test file is self-contained (no shared helpers module — follows existing test-suite convention). Each file includes its own helper functions. Tests verify DSP precision against established standards (AES17, ITU-R BS.1770) with explicit numeric tolerances.

**Tech Stack:** Rust, rustfft (FFT analysis), moonlitt-effects, moonlitt-core (AudioBackend trait)

**Spec:** `docs/superpowers/specs/2026-04-05-test-hardening-design.md`

**Baseline:** `cargo test --workspace -- --skip pianoteq --skip keyscape` — all tests pass.

**Key pattern:** Every test file follows this structure:
```rust
use moonlitt_effects::EffectName;
use moonlitt_core::AudioBackend;
use std::f64::consts::PI;
const SR: u32 = 44100;

// Inline helpers: sine_f32(), rms(), power_spectrum(), etc.
// Tests: #[test] fn l1_test_name() { ... }
```

**Effect constructors:** All use `EffectName::new(SR)`. Parameters set via `effect.set_param(id, value)`. Processing via `effect.process_effect(&in_l, &in_r, &mut out_l, &mut out_r)`.

---

## Task 1: Limiter Compliance (8 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/limiter_compliance.rs`

The implementer should create this file with 8 tests verifying the limiter's DSP precision. Read the spec section "limiter_compliance.rs" for exact test descriptions.

**Common helpers needed in this file:**
- `sine_f32(freq, amplitude, num_samples)` — generate sine wave
- `rms(buf)` — compute RMS
- `rms_dbfs(buf)` — RMS in dBFS

**Tests to write (from spec):**

1. `l1_true_peak_never_exceeds_ceiling` — Feed signal creating inter-sample peaks. At 2x oversampling, output true peak must ≤ ceiling + 0.01.
2. `l2_attack_timing_precision` — Step from silence to +6dB. Measure samples to 63.2% of target GR. Should match attack_ms ±10%.
3. `l3_release_timing_precision` — From limiting to silence. Measure samples to 63.2% recovery. Should match release_ms ±10%.
4. `l4_auto_release_adapts` — Sparse transients vs sustained signal. Auto-release should produce different behavior.
5. `l5_lookahead_latency_matches` — Impulse delay measurement must equal `limiter.latency()` ±1 sample.
6. `l6_ceiling_hard_clip` — Feed extreme amplitude (10.0), every output sample must be ≤ ceiling_linear.
7. `l7_oversampling_reduces_intersample_overshoot` — 1x vs 2x: 2x should have better true peak control.
8. `l8_bypass_thd` — Bypass mode is bit-exact (input == output).

**Implementation guidance:**
- `Limiter::new(SR)`, params: 0=threshold, 1=ceiling, 2=release_ms, 3=lookahead_ms, 4=attack_ms, 5=oversampling(0=1x,1=2x,2=4x), 6=auto_release, 7=bypass
- For timing tests: feed a block, scan output for the sample where gain crosses the 63.2% threshold
- For true peak test: create signal `[0.9, -0.9, 0.9, -0.9...]` which creates ~1.0 inter-sample peaks
- Process multiple blocks (10+) to let the limiter settle before measuring

- [ ] **Step 1: Create limiter_compliance.rs with all 8 tests**
- [ ] **Step 2: Run tests**
```bash
cargo test -p moonlitt-test-suite --test limiter_compliance 2>&1
```
Expected: 8 tests pass. If any fail, fix the test (adjust tolerance) ONLY if the effect's behavior is correct but the tolerance is too tight. Never weaken a test to mask a real bug — if the effect is wrong, report it.

- [ ] **Step 3: Run workspace tests for regression**
```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```

- [ ] **Step 4: Commit**
```bash
git add crates/moonlitt-test-suite/tests/limiter_compliance.rs
git commit -m "test: add limiter compliance tests (8 tests)

Timing precision, true peak, ceiling clip, lookahead latency,
auto-release adaptation, oversampling alias rejection."
```

---

## Task 2: Gate Compliance (7 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/gate_compliance.rs`

**Tests (from spec):**

1. `g1_closed_attenuation_matches_range` — Below threshold, attenuation = range_db ±0.5dB.
2. `g2_open_is_unity` — Above threshold, gain = 0dB ±0.1dB.
3. `g3_attack_timing` — Gate opens: 63.2% time ≈ attack_ms ±15%.
4. `g4_hold_prevents_chattering` — Rapid level oscillation near threshold. With hold=100ms, transitions ≤ 10/sec. Without hold, >50/sec.
5. `g5_hysteresis_deadband` — Signal slowly crossing threshold. Open at threshold, close at threshold-hysteresis.
6. `g6_release_timing` — Gate closes: 63.2% time ≈ release_ms ±15%.
7. `g7_sidechain_filter_isolation` — HPF=500Hz: 200Hz doesn't trigger, 2kHz does.

**Implementation guidance:**
- `Gate::new(SR)`, params: 0=threshold, 1=range, 2=attack_ms, 3=hold_ms, 4=release_ms, 5=hysteresis, 6=sidechain_hpf, 7=sidechain_lpf, 8=detection_mode, 9=bypass
- For chattering test: create 50Hz amplitude-modulated signal (level crosses threshold rapidly)
- For hysteresis test: create linearly rising then falling ramp signal, detect gate open/close points
- Use sine waves (not DC) for gate detection — the sidechain HPF filters DC

- [ ] **Step 1: Create gate_compliance.rs with all 7 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add gate compliance tests (7 tests)

Attenuation precision, timing, hold, hysteresis, sidechain filter isolation."
```

---

## Task 3: De-esser Compliance (5 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/deesser_compliance.rs`

**Tests:**

1. `d1_sibilance_attenuation_ratio` — 6kHz sine above threshold, verify GR matches ratio ±2dB.
2. `d2_non_sibilant_passthrough_splitband` — 200Hz passes < 0.5dB deviation in split-band mode.
3. `d3_listen_mode_is_bandpass` — Listen mode output = bandpass-filtered input.
4. `d4_wideband_vs_splitband_low_freq` — Wideband attenuates more low freq than split-band.
5. `d5_frequency_tracking` — Changing frequency param shifts the attenuation band.

**Implementation guidance:**
- `DeEsser::new(SR)`, params: 0=threshold, 1=frequency, 2=bandwidth_q, 3=ratio, 4=mode(0=wideband,1=split), 5=listen_mode, 6=bypass
- For listen mode test: compare output spectrum with a manually-designed bandpass
- Process 5+ blocks to settle before measuring

- [ ] **Step 1: Create deesser_compliance.rs with all 5 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add de-esser compliance tests (5 tests)

Attenuation ratio, split-band passthrough, listen mode, frequency tracking."
```

---

## Task 4: Modulation Compliance (13 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/modulation_compliance.rs`

This is the largest test file — covers delay, chorus, flanger, phaser, tremolo.

**Delay tests (3):**
1. `m1_delay_time_sample_accuracy` — Impulse appears at correct sample ±1.
2. `m2_tempo_sync_precision` — 1/4 @ 120BPM = 22050 samples ±2.
3. `m3_feedback_decay_rate` — Each repeat = feedback × previous ±0.5dB.

**Chorus tests (2):**
4. `m4_chorus_no_aliasing` — Aliasing products < -80dB.
5. `m5_chorus_depth_zero_no_modulation` — Fixed delay, no frequency modulation.

**Flanger tests (3):**
6. `m6_flanger_comb_frequencies` — Null positions match expected comb pattern.
7. `m7_flanger_through_zero_polarity` — Negative feedback inverts DC polarity.
8. `m8_flanger_saturation_bounds` — High feedback stays bounded (< 4.0).

**Phaser tests (2):**
9. `m9_phaser_notch_count` — 4 stages = 2 notches, 8 stages = 4 notches.
10. `m10_phaser_sweep_range` — Notch frequencies stay within min..max.

**Tremolo tests (3):**
11. `m11_tremolo_depth_modulation_range` — depth=1: output swings 0..1.
12. `m12_tremolo_stereo_phase_opposition` — L/R envelopes are anti-correlated.
13. `m13_tremolo_tempo_sync_rate` — Sync 1/4 @ 120BPM = 2Hz.

**Implementation guidance:**

Effect constructors and param IDs:
- `StereoDelay::new(SR)`: 0=time_left, 1=time_right, 2=sync_mode, 3=sync_note_left, 4=sync_note_right, 5=bpm, 6=feedback, 7=ping_pong, 8=filter_lp, 9=filter_hp, 10=dry_wet, 11=bypass
- `Chorus::new(SR)`: 0=rate, 1=depth, 2=delay_ms, 3=voices, 4=stereo_spread, 5=high_cut, 6=dry_wet, 7=bypass
- `Flanger::new(SR)`: 0=rate, 1=depth, 2=delay_ms, 3=feedback, 4=stereo_phase, 5=lfo_shape, 6=dry_wet, 7=bypass, 8=sync_mode, 9=sync_note, 10=bpm
- `Phaser::new(SR)`: 0=rate, 1=depth, 2=stages, 3=feedback, 4=min_freq, 5=max_freq, 6=stereo_phase, 7=sync_mode, 8=sync_note, 9=bpm, 10=bypass
- `Tremolo::new(SR)`: 0=rate, 1=depth, 2=lfo_shape, 3=stereo_mode, 4=sync_mode, 5=sync_note, 6=bpm, 7=bypass

For delay tests: impulse response (1.0 at sample 0, zeros after). Scan output for peak position.
For phaser notch count: feed white noise, FFT, count dips > 10dB below neighbors.
For tremolo stereo test: measure L/R amplitude envelopes, compute cross-correlation.

Helpers needed: `sine_f32()`, `rms()`, `power_spectrum()`, `find_peak_frequency()`, `count_zero_crossings()`

- [ ] **Step 1: Create modulation_compliance.rs with all 13 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add modulation compliance tests (13 tests)

Delay timing/sync/feedback, chorus aliasing, flanger comb/saturation,
phaser notch count/sweep, tremolo depth/stereo/sync."
```

---

## Task 5: Distortion Compliance (7 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/distortion_compliance.rs`

**Saturator tests (5):**
1. `s1_tube_even_harmonics` — 2nd harmonic > 3rd by >6dB (FFT of 1kHz sine).
2. `s2_transistor_odd_harmonics` — 3rd harmonic > 2nd by >6dB.
3. `s3_oversampling_alias_rejection` — 2x has >30dB less aliasing than 1x at 10kHz.
4. `s4_asymmetry_dc_blocked` — asymmetry≠0, DC component < -60dB after blocker.
5. `s5_drive_zero_thd` — drive=0dB, THD < -40dB.

**Bitcrusher tests (2):**
6. `s6_bit_depth_quantization_noise` — 8-bit noise floor ≈ -48dBFS ±3dB.
7. `s7_rate_reduction_imaging` — rate_reduction=4: mirror components appear at sr/4±f.

**Implementation guidance:**
- `Saturator::new(SR)`: 0=drive_db, 1=mode(0-4), 2=tone, 3=output_db, 4=oversampling, 5=asymmetry, 6=mix, 7=high_cut, 8=bypass
- `Bitcrusher::new(SR)`: 0=bit_depth, 1=rate_reduction, 2=dither, 3=dry_wet, 4=jitter, 5=bypass
- For harmonic analysis: feed 1kHz sine, FFT, measure magnitudes at 2kHz and 3kHz bins
- For aliasing test: feed 10kHz sine with high drive, measure energy in 18-22kHz range

Helpers needed: `sine_f32()`, `power_spectrum()`, `rms_dbfs()`

- [ ] **Step 1: Create distortion_compliance.rs with all 7 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add distortion compliance tests (7 tests)

Saturator harmonic character, aliasing, DC blocking.
Bitcrusher quantization noise floor, rate reduction imaging."
```

---

## Task 6: Utility Compliance (5 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/utility_compliance.rs`

**Tests:**
1. `u1_gain_db_to_linear_precision` — +6.0206dB = 2.0×, -6.0206dB = 0.5×, 0dB = 1.0×.
2. `u2_polarity_invert_bitexact` — output[i] == -input[i] bit-exact.
3. `u3_mono_sum_energy` — Correlated: same RMS. Anti-correlated: silence.
4. `u4_stereo_width_zero_mono` — width=0: L == R bit-exact.
5. `u5_mid_side_orthogonality` — Mid signal unaffected by side_gain, side signal unaffected by mid_gain.

**Implementation guidance:**
- `Gain::new(SR)`: 0=gain_db, 1=polarity, 2=mono, 3=bypass
- `StereoWidth::new(SR)`: 0=width, 1=mid_gain_db, 2=side_gain_db, 3=bypass
- For bit-exact tests: compare using `to_bits()` (handles -0.0 vs 0.0)
- Gain precision: use exact values `10.0_f64.powf(6.0206/20.0)` ≈ 2.0

- [ ] **Step 1: Create utility_compliance.rs with all 5 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add utility compliance tests (5 tests)

Gain dB precision, polarity invert bitexact, mono energy,
stereo width mono, mid/side orthogonality."
```

---

## Task 7: Multiband Compliance (6 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/multiband_compliance.rs`

**Tests:**
1. `mb1_crossover_flatness` — No compression, sweep sine 20Hz-20kHz, deviation < 0.5dB.
2. `mb2_crossover_slope_24db_oct` — LP at 1kHz: -24dB at 2kHz. HP at 1kHz: -24dB at 500Hz.
3. `mb3_band_independence` — Compress only band 2, others unaffected < 1dB change.
4. `mb4_crossover_phase_alignment` — LP + HP at crossover = -6dB (LR4 property).
5. `mb5_single_band_degenerates` — band_count=1 ≈ standalone Compressor ±0.5dB.
6. `mb6_per_band_ratio_precision` — Each band's actual ratio matches setting ±0.5.

**Implementation guidance:**
- `MultibandCompressor::new(SR)`: 0=band_count, 1=output_db, 2=bypass, 3-7=crossover freqs, 8-37=per-band params (6 bands × 5 params starting at id 8+N*5: threshold, ratio, attack, release, makeup)
- For flatness test: sweep sine at many frequencies, measure output level at each
- For slope test: use single crossover, measure output at octave intervals
- For independence: set band 2 to heavy compression (-40dB threshold, 10:1 ratio), verify adjacent bands unchanged

Helpers needed: `sine_f32()`, `rms_dbfs()`, `power_spectrum()`

- [ ] **Step 1: Create multiband_compliance.rs with all 6 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add multiband compressor compliance tests (6 tests)

Crossover flatness, slope, band independence, phase alignment,
single-band degeneration, per-band ratio precision."
```

---

## Task 8: Auto-Filter Compliance (4 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/auto_filter_compliance.rs`

**Tests:**
1. `af1_envelope_frequency_tracking` — Loud input: filter opens. Quiet: closes. Measure -3dB cutoff difference.
2. `af2_resonance_peak` — Q=10 at 1kHz: >15dB peak at 1kHz vs passband.
3. `af3_filter_type_response` — LP: 4kHz attenuated vs 1kHz. HP: 1kHz attenuated vs 4kHz. BP: both.
4. `af4_lfo_sweep_period` — LFO 2Hz: 2 full sweeps per second (count peaks in amplitude envelope).

**Implementation guidance:**
- `AutoFilter::new(SR)`: 0=source(0=env,1=lfo), 1=filter_type(0=LP,1=HP,2=BP), 2=min_freq, 3=max_freq, 4=resonance, 5=sensitivity, 6=attack_ms, 7=release_ms, 8=lfo_rate, 9=lfo_shape, 10=dry_wet, 11=bypass
- For envelope tracking: feed alternating loud/quiet blocks, measure HF content in each
- For resonance peak: feed white noise (random samples), FFT, find peak

- [ ] **Step 1: Create auto_filter_compliance.rs with all 4 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add auto-filter compliance tests (4 tests)

Envelope frequency tracking, resonance peak, filter types, LFO period."
```

---

## Task 9: Pitch Shifter Compliance (5 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/pitch_shifter_compliance.rs`

**Tests:**
1. `ps1_granular_pitch_ratio` — +12 semitones on 440Hz → 880Hz ±5% (FFT peak).
2. `ps2_vocoder_pitch_ratio` — +12 semitones on 440Hz → 880Hz ±2%.
3. `ps3_zero_shift_passthrough` — shift=0: cross-correlation with input > 0.9.
4. `ps4_latency_matches_report` — Impulse delay matches `latency()` ±1 sample.
5. `ps5_granular_no_clicks` — Max sample-to-sample difference < 0.5 (no discontinuities).

**Implementation guidance:**
- `PitchShifter::new(SR)`: 0=semitones, 1=cents, 2=mode(0=granular,1=vocoder), 3=grain_size_ms, 4=fft_size(0=1024,1=2048,2=4096), 5=dry_wet, 6=formant_preserve, 7=bypass
- For frequency measurement: process 1 second of sine, FFT, find peak bin
- For cross-correlation: `sum(x[i]*y[i+d]) / sqrt(sum(x²) * sum(y²))`, search over delay d
- For click detection: `(out[i+1] - out[i]).abs().max()` over all samples

Helpers needed: `sine_f32()`, `find_peak_frequency()`, `power_spectrum()`

- [ ] **Step 1: Create pitch_shifter_compliance.rs with all 5 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add pitch shifter compliance tests (5 tests)

Granular/vocoder pitch ratio, zero-shift passthrough,
latency accuracy, granular click-free."
```

---

## Task 10: Oversampler Compliance (4 tests)

**Files:**
- Create: `crates/moonlitt-test-suite/tests/oversampler_compliance.rs`

**Tests:**
1. `os1_alias_rejection_96db` — Inject tone at 0.6×Nyquist in upsampled domain, downsample. Residual > 90dB attenuation.
2. `os2_passband_ripple` — Sweep 20Hz to 0.4×Nyquist through up→down. Deviation < 0.1dB.
3. `os3_phase_linearity` — Group delay from impulse response is constant ±0.5 samples.
4. `os4_cascade_equivalence` — factor=4 vs two sequential factor=2. Output matches < -120dB difference.

**Implementation guidance:**
- `Oversampler` is at `moonlitt_effects::common::oversampler::Oversampler`
- `Oversampler::new(factor, max_block_size)` — factor: 1/2/4/8
- `oversampler.process(input, output, |buf| { /* identity */ })` — for passthrough measurement
- For alias rejection: upsample silence, inject tone in the upsampled buffer inside the callback, downsample, measure
- For passband ripple: process many frequencies, compare output amplitude to input

Helpers needed: `sine_f32()`, `rms()`, `power_spectrum()`

- [ ] **Step 1: Create oversampler_compliance.rs with all 4 tests**
- [ ] **Step 2: Run and verify**
- [ ] **Step 3: Commit**
```bash
git commit -m "test: add oversampler compliance tests (4 tests)

Alias rejection, passband ripple, phase linearity, cascade equivalence."
```

---

## Task 11: Final Verification

- [ ] **Step 1: Run all test-suite tests**
```bash
cargo test -p moonlitt-test-suite 2>&1
```
Expected: 115 existing + 64 new = **179 tests**, all pass.

- [ ] **Step 2: Run full workspace tests**
```bash
cargo test --workspace -- --skip pianoteq --skip keyscape 2>&1
```
Expected: all test suites pass. No regressions.

- [ ] **Step 3: Commit (if any fixes were needed)**
```bash
git commit -m "test: test hardening complete — 179 compliance tests"
```
