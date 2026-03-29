//! Dattorro Plate Reverb
//!
//! Implementation of the plate reverberator described in:
//! Jon Dattorro, "Effect Design Part 1: Reverberator and Other Filters"
//! Journal of the Audio Engineering Society, Vol. 45, No. 9, 1997.
//! https://ccrma.stanford.edu/~dattorro/EffectDesignPart1.pdf
//!
//! Signal flow: Dattorro 1997, Figure 1
//! Output taps: Dattorro 1997, Table 1
//! Delay lengths: Dattorro 1997, Table 2
//!
//! Zero tolerance: bypass = bit-exact, dry=100% = bit-exact.

use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Parameter IDs — 0-8 match existing Freeverb for compatibility
// ---------------------------------------------------------------------------

const PARAM_PREDELAY: u32 = 0;
const PARAM_DECAY: u32 = 1; // was room_size in Freeverb
const PARAM_DAMPING: u32 = 2;
const PARAM_DIFFUSION: u32 = 3; // maps to input_diffusion_1
const PARAM_WET_LP_FREQ: u32 = 4;
const PARAM_WET_HP_FREQ: u32 = 5;
const PARAM_STEREO_WIDTH: u32 = 6;
const PARAM_DRY_WET: u32 = 7;
const PARAM_BYPASS: u32 = 8;
// New Dattorro-specific params
const PARAM_MOD_DEPTH: u32 = 9;
const PARAM_MOD_RATE: u32 = 10;
const PARAM_INPUT_DIFFUSION_2: u32 = 11;
const PARAM_DECAY_DIFFUSION_1: u32 = 12;
const PARAM_DECAY_DIFFUSION_2: u32 = 13;

const PARAM_COUNT: u32 = 14;

// ---------------------------------------------------------------------------
// Stub: DattorroReverb
// ---------------------------------------------------------------------------

/// Dattorro plate reverb (Dattorro 1997).
///
/// **Current state: STUB.** `process_effect` does passthrough only.
/// This exists so TDD RED tests compile and the expected failures are visible.
pub struct DattorroReverb {
    sample_rate: u32,

    // Parameters (user-facing values)
    predelay_ms: f64,
    decay: f64,
    damping: f64,
    diffusion: f64,
    wet_lp_freq: f64,
    wet_hp_freq: f64,
    stereo_width: f64,
    dry_wet: f64,
    bypass: bool,
    mod_depth: f64,
    mod_rate: f64,
    input_diffusion_2: f64,
    decay_diffusion_1: f64,
    decay_diffusion_2: f64,
}

impl DattorroReverb {
    /// Create a new Dattorro plate reverb at the given sample rate.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            predelay_ms: 0.0,
            decay: 0.5,
            damping: 0.5,
            diffusion: 0.75,
            wet_lp_freq: 20000.0,
            wet_hp_freq: 20.0,
            stereo_width: 1.0,
            dry_wet: 0.5,
            bypass: false,
            mod_depth: 0.5,
            mod_rate: 1.0,
            input_diffusion_2: 0.625,
            decay_diffusion_1: 0.7,
            decay_diffusion_2: 0.5,
        }
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation (STUB — passthrough only)
// ---------------------------------------------------------------------------

