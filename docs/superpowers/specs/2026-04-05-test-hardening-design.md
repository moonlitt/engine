# Test Hardening Design Spec

**Date:** 2026-04-05
**Status:** Draft
**Scope:** DSP compliance tests for 16 new effects + oversampler, added to moonlitt-test-suite

## Motivation

The 16 new effects (limiter, gate, de-esser, delay, chorus, flanger, phaser, tremolo, gain, stereo width, saturator, bitcrusher, multiband compressor, auto-filter, pitch shifter) and the oversampler currently have only basic unit tests (bypass, param round-trip, simple behavior checks — 177 tests in moonlitt-effects). They lack the DSP precision verification that the original effects have in moonlitt-test-suite (AES17 signal quality, timing accuracy, spectral compliance, aliasing detection). This spec adds 64 compliance tests to close that gap.

## Design Principles

- **Follow existing test-suite patterns** — same naming convention (`l1_`, `g1_`, `m1_`), same assertion style (machine epsilon where possible, explicit dB tolerances otherwise), same FFT analysis tools
- **Standard-referenced** — cite ITU-R BS.1770, AES17, or Audio EQ Cookbook where applicable
- **Shared helpers** — extract common test utilities into a shared module to eliminate duplication across 10+ test files

## File Structure

```
crates/moonlitt-test-suite/tests/
├── helpers/
│   └── mod.rs                    NEW: shared test utilities
├── limiter_compliance.rs         NEW (8 tests)
├── gate_compliance.rs            NEW (7 tests)
├── deesser_compliance.rs         NEW (5 tests)
├── modulation_compliance.rs      NEW (13 tests: delay/chorus/flanger/phaser/tremolo)
├── distortion_compliance.rs      NEW (7 tests: saturator/bitcrusher)
├── utility_compliance.rs         NEW (5 tests: gain/stereo_width)
├── multiband_compliance.rs       NEW (6 tests)
├── auto_filter_compliance.rs     NEW (4 tests)
├── pitch_shifter_compliance.rs   NEW (5 tests)
├── oversampler_compliance.rs     NEW (4 tests)
└── (existing files unchanged)
```

Total: 10 new test files + 1 shared helper module = **64 new compliance tests**.

---

## Shared Test Helpers (helpers/mod.rs)

Extract common functions used across multiple test files:

