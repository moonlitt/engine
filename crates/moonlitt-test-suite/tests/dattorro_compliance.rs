//! DattorroReverb Compliance Tests
//!
//! Reference: Jon Dattorro, "Effect Design Part 1: Reverberator and Other Filters"
//! Journal of the Audio Engineering Society, Vol. 45, No. 9, 1997.
//!
//! These are **L1 invariant tests** — they assert mathematical properties that
//! must hold regardless of the snapshot value. They are designed to catch
//! systematic bugs that snapshot regression cannot detect.
//!
//! Historical context: prior to 2026-04, the Schroeder allpass filters in this
//! reverb used "form 1" (`buffer = input + g·delayed`) which has DC gain
//! `(1-g+g²)/(1-g) > 1`. This caused:
//!   - DC bias amplification (+0.001 input DC → -0.30 output DC)
//!   - Signal magnitude blow-up (440Hz/0.5 input → peak 7.39, ~14×)
//! No existing test caught this. The tests below are designed so that
//! reintroducing such a bug would fail at least one assertion.

use moonlitt_core::AudioBackend;
use moonlitt_effects::DattorroReverb;
use std::f64::consts::TAU;

const SR: u32 = 44100;

// Param IDs (mirrored from src for test legibility — kept in sync manually
// because they are private to the effects crate).
const PARAM_PREDELAY: u32 = 0;
const PARAM_DECAY: u32 = 1;
const PARAM_DAMPING: u32 = 2;
const PARAM_DIFFUSION: u32 = 3;
const PARAM_WET_LP_FREQ: u32 = 4;
const PARAM_WET_HP_FREQ: u32 = 5;
const PARAM_STEREO_WIDTH: u32 = 6;
const PARAM_DRY_WET: u32 = 7;
const PARAM_BYPASS: u32 = 8;
const PARAM_MOD_DEPTH: u32 = 9;
const PARAM_MOD_RATE: u32 = 10;
const PARAM_INPUT_DIFFUSION_2: u32 = 11;
const PARAM_DECAY_DIFFUSION_1: u32 = 12;
const PARAM_DECAY_DIFFUSION_2: u32 = 13;

const PARAM_IDS_ALL: &[u32] = &[
    PARAM_PREDELAY,
    PARAM_DECAY,
    PARAM_DAMPING,
    PARAM_DIFFUSION,
    PARAM_WET_LP_FREQ,
    PARAM_WET_HP_FREQ,
    PARAM_STEREO_WIDTH,
    PARAM_DRY_WET,
    PARAM_BYPASS,
    PARAM_MOD_DEPTH,
    PARAM_MOD_RATE,
    PARAM_INPUT_DIFFUSION_2,
    PARAM_DECAY_DIFFUSION_1,
    PARAM_DECAY_DIFFUSION_2,
];

// =============================================================================
// Helpers
// =============================================================================

fn render_block(
    rev: &mut DattorroReverb,
    in_l: &[f32],
    in_r: &[f32],
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let n = in_l.len();
    let mut out_l = vec![0.0f32; n];
    let mut out_r = vec![0.0f32; n];
    let mut i = 0;
    while i < n {
        let end = (i + block).min(n);
        rev.process_effect(
            &in_l[i..end],
            &in_r[i..end],
            &mut out_l[i..end],
            &mut out_r[i..end],
        );
        i = end;
    }
    (out_l, out_r)
}

fn dc(buf: &[f32]) -> f64 {
    if buf.is_empty() {
        0.0
    } else {
        buf.iter().map(|&s| s as f64).sum::<f64>() / buf.len() as f64
    }
}

fn peak(buf: &[f32]) -> f32 {
    buf.iter().map(|&s| s.abs()).fold(0.0, f32::max)
}

// =============================================================================
// L1: silence_in_silence_out — fundamental zero-state invariant
//
// A reverb fed pure silence (and whose internal state is zeroed at construction)
// must produce pure silence. Any nonzero output indicates either:
//   - uninitialized buffers
//   - an oscillator / feedback loop with positive DC gain
//   - parameter denormalization noise (which should be flushed)
// =============================================================================