impl AudioBackend for DattorroReverb {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "moonlitt-dattorro-reverb",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        // STUB: nothing to clear yet
    }

    // MIDI — reverb is an effect, these are no-ops.
    fn note_on(&mut self, _channel: u8, _note: u8, _velocity: u8) {}
    fn note_off(&mut self, _channel: u8, _note: u8) {}
    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: i16) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn all_notes_off(&mut self) {}

    fn render(&mut self, _left: &mut [f32], _right: &mut [f32]) {
        // Reverb is an effect — use process_effect instead.
    }

    fn process_effect(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        let len = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());

        // STUB: passthrough (bypass and dry_100 tests will pass; others will fail)
        out_l[..len].copy_from_slice(&in_l[..len]);
        out_r[..len].copy_from_slice(&in_r[..len]);
    }

    fn set_volume(&mut self, _volume: f32) {
        // STUB
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    fn param_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        let info = match index {
            PARAM_PREDELAY => ParamInfo {
                id: PARAM_PREDELAY,
                name: "Pre-delay".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 200.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_DECAY => ParamInfo {
                id: PARAM_DECAY,
                name: "Decay".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_DAMPING => ParamInfo {
                id: PARAM_DAMPING,
                name: "Damping".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_DIFFUSION => ParamInfo {
                id: PARAM_DIFFUSION,
                name: "Input Diffusion 1".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.75,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_WET_LP_FREQ => ParamInfo {
                id: PARAM_WET_LP_FREQ,
                name: "Wet LP Freq".into(),
                group: "Reverb".into(),
                min: 200.0,
                max: 20000.0,
                default: 20000.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_WET_HP_FREQ => ParamInfo {
                id: PARAM_WET_HP_FREQ,
                name: "Wet HP Freq".into(),
                group: "Reverb".into(),
                min: 20.0,
                max: 2000.0,
                default: 20.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_STEREO_WIDTH => ParamInfo {
                id: PARAM_STEREO_WIDTH,
                name: "Stereo Width".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_DRY_WET => ParamInfo {
                id: PARAM_DRY_WET,
                name: "Dry/Wet".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_BYPASS => ParamInfo {
                id: PARAM_BYPASS,
                name: "Bypass".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            },
            PARAM_MOD_DEPTH => ParamInfo {
                id: PARAM_MOD_DEPTH,
                name: "Mod Depth".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_MOD_RATE => ParamInfo {
                id: PARAM_MOD_RATE,
                name: "Mod Rate".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 10.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_INPUT_DIFFUSION_2 => ParamInfo {
                id: PARAM_INPUT_DIFFUSION_2,
                name: "Input Diffusion 2".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.625,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_DECAY_DIFFUSION_1 => ParamInfo {
                id: PARAM_DECAY_DIFFUSION_1,
                name: "Decay Diffusion 1".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.7,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            PARAM_DECAY_DIFFUSION_2 => ParamInfo {
                id: PARAM_DECAY_DIFFUSION_2,
                name: "Decay Diffusion 2".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step_count: 0,
                flags: ParamFlags::empty(),
            },
            _ => return None,
        };
        Some(info)
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            PARAM_PREDELAY => Some(self.predelay_ms),
            PARAM_DECAY => Some(self.decay),
            PARAM_DAMPING => Some(self.damping),
            PARAM_DIFFUSION => Some(self.diffusion),
            PARAM_WET_LP_FREQ => Some(self.wet_lp_freq),
            PARAM_WET_HP_FREQ => Some(self.wet_hp_freq),
            PARAM_STEREO_WIDTH => Some(self.stereo_width),
            PARAM_DRY_WET => Some(self.dry_wet),
            PARAM_BYPASS => Some(if self.bypass { 1.0 } else { 0.0 }),
            PARAM_MOD_DEPTH => Some(self.mod_depth),
            PARAM_MOD_RATE => Some(self.mod_rate),
            PARAM_INPUT_DIFFUSION_2 => Some(self.input_diffusion_2),
            PARAM_DECAY_DIFFUSION_1 => Some(self.decay_diffusion_1),
            PARAM_DECAY_DIFFUSION_2 => Some(self.decay_diffusion_2),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PARAM_PREDELAY => self.predelay_ms = value.clamp(0.0, 200.0),
            PARAM_DECAY => self.decay = value.clamp(0.0, 1.0),
            PARAM_DAMPING => self.damping = value.clamp(0.0, 1.0),
            PARAM_DIFFUSION => self.diffusion = value.clamp(0.0, 1.0),
            PARAM_WET_LP_FREQ => self.wet_lp_freq = value.clamp(200.0, 20000.0),
            PARAM_WET_HP_FREQ => self.wet_hp_freq = value.clamp(20.0, 2000.0),
            PARAM_STEREO_WIDTH => self.stereo_width = value.clamp(0.0, 1.0),
            PARAM_DRY_WET => self.dry_wet = value.clamp(0.0, 1.0),
            PARAM_BYPASS => self.bypass = value >= 0.5,
            PARAM_MOD_DEPTH => self.mod_depth = value.clamp(0.0, 1.0),
            PARAM_MOD_RATE => self.mod_rate = value.clamp(0.0, 10.0),
            PARAM_INPUT_DIFFUSION_2 => self.input_diffusion_2 = value.clamp(0.0, 1.0),
            PARAM_DECAY_DIFFUSION_1 => self.decay_diffusion_1 = value.clamp(0.0, 1.0),
            PARAM_DECAY_DIFFUSION_2 => self.decay_diffusion_2 = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            PARAM_PREDELAY => Some(format!("{:.1} ms", value)),
            PARAM_DECAY => Some(format!("{:.0}%", value * 100.0)),
            PARAM_DAMPING => Some(format!("{:.0}%", value * 100.0)),
            PARAM_DIFFUSION => Some(format!("{:.0}%", value * 100.0)),
            PARAM_WET_LP_FREQ => {
                if value >= 1000.0 {
                    Some(format!("{:.1} kHz", value / 1000.0))
                } else {
                    Some(format!("{:.0} Hz", value))
                }
            }
            PARAM_WET_HP_FREQ => Some(format!("{:.0} Hz", value)),
            PARAM_STEREO_WIDTH => Some(format!("{:.0}%", value * 100.0)),
            PARAM_DRY_WET => Some(format!("{:.0}%", value * 100.0)),
            PARAM_BYPASS => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
            PARAM_MOD_DEPTH => Some(format!("{:.0}%", value * 100.0)),
            PARAM_MOD_RATE => Some(format!("{:.2} Hz", value)),
            PARAM_INPUT_DIFFUSION_2 => Some(format!("{:.3}", value)),
            PARAM_DECAY_DIFFUSION_1 => Some(format!("{:.3}", value)),
            PARAM_DECAY_DIFFUSION_2 => Some(format!("{:.3}", value)),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — TDD RED phase
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44100;
    const BLOCK: usize = 512;

    fn make_reverb() -> DattorroReverb {
        DattorroReverb::new(SR)
    }

    /// Helper: process an impulse (1.0 at sample 0) through the reverb.
    fn process_impulse(reverb: &mut DattorroReverb, length: usize) -> (Vec<f32>, Vec<f32>) {
        let mut in_l = vec![0.0f32; length];
        let mut in_r = vec![0.0f32; length];
        in_l[0] = 1.0;
        in_r[0] = 1.0;

        let mut out_l = vec![0.0f32; length];
        let mut out_r = vec![0.0f32; length];
        reverb.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);
        (out_l, out_r)
    }

    /// Helper: process multiple blocks of audio through the reverb, accumulating output.
    fn process_blocks(
        reverb: &mut DattorroReverb,
        in_l: &[f32],
        in_r: &[f32],
        block_size: usize,
    ) -> (Vec<f32>, Vec<f32>) {
        let total = in_l.len();
        let mut out_l = vec![0.0f32; total];
        let mut out_r = vec![0.0f32; total];

        let mut offset = 0;
        while offset < total {
            let end = (offset + block_size).min(total);
            reverb.process_effect(
                &in_l[offset..end],
                &in_r[offset..end],
                &mut out_l[offset..end],
                &mut out_r[offset..end],
            );
            offset = end;
        }
        (out_l, out_r)
    }

    // -----------------------------------------------------------------------
    // Test 1: bypass_bitexact — Dattorro 1997, bypass property
    //
    // When bypass is enabled, output must be BIT-EXACT copy of input.
    // No processing, no latency, no rounding — `==` comparison.
    // -----------------------------------------------------------------------
    #[test]
    fn bypass_bitexact() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_BYPASS, 1.0);

        let in_l: Vec<f32> = (0..BLOCK).map(|i| (i as f32) * 0.001).collect();
        let in_r: Vec<f32> = (0..BLOCK).map(|i| (i as f32) * -0.002).collect();
        let mut out_l = vec![0.0f32; BLOCK];
        let mut out_r = vec![0.0f32; BLOCK];

        rev.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        assert_eq!(out_l, in_l, "bypass L must be bit-exact");
        assert_eq!(out_r, in_r, "bypass R must be bit-exact");
    }

    // -----------------------------------------------------------------------
    // Test 2: dry_100_bitexact — dry/wet=0 means 100% dry
    //
    // With dry_wet=0.0, no wet signal is mixed in. Output = input, bit-exact.
    // -----------------------------------------------------------------------
    #[test]
    fn dry_100_bitexact() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 0.0); // 100% dry

        let in_l: Vec<f32> = (0..BLOCK).map(|i| (i as f32) * 0.001).collect();
        let in_r: Vec<f32> = (0..BLOCK).map(|i| (i as f32) * -0.002).collect();
        let mut out_l = vec![0.0f32; BLOCK];
        let mut out_r = vec![0.0f32; BLOCK];

        rev.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        assert_eq!(out_l, in_l, "dry=100% L must be bit-exact");
        assert_eq!(out_r, in_r, "dry=100% R must be bit-exact");
    }

    // -----------------------------------------------------------------------
    // Test 3: allpass_energy_preservation — Dattorro 1997 §2
    //
    // "An allpass filter has unity gain at all frequencies."
    //
    // A properly implemented Dattorro reverb preserves energy through its
    // allpass diffusion network. We send a burst through 100% wet, collect
    // output over many blocks. Total output energy should be >= 50% of input
    // energy. (Allpass preserves perfectly; the decay/damping path loses some,
    // hence 50% not 100%.)
    // -----------------------------------------------------------------------
    #[test]
    fn allpass_energy_preservation() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0); // 100% wet
        rev.set_param(PARAM_DECAY, 0.9); // high decay to let energy recirculate
        rev.set_param(PARAM_DAMPING, 0.0); // no damping = no HF loss

        // Input: a short burst of noise-like signal (deterministic).
        let burst_len = 512;
        let total_len = SR as usize * 3; // 3 seconds to capture tail
        let mut in_l = vec![0.0f32; total_len];
        let mut in_r = vec![0.0f32; total_len];
        for i in 0..burst_len {
            // Deterministic pseudo-noise using sin
            let v = ((i as f32) * 0.1).sin() * 0.8;
            in_l[i] = v;
            in_r[i] = v;
        }

        let input_energy: f64 = in_l.iter().map(|&s| (s as f64) * (s as f64)).sum();

        let (out_l, out_r) = process_blocks(&mut rev, &in_l, &in_r, 512);

        let output_energy: f64 = out_l
            .iter()
            .zip(out_r.iter())
            .map(|(&l, &r)| {
                let l = l as f64;
                let r = r as f64;
                (l * l + r * r) * 0.5
            })
            .sum();

        // Energy preservation: output should retain at least 50% of input energy.
        // A passthrough stub will output 0 energy when dry_wet=1.0 (if properly
        // implemented — but our stub just passes through, which means it returns
        // the input unchanged, which equals the input energy, which PASSES.
        // However, a correct Dattorro implementation at 100% wet should ALSO pass
        // this. The real distinguishing test is that wet output != dry input, which
        // is tested in wet_100_no_dry_leak.)
        //
        // This test catches implementations that lose too much energy
        // (broken allpass, gain < 1 in feedback paths, etc.)
        assert!(
            output_energy >= input_energy * 0.5,
            "Allpass network should preserve energy (Dattorro 1997 §2). \
             Input energy: {input_energy:.4}, output energy: {output_energy:.4}, \
             ratio: {:.4}",
            output_energy / input_energy
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: predelay_exact — Pre-delay timing
    //
    // With predelay=50ms at 44100Hz, that's exactly 2205 samples.
    // The first 2205 wet output samples must be exactly 0.0.
    // After the predelay, signal must appear.
    //
    // The stub does passthrough (no predelay), so the impulse appears at
    // sample 0 — this test will FAIL.
    // -----------------------------------------------------------------------
    #[test]
    fn predelay_exact() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0); // 100% wet — only reverb output
        rev.set_param(PARAM_PREDELAY, 50.0); // 50 ms
        rev.set_param(PARAM_DECAY, 0.5);

        let predelay_samples = ((50.0 / 1000.0) * SR as f64).round() as usize; // 2205
        assert_eq!(predelay_samples, 2205, "sanity: 50ms at 44100Hz = 2205 samples");

        let total_len = predelay_samples + 4096; // enough room after predelay
        let (out_l, _out_r) = process_impulse(&mut rev, total_len);

        // All samples before predelay must be exactly zero.
        for i in 0..predelay_samples {
            assert_eq!(
                out_l[i], 0.0,
                "sample {i} must be exactly 0.0 during predelay (got {})",
                out_l[i]
            );
        }

        // After predelay, at least some signal must appear (within a reasonable window).
        let post_predelay = &out_l[predelay_samples..];
        let has_signal = post_predelay.iter().any(|&s| s.abs() > f32::EPSILON);
        assert!(
            has_signal,
            "Signal must appear after predelay ({predelay_samples} samples)"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: decay_affects_tail — Dattorro 1997 §3
    //
    // Higher decay coefficient = longer reverb tail.
    // decay=0.3 should produce less tail energy than decay=0.9 in late blocks.
    //
    // The stub does passthrough (identical for both decay values), so the
    // late-block energies will be equal — this test will FAIL.
    // -----------------------------------------------------------------------
    #[test]
    fn decay_affects_tail() {
        // Short decay
        let mut rev_short = make_reverb();
        rev_short.set_param(PARAM_DRY_WET, 1.0);
        rev_short.set_param(PARAM_DECAY, 0.3);

        // Long decay
        let mut rev_long = make_reverb();
        rev_long.set_param(PARAM_DRY_WET, 1.0);
        rev_long.set_param(PARAM_DECAY, 0.9);

        let total_len = SR as usize * 4; // 4 seconds
        let (out_short_l, _) = process_impulse(&mut rev_short, total_len);
        let (out_long_l, _) = process_impulse(&mut rev_long, total_len);

        // Measure energy in the "late tail" region: samples after 1 second.
        let late_start = SR as usize; // 1 second
        let late_energy = |buf: &[f32]| -> f64 {
            buf[late_start..]
                .iter()
                .map(|&s| (s as f64) * (s as f64))
                .sum::<f64>()
        };

        let energy_short = late_energy(&out_short_l);
        let energy_long = late_energy(&out_long_l);

        assert!(
            energy_long > energy_short,
            "Longer decay should produce more tail energy (Dattorro 1997 §3). \
             decay=0.3 late energy: {energy_short:.6}, \
             decay=0.9 late energy: {energy_long:.6}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6: wet_100_no_dry_leak — 100% wet, no dry signal at sample[0]
    //
    // With dry_wet=1.0 and an impulse at sample 0, the output at sample 0
    // should NOT equal the impulse value. In a real Dattorro reverb, the
    // signal passes through 4 input diffusion allpasses before reaching
    // the output taps — sample 0 comes from empty delay lines, not the
    // direct input.
    //
    // The stub does passthrough, so out[0] == in[0] — this test will FAIL.
    // -----------------------------------------------------------------------
    #[test]
    fn wet_100_no_dry_leak() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0); // 100% wet
        rev.set_param(PARAM_PREDELAY, 0.0); // no predelay

        let len = 4096;
        let (out_l, _out_r) = process_impulse(&mut rev, len);

        // The impulse is 1.0 at sample 0. A proper reverb at 100% wet should
        // NOT output 1.0 at sample 0 — the signal goes through diffusers first.
        assert_ne!(
            out_l[0], 1.0,
            "At 100% wet, sample 0 should not equal the dry impulse (1.0). \
             The signal passes through input diffusion allpasses (Dattorro 1997, Figure 1) \
             before reaching output taps."
        );
    }
}