```rust
use rustfft::{FftPlanner, num_complex::Complex};
use std::f64::consts::PI;

/// Generate a mono sine wave at the given frequency.
pub fn sine_wave(freq: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| (2.0 * PI * freq * i as f64 / sample_rate as f64).sin() as f32)
        .collect()
}

/// Generate a sine wave at a specific dBFS level.
pub fn sine_wave_dbfs(freq: f64, dbfs: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
    let amp = 10.0_f64.powf(dbfs / 20.0);
    (0..num_samples)
        .map(|i| (amp * (2.0 * PI * freq * i as f64 / sample_rate as f64).sin()) as f32)
        .collect()
}

/// RMS of a buffer.
pub fn rms(buf: &[f32]) -> f64 {
    (buf.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / buf.len() as f64).sqrt()
}

/// RMS in dBFS.
pub fn rms_dbfs(buf: &[f32]) -> f64 {
    let r = rms(buf);
    if r < 1e-10 { -200.0 } else { 20.0 * r.log10() }
}

/// Compute power spectrum via FFT. Returns magnitude in dB per bin.
pub fn power_spectrum(signal: &[f32]) -> Vec<f64> {
    let n = signal.len();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    let mut buffer: Vec<Complex<f64>> = signal.iter()
        .map(|&s| Complex::new(s as f64, 0.0))
        .collect();
    fft.process(&mut buffer);
    buffer.iter()
        .take(n / 2)
        .map(|c| 20.0 * (c.norm() / n as f64).max(1e-20).log10())
        .collect()
}

/// Find the frequency of the highest peak in the spectrum.
pub fn find_peak_frequency(signal: &[f32], sample_rate: u32) -> f64 {
    let spectrum = power_spectrum(signal);
    let peak_bin = spectrum.iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap().0;
    peak_bin as f64 * sample_rate as f64 / signal.len() as f64
}

/// Count positive-going zero crossings.
pub fn count_zero_crossings(signal: &[f32]) -> usize {
    signal.windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count()
}

/// Measure THD+N: ratio of harmonic+noise power to fundamental power.
pub fn measure_thd_db(signal: &[f32], fundamental_freq: f64, sample_rate: u32) -> f64 {
    let spectrum = power_spectrum(signal);
    let bin_width = sample_rate as f64 / signal.len() as f64;
    let fund_bin = (fundamental_freq / bin_width).round() as usize;

    // Fundamental power (±2 bins)
    let fund_power: f64 = spectrum[fund_bin.saturating_sub(2)..=(fund_bin + 2).min(spectrum.len() - 1)]
        .iter()
        .map(|db| 10.0_f64.powf(db / 10.0))
        .sum();

    // Total power
    let total_power: f64 = spectrum.iter()
        .map(|db| 10.0_f64.powf(db / 10.0))
        .sum();

    let thd_power = total_power - fund_power;
    if fund_power < 1e-20 { -200.0 } else { 10.0 * (thd_power / fund_power).max(1e-20).log10() }
}

/// Measure actual output delay by finding impulse peak position.
pub fn measure_impulse_delay(effect: &mut dyn moonlitt_core::AudioBackend, sample_rate: u32) -> usize {
    let block = 4096;
    let mut impulse = vec![0.0f32; block];
    impulse[0] = 1.0;
    let silence = vec![0.0f32; block];
    let mut out_l = vec![0.0f32; block];
    let mut out_r = vec![0.0f32; block];

    // Process multiple blocks to find delayed impulse
    effect.process_effect(&impulse, &impulse, &mut out_l, &mut out_r);
    // Process additional blocks if needed
    let mut all_output = out_l.clone();
    for _ in 0..4 {
        effect.process_effect(&silence, &silence, &mut out_l, &mut out_r);
        all_output.extend_from_slice(&out_l);
    }

    all_output.iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
        .unwrap().0
}

/// Process effect for N blocks to let it settle, then return one final block.
pub fn process_settled(
    effect: &mut dyn moonlitt_core::AudioBackend,
    input_l: &[f32],
    input_r: &[f32],
    settle_blocks: usize,
) -> (Vec<f32>, Vec<f32>) {
    let len = input_l.len();
    let mut out_l = vec![0.0f32; len];
    let mut out_r = vec![0.0f32; len];
    for _ in 0..settle_blocks {
        effect.process_effect(input_l, input_r, &mut out_l, &mut out_r);
    }
    effect.process_effect(input_l, input_r, &mut out_l, &mut out_r);
    (out_l, out_r)
}
```

---

## Test Specifications

### limiter_compliance.rs (8 tests)

```
l1_true_peak_never_exceeds_ceiling
  — Feed signal with known inter-sample peaks (two samples: 0.9, -0.9 creating
    1.0+ inter-sample peak). At 2x oversampling, output true peak must ≤ ceiling.
  — Tolerance: ceiling + 0.01 (f32 rounding)

l2_attack_timing_precision
  — Step from silence to +6dB. Measure samples until gain reduction reaches 63.2%
    of target. Should match attack_ms × sample_rate/1000 ± 10%.

l3_release_timing_precision
  — From full limiting to silence. Measure samples until gain recovers to 63.2%
    of unity. Should match release_ms × sample_rate/1000 ± 10%.

l4_auto_release_adapts
  — Process two signals: (A) sparse transients (single impulse per 100ms),
    (B) sustained loud signal. Measure effective release time of each.
    Auto-release should produce shorter release for A than for B.

l5_lookahead_latency_matches
  — Feed impulse, measure delay via measure_impulse_delay().
    Must equal latency() return value ± 1 sample.

l6_ceiling_hard_clip
  — Feed 10.0 amplitude sine (extreme), verify every output sample
    abs ≤ ceiling_linear. No exceptions.

l7_oversampling_alias_rejection
  — 1x vs 2x: process 15kHz sine at high drive through limiter.
    Measure aliasing energy above 18kHz. 2x should have > 30dB less aliasing.

l8_bypass_thd
  — Bypass mode THD+N < -140dB (bit-exact passthrough).
```

### gate_compliance.rs (7 tests)