#[test]
fn d1_silence_in_silence_out_default_params() {
    let mut rev = DattorroReverb::new(SR);
    let n = SR as usize * 5; // 5 seconds
    let zeros = vec![0.0f32; n];
    let (out_l, out_r) = render_block(&mut rev, &zeros, &zeros, 256);

    let p_l = peak(&out_l);
    let p_r = peak(&out_r);
    assert!(
        p_l < 1e-9 && p_r < 1e-9,
        "silence in must yield silence out (peak L={:.3e}, peak R={:.3e})",
        p_l,
        p_r
    );
}

#[test]
fn d1_silence_in_silence_out_extreme_params() {
    // Same invariant must hold for any legal parameter setting.
    let extreme_decays: &[f64] = &[0.0, 0.5, 0.99];
    let extreme_damps: &[f64] = &[0.0, 0.5, 0.99];
    let extreme_diffs: &[f64] = &[0.0, 0.5, 1.0];

    let n = SR as usize * 2; // shorter per case (we cover many)
    let zeros = vec![0.0f32; n];

    for &d in extreme_decays {
        for &dmp in extreme_damps {
            for &diff in extreme_diffs {
                let mut rev = DattorroReverb::new(SR);
                rev.set_param(PARAM_DECAY, d);
                rev.set_param(PARAM_DAMPING, dmp);
                rev.set_param(PARAM_DIFFUSION, diff);
                rev.set_param(PARAM_INPUT_DIFFUSION_2, diff);
                rev.set_param(PARAM_DECAY_DIFFUSION_1, diff);
                rev.set_param(PARAM_DECAY_DIFFUSION_2, diff);
                rev.set_param(PARAM_DRY_WET, 1.0);

                let (out_l, out_r) = render_block(&mut rev, &zeros, &zeros, 256);
                let p = peak(&out_l).max(peak(&out_r));
                assert!(
                    p < 1e-9,
                    "silence in must yield silence out at decay={d}, damp={dmp}, diff={diff} \
                     (peak={p:.3e})"
                );
            }
        }
    }
}

// =============================================================================
// L1: bounded_output_for_sine — energy / magnitude invariant
//
// The pre-fix bug amplified a 440Hz / 0.5 sine to peak 7.39 (~14×).
// A correctly designed reverb with allpass diffusion and decay < 1 should
// NOT amplify steady-state input by more than a small factor. We assert a
// generous bound (≤4× input peak) — the real implementation comes in around
// 1.5×; a 14× failure would jump well past 4×.
// =============================================================================

#[test]
fn d2_bounded_output_for_sine_default() {
    let mut rev = DattorroReverb::new(SR);
    rev.set_param(PARAM_DRY_WET, 0.5); // user-typical chain setting

    let n = SR as usize * 5;
    let amp = 0.5;
    let sine: Vec<f32> = (0..n)
        .map(|i| (i as f64 / SR as f64 * 440.0 * TAU).sin() as f32 * amp)
        .collect();

    let (out_l, out_r) = render_block(&mut rev, &sine, &sine, 256);

    // Skip the first 1s of buildup before measuring steady-state peak.
    let warmup = SR as usize;
    let p_l = peak(&out_l[warmup..]);
    let p_r = peak(&out_r[warmup..]);

    let bound = 4.0 * amp;
    assert!(
        p_l <= bound && p_r <= bound,
        "440Hz sine (amp={amp}) must not be amplified beyond {bound}× \
         (peak L={p_l:.4}, peak R={p_r:.4}) — historical DC-form-1 bug yielded peak ≈ {:.2}",
        7.39
    );
}

