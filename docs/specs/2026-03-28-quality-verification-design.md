# moonlitt Audio Quality Verification Suite

**Date:** 2026-03-28
**Status:** Approved

## Goal

Mathematically verify that every audio path in moonlitt produces spec-correct output. Zero tolerance for technically solvable errors.

## New Functionality

### TPDF Dithering (`crates/moonlitt-runtime/src/dither.rs`)

TPDF (Triangular Probability Density Function) dither applied at master bus output stage.

```
dither = (rand1 - rand2) / 2^target_bits
output = input_float + dither
```

- Default target: 24-bit (macOS CoreAudio)
- Two independent uniform random sources, subtracted to form triangular distribution
- Integrated into `Mixer::render()` output stage, after soft limiter

### True Peak Metering (extend `LevelMeter`)

4x oversampled peak detection per EBU R128.

- 4-point linear interpolation between each pair of samples (3 interpolated points per gap)
- Take max absolute value across all original + interpolated samples
- No heap allocation: fixed 4-sample sliding window
- New method: `LevelMeter::true_peak() -> (f32, f32)`

## Verification Tests (13 items)

All tests in `crates/moonlitt-test-suite/tests/quality_verification.rs`.
Test SF2 files constructed programmatically via `soundfont-rs`.

### 1. SM24 24-bit Precision

- Build two SF2: same 440Hz sine, one 16-bit (smpl only), one 24-bit (smpl + sm24)
- Render same note through OxiSynth
- Measure noise floor via FFT (signal bin excluded)
- **Assert:** 24-bit noise floor lower than 16-bit by >= 40dB

### 2. Modulator Linking + Cycle Safety

- Build SF2 with Link modulator: CC1 → Mod A (output) → Mod B (Link source) → filter cutoff
- Render with CC1=0, then CC1=127
- Measure high-frequency energy (>2kHz) via FFT
- **Assert:** spectral difference > 6dB (filter is being modulated)
- Build SF2 with circular Link (A→B→A), render
- **Assert:** no crash, no infinite loop, completes in bounded time

### 3. Group Track Routing + Insert Chain

- Create mixer: track0 (sine 440Hz), track1 (sine 880Hz) → group track → master
- Group track has a passthrough insert (no-backend engine = passthrough)
- Render
- **Assert:** output = track0 + track1, per-sample error < 1e-6
- Also verify group's insert was invoked (replace passthrough with gain-doubling mock → output doubles)

### 4. Nested Group (A→B→C→Master)

- 3 tracks: src → groupA → groupB → master
- **Assert:** output matches src signal, per-sample error < 1e-6

### 5. PDC Multi-Latency Alignment

- Track0 with mock insert reporting 512 samples latency
- Track1 with mock insert reporting 256 samples latency
- Both play same note simultaneously
- Cross-correlate the two tracks' contributions in master output
- **Assert:** correlation peak at offset = 0 (both aligned)

### 6. Session Restore Complete State

- Load GeneralUser_GS.sf2, set volume=0.7, pan=-0.3, add insert, set send level, set routing
- Save session → restore from JSON → render
- Compare with original render
- **Assert:** output bit-exact (zero difference)

### 7. TPDF Dither Spectral Flatness

- Render silence through dithered output (dither-only signal)
- FFT analysis, divide spectrum into 8 equal bands
- Measure power in each band
- **Assert:** max band power - min band power < 3dB (uniform noise)

### 8. True Peak Intersample Detection

- Construct signal with two adjacent samples at +0.5, surrounded by zeros
- True peak between them should exceed 0.5 (intersample overshoot)
- **Assert:** true_peak > sample_peak
- **Assert:** true_peak value error < 0.01dB vs analytical expectation

### 9. SF2 Waveform Precision (1:1 Playback)

- Build SF2 with known 440Hz sine at 44100Hz sample rate
- Play back at 44100Hz (1:1 ratio, no resampling)
- Compare rendered output with original sine samples
- **Assert:** per-sample error < 1e-6

### 10. SF2 Velocity→Attenuation

- SF2 spec formula: `attenuation = -200 * log2(velocity^2 / 127^2)` centibels
- Render same note at velocity 32, 64, 96, 127
- Measure peak amplitude of each
- Convert to centibels, compare with spec formula
- **Assert:** error < 0.1 centibel per velocity value

### 11. SF2 Filter -12dB/octave

- Build SF2 with initialFilterFc set to known cutoff (e.g., 1000Hz)
- Render white noise through it (or impulse response)
- FFT analysis: measure attenuation at 2kHz, 4kHz, 8kHz relative to passband
- **Assert:** slope is -12 ±1 dB per octave (2-pole lowpass)

### 12. Insert Chain Audio Flow

- Track with one insert (no-backend engine → zeros output = "mute" effect)
- Render track that produces audio
- **Assert:** output is silence (insert zeroed it)
- Track with bypassed insert
- **Assert:** output is non-silent (bypass skips the insert)

### 13. Soft Limiter THD

- Render pure sine at 0.5 amplitude (below 0.95 threshold)
- FFT: measure THD (total harmonic distortion)
- **Assert:** THD < -120dB (effectively zero distortion below threshold)
- Render pure sine at 2.0 amplitude (above threshold)
- **Assert:** THD < -40dB (controlled distortion)
- **Assert:** peak output <= 1.0 (no clipping past DAC range)

## File Structure

```
crates/moonlitt-runtime/src/dither.rs                    # TPDF dithering module
crates/moonlitt-runtime/src/mixer.rs                     # true peak extension
crates/moonlitt-test-suite/tests/quality_verification.rs  # 13 verification tests
crates/moonlitt-test-suite/tests/test_sf2_builder.rs     # programmatic SF2 construction
```

## Success Criteria

All 13 tests pass. Every audio path has mathematical proof of correctness. Zero "faith code."