```
g1_closed_attenuation_matches_range
  — Feed -60dB signal (below threshold=-40dB). After settling,
    output level should be input_level + range_db ± 0.5dB.

g2_open_is_unity
  — Feed -10dB signal (above threshold=-40dB). After settling,
    output level should equal input level ± 0.1dB.

g3_attack_timing
  — Sudden loud signal after silence. Measure time to reach 63.2% of open gain.
    Should match attack_ms ± 15%.

g4_hold_prevents_chattering
  — Feed 50Hz amplitude-modulated signal (level oscillates around threshold).
    With hold=100ms, gate transitions should be ≤ 10/second.
    Without hold (0ms), transitions should be > 50/second.

g5_hysteresis_deadband
  — Feed slowly rising then falling signal crossing threshold.
    Gate should open at threshold_db and close at threshold_db - hysteresis_db.
    Verify the two crossing points differ by exactly hysteresis_db ± 0.5dB.

g6_release_timing
  — From open to closed. Measure time to reach 63.2% of closed gain.
    Should match release_ms ± 15%.

g7_sidechain_filter_isolation
  — HPF=500Hz: 200Hz sine should not trigger gate, 2kHz sine should.
    LPF=2000Hz: 5kHz sine should not trigger gate, 1kHz sine should.
```

### deesser_compliance.rs (5 tests)

```
d1_sibilance_attenuation_ratio
  — 6kHz sine at -10dBFS, threshold=-20dB, ratio=4. Expected GR ≈ 7.5dB.
    Actual attenuation should be within ± 2dB of expected.

d2_non_sibilant_passthrough_splitband
  — Split-band mode: 200Hz sine passes with < 0.5dB deviation.

d3_listen_mode_is_bandpass
  — Listen mode output spectrum should show bandpass shape centered at
    frequency param. Verify -3dB points are near frequency/Q and frequency×Q.

d4_wideband_vs_splitband_low_freq
  — Same sibilant input: wideband attenuates 200Hz component,
    split-band preserves it. Difference > 3dB at 200Hz.

d5_frequency_tracking
  — Set frequency=4000, process 4kHz sine → attenuation.
    Set frequency=8000, process same 4kHz sine → no attenuation.
    Verifies detection band tracks parameter.
```

### modulation_compliance.rs (13 tests)

```
Delay:
m1_delay_time_sample_accuracy
  — Impulse at t=0, verify peak appears at exactly delay_time_ms × sr/1000 samples.
    Tolerance: ±1 sample.

m2_tempo_sync_precision
  — sync_mode=1, 1/4 note @ 120BPM. Measure impulse delay.
    Expected: 500ms = 22050 samples @ 44100Hz. Tolerance: ±2 samples.

m3_feedback_decay_rate
  — Impulse with feedback=0.5. Measure amplitude of 1st, 2nd, 3rd repeats.
    Each should be 0.5× the previous ± 0.5dB.

Chorus:
m4_chorus_no_aliasing
  — 10kHz sine through chorus. Measure energy above 20kHz (if oversampled) or
    check for spurious frequencies below Nyquist that weren't in input.
    Aliasing products should be < -80dB relative to fundamental.

m5_chorus_depth_zero_no_modulation
  — depth=0: output should be a fixed-delay copy. Verify output is
    time-invariant (no frequency modulation). Cross-correlate successive blocks.

Flanger:
m6_flanger_comb_frequencies
  — Static flanger (rate=0 or very slow): delay=2ms at 44100Hz = 88.2 samples.
    Expected null frequencies at sr/(2×delay_samples), sr/(4×delay_samples) etc.
    Verify via FFT that nulls appear at expected positions ± 5%.

m7_flanger_through_zero_polarity
  — Positive feedback: wet signal in-phase with dry at DC.
    Negative feedback: wet signal inverted at DC.
    Verify sign of DC component matches feedback polarity.

m8_flanger_saturation_bounds
  — feedback=0.95, input amplitude 1.0, process 10 seconds.
    Output should never exceed ±4.0 (tanh bounds the feedback).

Phaser:
m9_phaser_notch_count
  — White noise through static phaser (rate≈0). 4 stages → 2 notches,
    8 stages → 4 notches. Count notches in FFT (dips > 10dB below neighbors).

m10_phaser_sweep_range
  — Sweep with min=200Hz, max=4000Hz. Record the notch frequency over time.
    Minimum notch should be ≥ 200Hz, maximum should be ≤ 4000Hz.

Tremolo:
m11_tremolo_depth_modulation_range
  — depth=1, rate=1Hz. Over one full LFO cycle, output amplitude should
    reach near-zero (min) and near-input (max). Verify min < 0.05, max > 0.9.

m12_tremolo_stereo_phase_opposition
  — Stereo mode: measure L and R amplitude envelopes over one cycle.
    When L is at max, R should be at min. Cross-correlation of envelopes ≈ -1.

m13_tremolo_tempo_sync_rate
  — sync=1/4 @ 120BPM = 2Hz. Count zero crossings in the amplitude envelope
    over 1 second. Should be ≈ 2.
```

