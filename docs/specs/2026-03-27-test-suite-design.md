# Moonlitt Audio DSP Test Suite Design

## Goal

Ensure every DSP formula in moonlitt is mathematically correct and doesn't regress. Audio code tolerates zero error — a wrong coefficient means audible artifacts.

## Architecture

New crate: `moonlitt-test-suite` (integration tests only, not shipped).

Dependencies: `rustfft`, `hound`, `approx`, `moonlitt-resampler`, `moonlitt-engine`, `moonlitt-runtime`.

## Layer 1: Mathematical Correctness (moonlitt-resampler)

7 tests verifying sinc interpolation math:

1. `sinc_at_zero`: sinc(0) == 1.0 (exact)
2. `sinc_table_normalized`: for each fractional step, sum of coefficients ≈ 1.0 (tolerance 1e-4)
3. `constant_signal_preserved`: interpolate [1.0; 256] at any frac → 1.0 (tolerance 1e-6)
4. `sine_wave_reconstruction`: 440Hz sine, interpolate at 0.5 offsets → compare to sin(midpoint), error < 0.01
5. `quality_hierarchy`: Sinc72 error < Sinc8 error on high-freq sine
6. `kaiser_window_symmetry`: window(n) == window(-n) for all n
7. `bessel_i0_reference`: I0(0)=1.0, I0(1)≈1.266066, I0(5)≈27.2399, I0(10)≈2815.72 (tolerance 1e-2)

## Layer 2: Spectral Analysis (Aliasing Detection)

3 tests using rustfft:

8. `aliasing_measurement`: Generate 10kHz sine at 44100Hz → pitch shift 2x via Sinc72 → FFT → energy above 20kHz should be < -80dB below fundamental
9. `snr_measurement`: Generate windowed 1kHz sine → resample → FFT → measure signal peak vs noise floor → SNR ≥ 100dB for Sinc72
10. `sinc72_vs_linear_snr`: Same test for Linear — Sinc72 SNR must be > Linear SNR by at least 40dB

## Layer 3: Mixer Pipeline

7 tests verifying mixer math:

11. `pan_constant_power`: For pan values -1.0 to 1.0 in 0.1 steps: sqrt(L² + R²) variation < 0.5dB
12. `pan_hard_left`: pan=-1.0 → R < 0.01, L > 0.99
13. `pan_hard_right`: pan=1.0 → L < 0.01, R > 0.99
14. `mute_produces_silence`: muted track → output all zeros
15. `solo_isolates_track`: solo track A → output == track A alone
16. `limiter_bounds_output`: input 5.0 → output ≤ 1.0 + epsilon
17. `limiter_continuity`: samples at threshold-ε and threshold+ε differ by < 0.1

## Layer 4: Golden Master Regression

3 tests using saved reference WAVs:

18. `golden_sf_spec_test`: Render sf_spec_test.mid + sf_spec_test.sf2 → compare to saved golden WAV → SNR ≥ 60dB
19. `golden_interpolation_test`: Render interpolation test → compare → SNR ≥ 60dB
20. `golden_voyage`: Render voyage.mid first 30s → compare → SNR ≥ 60dB

Golden WAVs are generated once and committed. Any future code change that breaks SNR below 60dB is a regression.

## Implementation

Single file: `crates/moonlitt-test-suite/tests/dsp_validation.rs`

Helper functions:
- `generate_sine(freq, sample_rate, duration) -> Vec<f32>`
- `compute_snr_fft(signal: &[f32], sample_rate: u32) -> f64` (using rustfft)
- `render_midi_to_wav(midi_path, sf2_path) -> Vec<f32>`
- `compare_wav_snr(a: &[f32], b: &[f32]) -> f64`

## Non-Goals

- Perceptual quality metrics (ViSQOL/POLQA) — too complex for v1
- Real-time performance benchmarks — separate concern
- Cross-implementation comparison (vs FluidSynth) — different engines, different output