#[test]
fn d2_bounded_output_high_decay() {
    // High decay is the harshest stability test. Even at decay=0.95, output
    // must remain bounded (no runaway feedback).
    let mut rev = DattorroReverb::new(SR);
    rev.set_param(PARAM_DECAY, 0.95);
    rev.set_param(PARAM_DRY_WET, 1.0); // 100% wet — pure tank output

    let n = SR as usize * 8;
    let amp = 0.5;
    let sine: Vec<f32> = (0..n)
        .map(|i| (i as f64 / SR as f64 * 440.0 * TAU).sin() as f32 * amp)
        .collect();

    let (out_l, out_r) = render_block(&mut rev, &sine, &sine, 256);

    // After 8 seconds at decay=0.95 the field is fully developed.
    let warmup = SR as usize * 2;
    let p = peak(&out_l[warmup..]).max(peak(&out_r[warmup..]));

    let bound = 6.0 * amp; // generous — decay=0.95 builds up significant tail
    assert!(
        p <= bound,
        "high-decay reverb must remain bounded for steady sine input \
         (peak={p:.4}, bound={bound})"
    );
}

// =============================================================================
// L1: dc_bias_bounded — DC accumulation invariant
//
// The pre-fix bug took an input DC bias of +0.001 and produced output DC of
// -0.30 (default) or -1.04 (user chain) — a 300×–1000× amplification.
// Correct allpass topology yields unity DC gain through the diffusion network;
// the tank's DC gain must be ≤ 1 so that DC does not run away.
//
// We feed a small DC bias for several seconds and assert that the absolute
// output DC is at most ~10× the input DC (a generous bound).
// =============================================================================

#[test]
fn d3_dc_bias_bounded_default() {
    let mut rev = DattorroReverb::new(SR);
    let n = SR as usize * 5;
    let bias = 0.001_f32;
    let dc_input = vec![bias; n];

    let (out_l, out_r) = render_block(&mut rev, &dc_input, &dc_input, 256);

    let warmup = SR as usize;
    let dc_l = dc(&out_l[warmup..]).abs();
    let dc_r = dc(&out_r[warmup..]).abs();

    let bound = (bias as f64) * 10.0; // ≤ 10× amplification
    assert!(
        dc_l <= bound && dc_r <= bound,
        "DC input {bias} must not be amplified beyond {bound:.4} \
         (output |DC| L={dc_l:.6}, R={dc_r:.6}) — historical form-1 bug yielded ~0.30",
    );
}

#[test]
fn d3_dc_bias_bounded_user_chain() {
    // Reproduces the exact param combination that exposed the original bug:
    // decay=0.6, dry_wet=0.2 (the user's typical insert chain settings).
    let mut rev = DattorroReverb::new(SR);
    rev.set_param(PARAM_DECAY, 0.6);
    rev.set_param(PARAM_DRY_WET, 0.2);

    let n = SR as usize * 5;
    let bias = 0.001_f32;
    let dc_input = vec![bias; n];

    let (out_l, out_r) = render_block(&mut rev, &dc_input, &dc_input, 256);

    let warmup = SR as usize;
    let dc_l = dc(&out_l[warmup..]).abs();
    let dc_r = dc(&out_r[warmup..]).abs();

    let bound = (bias as f64) * 10.0;
    assert!(
        dc_l <= bound && dc_r <= bound,
        "DC input {bias} (user chain) must not be amplified beyond {bound:.4} \
         (output |DC| L={dc_l:.6}, R={dc_r:.6}) — historical form-1 bug yielded ~1.04",
    );
}

// =============================================================================
// L1: no_nan_inf_on_param_sweep — finiteness invariant
//
// For any legal parameter value, processing finite input must produce only
// finite output. A reverb that emits NaN/Inf for some param combination would
// crash the audio thread or silently corrupt downstream effects.
// =============================================================================