### distortion_compliance.rs (7 tests)

```
Saturator:
s1_tube_even_harmonics
  — Feed 1kHz sine through Tube mode. FFT: 2nd harmonic (2kHz) should be
    stronger than 3rd harmonic (3kHz). Ratio > 6dB.

s2_transistor_odd_harmonics
  — Feed 1kHz sine through Transistor mode (tanh). FFT: 3rd harmonic (3kHz)
    should be stronger than 2nd harmonic (2kHz). Ratio > 6dB.

s3_oversampling_alias_rejection
  — 10kHz sine, high drive. 1x vs 2x oversampling.
    Measure energy in 18-22kHz band. 2x should have > 30dB less.

s4_asymmetry_dc_blocked
  — asymmetry=0.5, feed 1kHz sine. Output DC component should be < -60dB
    (DC blocker working). Without DC blocker, would be significant.

s5_drive_zero_thd
  — drive=0dB (minimum), Transistor mode. THD < -40dB (near-linear region of tanh).

Bitcrusher:
s6_bit_depth_quantization_noise
  — 8-bit: noise floor ≈ -48dBFS (6dB × 8). Measure noise floor of
    quantized silence+dither vs expected. Tolerance: ±3dB.

s7_rate_reduction_imaging
  — rate_reduction=4: process 1kHz sine. Output should contain mirrored
    components at sr/4 - 1kHz and sr/4 + 1kHz. Verify via FFT peak detection.
```

### utility_compliance.rs (5 tests)

```
u1_gain_db_to_linear_precision
  — +6.0206dB = exactly 2.0× linear. -6.0206dB = exactly 0.5× linear.
    0dB = exactly 1.0×. Verify to 4 decimal places.

u2_polarity_invert_bitexact
  — polarity=1: output[i] == -input[i] for all samples. Bit-exact comparison
    (using to_bits() to handle -0.0 vs 0.0).

u3_mono_sum_energy
  — L=sin(x), R=sin(x) (correlated): mono output RMS should be same as input RMS.
    L=sin(x), R=-sin(x) (anti-correlated): mono output should be silence.

u4_stereo_width_zero_mono
  — width=0: L and R outputs must be identical. Bit-exact comparison.

u5_mid_side_orthogonality
  — Pure mid signal (L=R=0.5): adjusting side_gain should have no effect.
    Pure side signal (L=0.5, R=-0.5): adjusting mid_gain should have no effect.
    Tolerance: < -120dB crosstalk.
```

### multiband_compliance.rs (6 tests)

```
mb1_crossover_flatness
  — No compression (threshold=0, ratio=1). Process white noise.
    Output RMS should equal input RMS ± 0.1dB. Also verify via FFT:
    sweep sine from 20Hz to 20kHz, measure output level at each frequency.
    Deviation < 0.5dB across entire range.

mb2_crossover_slope_24db_oct
  — Single crossover at 1kHz. Measure LP output at 2kHz (should be ≈ -24dB).
    Measure HP output at 500Hz (should be ≈ -24dB). LR4 = 24dB/oct.

mb3_band_independence
  — 4 bands. Compress only band 2 (threshold=-20, ratio=10). Feed broadband signal.
    Band 2 frequency range should be attenuated. Other bands unaffected (< 1dB change).

mb4_crossover_phase_alignment
  — Sum of LP + HP at crossover frequency should be -6dB (LR4 property).
    Not 0dB (Butterworth) or -3dB. This verifies proper LR4 alignment.

mb5_single_band_degenerates
  — band_count=1, same threshold/ratio as standalone Compressor.
    Process identical signal through both. Output RMS should match ± 0.5dB.

mb6_per_band_ratio_precision
  — Each band independently: feed band-limited signal (sine within band range),
    compress at ratio=4, threshold=-20dB. Measure actual ratio from input/output
    levels. Should be 4:1 ± 0.5.
```

