//! Universal silence-in / silence-out invariant for every built-in effect.
//!
//! An effect at default parameters that is fed pure silence MUST emit pure
//! silence (within denormal tolerance). Violations indicate one of:
//!   - uninitialized internal state
//!   - an LFO bleeding into the output without an input gate
//!   - feedback loop with positive DC gain (the bug class that cost us the
//!     DattorroReverb DC offset incident on 2026-04)
//!   - denormal arithmetic noise that should be flushed
//!
//! This single test loops every effect — adding a new effect requires
//! adding one line below, and the universal invariant kicks in immediately.

use moonlitt_core::AudioBackend;
use moonlitt_effects::{
    AutoFilter, Bitcrusher, Chorus, Compressor, DattorroReverb, DeEsser, Flanger, Gain, Gate,
    Limiter, MultibandCompressor, ParametricEq, Phaser, PitchShifter, Reverb, Saturator,
    StereoDelay, StereoWidth, Tremolo,
};

const SR: u32 = 44100;
const BLOCK: usize = 256;
/// 2 seconds — long enough for any reasonable LFO / envelope to cycle, short
/// enough to keep the test suite fast.
const SECONDS: usize = 2;
/// Tolerance: 1e-6 ≈ -120 dBFS. Any nonzero output above this threshold
/// indicates real signal, not denormal noise (denormals flush near 1e-38).
const SILENCE_TOLERANCE: f32 = 1e-6;

fn render_silence<E: AudioBackend>(effect: &mut E) -> (f32, f32) {
    let n = SR as usize * SECONDS;
    let zeros = vec![0.0f32; n];
    let mut out_l = vec![0.0f32; n];
    let mut out_r = vec![0.0f32; n];

    let mut i = 0;
    while i < n {
        let end = (i + BLOCK).min(n);
        effect.process_effect(
            &zeros[i..end],
            &zeros[i..end],
            &mut out_l[i..end],
            &mut out_r[i..end],
        );
        i = end;
    }

    let peak = |b: &[f32]| b.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    (peak(&out_l), peak(&out_r))
}

fn assert_silent<E: AudioBackend>(name: &str, mut effect: E) {
    let (peak_l, peak_r) = render_silence(&mut effect);
    let bad = peak_l > SILENCE_TOLERANCE || peak_r > SILENCE_TOLERANCE;
    assert!(
        !bad,
        "{name}: silence in must yield silence out (peak L={peak_l:.3e}, R={peak_r:.3e}, \
         tolerance={SILENCE_TOLERANCE:.0e})"
    );
}

// =============================================================================
// Spatial
// =============================================================================

#[test]
fn silence_reverb() {
    assert_silent("Reverb (Freeverb)", Reverb::new(SR));
}

#[test]
fn silence_dattorro_reverb() {
    assert_silent("DattorroReverb", DattorroReverb::new(SR));
}

// =============================================================================
// Dynamics
// =============================================================================

#[test]
fn silence_compressor() {
    assert_silent("Compressor", Compressor::new(SR));
}

#[test]
fn silence_limiter() {
    assert_silent("Limiter", Limiter::new(SR));
}

#[test]
fn silence_gate() {
    assert_silent("Gate", Gate::new(SR));
}

#[test]
fn silence_deesser() {
    assert_silent("DeEsser", DeEsser::new(SR));
}

#[test]
fn silence_multiband_compressor() {
    assert_silent("MultibandCompressor", MultibandCompressor::new(SR));
}

// =============================================================================
// EQ
// =============================================================================

#[test]
fn silence_parametric_eq() {
    assert_silent("ParametricEq", ParametricEq::new(SR));
}

// =============================================================================
// Modulation
// =============================================================================

#[test]
fn silence_stereo_delay() {
    assert_silent("StereoDelay", StereoDelay::new(SR));
}

#[test]
fn silence_chorus() {
    assert_silent("Chorus", Chorus::new(SR));
}

#[test]
fn silence_flanger() {
    assert_silent("Flanger", Flanger::new(SR));
}

#[test]
fn silence_phaser() {
    assert_silent("Phaser", Phaser::new(SR));
}

#[test]
fn silence_tremolo() {
    assert_silent("Tremolo", Tremolo::new(SR));
}

#[test]
fn silence_auto_filter() {
    assert_silent("AutoFilter", AutoFilter::new(SR));
}

#[test]
fn silence_pitch_shifter() {
    assert_silent("PitchShifter", PitchShifter::new(SR));
}

// =============================================================================
// Distortion
// =============================================================================

#[test]
fn silence_saturator() {
    assert_silent("Saturator", Saturator::new(SR));
}

#[test]
fn silence_bitcrusher() {
    assert_silent("Bitcrusher", Bitcrusher::new(SR));
}

// =============================================================================
// Utility
// =============================================================================

#[test]
fn silence_gain() {
    assert_silent("Gain", Gain::new(SR));
}

#[test]
fn silence_stereo_width() {
    assert_silent("StereoWidth", StereoWidth::new(SR));
}