#[test]
fn d4_no_nan_inf_on_param_extremes() {
    // Use a deterministic non-trivial input: pulse train + tone.
    let n = SR as usize; // 1 second per case
    let input: Vec<f32> = (0..n)
        .map(|i| {
            let tone = (i as f64 / SR as f64 * 220.0 * TAU).sin() as f32 * 0.3;
            let pulse = if i % 4410 == 0 { 0.5 } else { 0.0 };
            tone + pulse
        })
        .collect();

    for &id in PARAM_IDS_ALL {
        for &val in &[0.0_f64, 1.0, 100.0, 20000.0, -1.0, 1e6] {
            let mut rev = DattorroReverb::new(SR);
            // set_param clamps internally; we still want to verify no NaN escapes.
            rev.set_param(id, val);
            let (out_l, out_r) = render_block(&mut rev, &input, &input, 256);

            let bad_l = out_l.iter().any(|s| !s.is_finite());
            let bad_r = out_r.iter().any(|s| !s.is_finite());
            assert!(
                !bad_l && !bad_r,
                "non-finite output at param {id} = {val} (L bad={bad_l}, R bad={bad_r})"
            );
        }
    }
}

// =============================================================================
// L1: impulse_response_decays — stability invariant (Schroeder/Dattorro)
//
// For decay < 1, the impulse response must decay over time. We measure
// energy in three contiguous windows after the impulse; each subsequent
// window must hold less energy than the previous (monotone decay).
// =============================================================================

#[test]
fn d5_impulse_response_decays() {
    let mut rev = DattorroReverb::new(SR);
    rev.set_param(PARAM_DECAY, 0.7);
    rev.set_param(PARAM_DRY_WET, 1.0);
    rev.set_param(PARAM_DAMPING, 0.3);

    let n = SR as usize * 4; // 4 seconds
    let mut input = vec![0.0f32; n];
    input[0] = 1.0;
    let zeros = vec![0.0f32; n];

    let (out_l, _) = render_block(&mut rev, &input, &zeros, 256);

    // Skip the initial 0.5 s for the field to fully develop, then bin into
    // three 1-second windows.
    let bin = SR as usize;
    let start = SR as usize / 2;
    let energy = |s: &[f32]| s.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>();
    let e1 = energy(&out_l[start..start + bin]);
    let e2 = energy(&out_l[start + bin..start + 2 * bin]);
    let e3 = energy(&out_l[start + 2 * bin..start + 3 * bin]);

    assert!(
        e1 > e2 && e2 > e3,
        "impulse response must decay monotonically across windows \
         (E1={e1:.6}, E2={e2:.6}, E3={e3:.6})"
    );
}

// =============================================================================
// L1: stereo_decorrelation — ModAllpass / cross-feedback invariant
//
// Mono input fed through the Dattorro tank must produce stereo output where
// L != R (the LFO-modulated allpasses and the asymmetric tank delay lengths
// guarantee this). A bug that wires both tanks identically would yield L == R.
// =============================================================================

#[test]
fn d6_stereo_decorrelation() {
    let mut rev = DattorroReverb::new(SR);
    rev.set_param(PARAM_DRY_WET, 1.0);
    rev.set_param(PARAM_STEREO_WIDTH, 1.0);

    let n = SR as usize * 2;
    let amp = 0.3;
    let sine: Vec<f32> = (0..n)
        .map(|i| (i as f64 / SR as f64 * 440.0 * TAU).sin() as f32 * amp)
        .collect();

    let (out_l, out_r) = render_block(&mut rev, &sine, &sine, 256);

    let warmup = SR as usize / 2;
    let mut sum_lr = 0.0_f64;
    let mut sum_ll = 0.0_f64;
    let mut sum_rr = 0.0_f64;
    for i in warmup..n {
        let l = out_l[i] as f64;
        let r = out_r[i] as f64;
        sum_lr += l * r;
        sum_ll += l * l;
        sum_rr += r * r;
    }
    let corr = if sum_ll > 0.0 && sum_rr > 0.0 {
        sum_lr / (sum_ll.sqrt() * sum_rr.sqrt())
    } else {
        1.0
    };

    assert!(
        corr.abs() < 0.999,
        "L/R correlation {corr:.6} too high — stereo decorrelation broken"
    );
}