### auto_filter_compliance.rs (4 tests)

```
af1_envelope_frequency_tracking
  — Envelope mode. Feed alternating loud/quiet blocks (100ms each).
    During loud: filter opens (more HF passes). During quiet: filter closes.
    Measure -3dB cutoff frequency in each state via FFT. Loud state cutoff
    should be > 2× quiet state cutoff.

af2_resonance_peak
  — LP filter, Q=10, static frequency=1kHz. Feed white noise.
    FFT should show > 15dB peak at 1kHz relative to passband.

af3_filter_type_response
  — At fixed frequency=2kHz:
    LP: 4kHz should be attenuated > 12dB vs 1kHz
    HP: 1kHz should be attenuated > 12dB vs 4kHz
    BP: both 500Hz and 8kHz should be attenuated > 12dB vs 2kHz

af4_lfo_sweep_period
  — LFO mode, rate=2Hz. Record output spectrum over 1 second.
    The filter cutoff should complete 2 full sweeps. Verify by counting
    peaks in the amplitude envelope of a fixed-frequency probe tone.
```

### pitch_shifter_compliance.rs (5 tests)

```
ps1_granular_pitch_ratio
  — Granular mode, +12 semitones. Feed 440Hz sine.
    Output peak frequency (via FFT) should be 880Hz ± 5%.

ps2_vocoder_pitch_ratio
  — Phase vocoder mode, +12 semitones. Feed 440Hz sine.
    Output peak frequency should be 880Hz ± 2% (vocoder is more precise).

ps3_zero_shift_passthrough
  — semitones=0, cents=0. Process 1kHz sine for 1 second.
    Cross-correlation with delayed input should be > 0.9 (high similarity).

ps4_latency_matches_report
  — Granular: latency() should equal grain_size_ms/2 × sr/1000 ± 1 sample.
    Vocoder: latency() should equal fft_size/2 ± 1 sample.
    Verify with impulse measurement.

ps5_granular_no_clicks
  — Process 1 second of 440Hz sine, granular mode.
    Compute sample-to-sample differences. Max difference should be < 0.5
    (no discontinuities from grain boundaries).
```

### oversampler_compliance.rs (4 tests)

```
os1_alias_rejection_96db
  — Upsample 2x, inject tone at 0.6×Nyquist (in the stopband), downsample.
    Residual tone should be attenuated > 90dB.

os2_passband_ripple
  — Sweep sine from 20Hz to 0.4×Nyquist through upsample→downsample.
    Measure output level at each frequency. Deviation < 0.1dB.

os3_phase_linearity
  — Process impulse through 2x up→down. Measure group delay via
    phase spectrum derivative. Should be constant ± 0.5 samples across passband.

os4_cascade_equivalence
  — Process same signal through factor=4 and through two sequential factor=2.
    Output should match within floating-point tolerance (< -120dB difference).
```

---

## Implementation Notes

- All tests go in `crates/moonlitt-test-suite/tests/`
- Each file uses `mod helpers;` to import shared utilities
- Effects are created via `moonlitt_effects::EffectName::new(sample_rate)`
- All effects are used through the `AudioBackend` trait (`use moonlitt_core::AudioBackend`)
- FFT analysis uses `rustfft` (already in test-suite dev-dependencies)
- Process with `process_settled()` helper (typically 5-10 settle blocks) before measuring
- Assertions use explicit numeric tolerances, not `approx` macros (for clarity)

## Implementation Order

```
(1) helpers/mod.rs — shared test utilities
(2) limiter_compliance.rs
(3) gate_compliance.rs
(4) deesser_compliance.rs
(5) modulation_compliance.rs (largest — 13 tests)
(6) distortion_compliance.rs
(7) utility_compliance.rs
(8) multiband_compliance.rs
(9) auto_filter_compliance.rs
(10) pitch_shifter_compliance.rs
(11) oversampler_compliance.rs
```

Each step adds one test file and must pass `cargo test -p moonlitt-test-suite` before proceeding.
