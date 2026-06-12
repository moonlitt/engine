//! Dattorro Plate Reverb
//!
//! Implementation of the plate reverberator described in:
//! Jon Dattorro, "Effect Design Part 1: Reverberator and Other Filters"
//! Journal of the Audio Engineering Society, Vol. 45, No. 9, 1997.
//! <https://ccrma.stanford.edu/~dattorro/EffectDesignPart1.pdf>
//!
//! Signal flow: Dattorro 1997, Figure 1
//! Output taps: Dattorro 1997, Table 1
//! Delay lengths: Dattorro 1997, Table 2
//!
//! Zero tolerance: bypass = bit-exact, dry=100% = bit-exact.

use super::mod_allpass::ModAllpass;
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
// Dattorro 1997 Table 2 — delay lengths at 44100 Hz
// ---------------------------------------------------------------------------

// Input diffusion allpasses
const INPUT_AP1_LEN: usize = 142;
const INPUT_AP2_LEN: usize = 107;
const INPUT_AP3_LEN: usize = 379;
const INPUT_AP4_LEN: usize = 277;

// Left tank
const L_MOD_AP_LEN: usize = 672;
const L_DELAY1_LEN: usize = 4453;
const L_AP_LEN: usize = 1800;
const L_DELAY2_LEN: usize = 3720;

// Right tank
const R_MOD_AP_LEN: usize = 908;
const R_DELAY1_LEN: usize = 4217;
const R_AP_LEN: usize = 2656;
const R_DELAY2_LEN: usize = 3163;

// ---------------------------------------------------------------------------
// Output tap positions (Dattorro 1997 Table 1)
// ---------------------------------------------------------------------------

// Left output taps
const L_TAP_FROM_L_DELAY1_A: usize = 266;
const L_TAP_FROM_L_DELAY1_B: usize = 2974;
const L_TAP_FROM_L_AP: usize = 1913; // subtracted
const L_TAP_FROM_L_DELAY2: usize = 1996;
const L_TAP_FROM_R_DELAY1: usize = 1990; // subtracted
const L_TAP_FROM_R_AP: usize = 187;  // subtracted
const L_TAP_FROM_R_DELAY2: usize = 1066; // subtracted

// Right output taps
const R_TAP_FROM_R_DELAY1_A: usize = 353;
const R_TAP_FROM_R_DELAY1_B: usize = 3627;
const R_TAP_FROM_R_AP: usize = 1228; // subtracted
const R_TAP_FROM_R_DELAY2: usize = 2111;
const R_TAP_FROM_L_DELAY1: usize = 2673; // subtracted
const R_TAP_FROM_L_AP: usize = 335;  // subtracted
const R_TAP_FROM_L_DELAY2: usize = 121; // subtracted

// ---------------------------------------------------------------------------
// Simple allpass (non-modulated, for input diffusion)
// ---------------------------------------------------------------------------

struct SimpleAllpass {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
}

impl SimpleAllpass {
    fn new(size: usize, feedback: f32) -> Self {
        Self {
            buffer: vec![0.0; size.max(1)],
            index: 0,
            feedback,
        }
    }

    #[inline]
    fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback;
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        // Canonical Schroeder allpass (unity gain at all frequencies, DC included).
        // Form 1 — `buffer = input + g·delayed` — has DC gain (1-g+g²)/(1-g) > 1
        // and silently turns this into a comb-style filter that biases the
        // reverb tank's DC accumulator (caught by analyze: -0.93 DC offset).
        let delayed = self.buffer[self.index];
        let output = -self.feedback * input + delayed;
        self.buffer[self.index] = input + self.feedback * output;
        self.index += 1;
        if self.index >= self.buffer.len() {
            self.index = 0;
        }
        output
    }

    /// Read from the internal buffer at an offset behind the write head.
    /// Offset wraps modularly within the buffer.
    #[inline]
    fn read_at(&self, offset: usize) -> f32 {
        let len = self.buffer.len();
        let off = offset % len;
        let pos = if self.index >= off {
            self.index - off
        } else {
            len - (off - self.index)
        };
        self.buffer[pos]
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}

// ---------------------------------------------------------------------------
// Delay line (simple ring buffer)
// ---------------------------------------------------------------------------

struct DelayLine {
    buffer: Vec<f32>,
    write_index: usize,
}

impl DelayLine {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size.max(1)],
            write_index: 0,
        }
    }

    /// Write a sample and return the oldest sample (full delay length).
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.write_index];
        self.buffer[self.write_index] = input;
        self.write_index += 1;
        if self.write_index >= self.buffer.len() {
            self.write_index = 0;
        }
        output
    }

    /// Read from a tap at `offset` samples behind the write head.
    /// Offset wraps modularly within the buffer.
    #[inline]
    fn read_at(&self, offset: usize) -> f32 {
        let len = self.buffer.len();
        let off = offset % len;
        let pos = if self.write_index >= off {
            self.write_index - off
        } else {
            len - (off - self.write_index)
        };
        self.buffer[pos]
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_index = 0;
    }
}

// ---------------------------------------------------------------------------
// One-pole lowpass (damping filter)
// ---------------------------------------------------------------------------

struct OnePoleLP {
    prev: f32,
    damp: f32, // coefficient: 0=no filtering, 1=max filtering
}

impl OnePoleLP {
    fn new(damp: f32) -> Self {
        Self { prev: 0.0, damp }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        // y[n] = (1 - damp) * x[n] + damp * y[n-1]
        self.prev = (1.0 - self.damp) * input + self.damp * self.prev;
        self.prev
    }

    fn clear(&mut self) {
        self.prev = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Predelay ring buffer
// ---------------------------------------------------------------------------

struct PreDelay {
    buffer: Vec<f32>,
    write_index: usize,
    delay_samples: usize,
}

impl PreDelay {
    fn new(max_samples: usize) -> Self {
        Self {
            buffer: vec![0.0; max_samples.max(1)],
            write_index: 0,
            delay_samples: 0,
        }
    }

    fn set_delay(&mut self, samples: usize) {
        self.delay_samples = samples.min(self.buffer.len() - 1);
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let len = self.buffer.len();
        // Read from delay_samples behind write head
        let read_index = if self.write_index >= self.delay_samples {
            self.write_index - self.delay_samples
        } else {
            len - (self.delay_samples - self.write_index)
        };
        let output = self.buffer[read_index % len];
        self.buffer[self.write_index] = input;
        self.write_index += 1;
        if self.write_index >= len {
            self.write_index = 0;
        }
        output
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_index = 0;
    }
}

// ---------------------------------------------------------------------------
// Helper: scale delay length from 44100 Hz reference
// ---------------------------------------------------------------------------

#[inline]
fn scale_delay(base: usize, sample_rate: u32) -> usize {
    (base as f64 * sample_rate as f64 / 44100.0).round() as usize
}

// ---------------------------------------------------------------------------
// DattorroReverb
// ---------------------------------------------------------------------------

/// Dattorro plate reverb (Dattorro 1997).
///
/// Full implementation of Figure 1: input diffusion, two cross-fed tanks
/// with modulated allpasses, damping lowpass filters, and multi-tap output.
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

    // Pre-delay
    predelay: PreDelay,

    // Input diffusion: 4 allpasses in series
    input_ap1: SimpleAllpass,
    input_ap2: SimpleAllpass,
    input_ap3: SimpleAllpass,
    input_ap4: SimpleAllpass,

    // Left tank
    l_mod_ap: ModAllpass,
    l_delay1: DelayLine,
    l_damp: OnePoleLP,
    l_ap: SimpleAllpass,
    l_delay2: DelayLine,

    // Right tank
    r_mod_ap: ModAllpass,
    r_delay1: DelayLine,
    r_damp: OnePoleLP,
    r_ap: SimpleAllpass,
    r_delay2: DelayLine,

    // Cross-feedback storage
    l_tank_out: f32,
    r_tank_out: f32,

    // LFO state
    lfo_phase: f64,

    // Scaled tap positions
    l_tap_l_delay1_a: usize,
    l_tap_l_delay1_b: usize,
    l_tap_l_ap: usize,
    l_tap_l_delay2: usize,
    l_tap_r_delay1: usize,
    l_tap_r_ap: usize,
    l_tap_r_delay2: usize,

    r_tap_r_delay1_a: usize,
    r_tap_r_delay1_b: usize,
    r_tap_r_ap: usize,
    r_tap_r_delay2: usize,
    r_tap_l_delay1: usize,
    r_tap_l_ap: usize,
    r_tap_l_delay2: usize,
}

impl DattorroReverb {
    /// Create a new Dattorro plate reverb at the given sample rate.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate;

        // Max predelay: 200ms
        let max_predelay = (0.2 * sr as f64).ceil() as usize + 1;

        // Scale all delay lengths
        let input_ap1_len = scale_delay(INPUT_AP1_LEN, sr);
        let input_ap2_len = scale_delay(INPUT_AP2_LEN, sr);
        let input_ap3_len = scale_delay(INPUT_AP3_LEN, sr);
        let input_ap4_len = scale_delay(INPUT_AP4_LEN, sr);

        let l_mod_ap_len = scale_delay(L_MOD_AP_LEN, sr);
        let l_delay1_len = scale_delay(L_DELAY1_LEN, sr);
        let l_ap_len = scale_delay(L_AP_LEN, sr);
        let l_delay2_len = scale_delay(L_DELAY2_LEN, sr);

        let r_mod_ap_len = scale_delay(R_MOD_AP_LEN, sr);
        let r_delay1_len = scale_delay(R_DELAY1_LEN, sr);
        let r_ap_len = scale_delay(R_AP_LEN, sr);
        let r_delay2_len = scale_delay(R_DELAY2_LEN, sr);

        // Default coefficients
        let diff1: f32 = 0.75;
        let diff2: f32 = 0.625;
        let decay_diff1: f32 = 0.7;
        let decay_diff2: f32 = 0.5;
        let damp: f32 = 0.5;

        let mut reverb = Self {
            sample_rate: sr,

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

            predelay: PreDelay::new(max_predelay),

            input_ap1: SimpleAllpass::new(input_ap1_len, diff1),
            input_ap2: SimpleAllpass::new(input_ap2_len, diff1),
            input_ap3: SimpleAllpass::new(input_ap3_len, diff2),
            input_ap4: SimpleAllpass::new(input_ap4_len, diff2),

            l_mod_ap: ModAllpass::new(l_mod_ap_len, decay_diff1),
            l_delay1: DelayLine::new(l_delay1_len),
            l_damp: OnePoleLP::new(damp),
            l_ap: SimpleAllpass::new(l_ap_len, decay_diff2),
            l_delay2: DelayLine::new(l_delay2_len),

            r_mod_ap: ModAllpass::new(r_mod_ap_len, decay_diff1),
            r_delay1: DelayLine::new(r_delay1_len),
            r_damp: OnePoleLP::new(damp),
            r_ap: SimpleAllpass::new(r_ap_len, decay_diff2),
            r_delay2: DelayLine::new(r_delay2_len),

            l_tank_out: 0.0,
            r_tank_out: 0.0,

            lfo_phase: 0.0,

            // Tap positions (will be set below)
            l_tap_l_delay1_a: 0,
            l_tap_l_delay1_b: 0,
            l_tap_l_ap: 0,
            l_tap_l_delay2: 0,
            l_tap_r_delay1: 0,
            l_tap_r_ap: 0,
            l_tap_r_delay2: 0,

            r_tap_r_delay1_a: 0,
            r_tap_r_delay1_b: 0,
            r_tap_r_ap: 0,
            r_tap_r_delay2: 0,
            r_tap_l_delay1: 0,
            r_tap_l_ap: 0,
            r_tap_l_delay2: 0,
        };

        reverb.update_taps();
        reverb
    }

    /// Recalculate scaled tap positions for current sample rate.
    fn update_taps(&mut self) {
        let sr = self.sample_rate;

        self.l_tap_l_delay1_a = scale_delay(L_TAP_FROM_L_DELAY1_A, sr);
        self.l_tap_l_delay1_b = scale_delay(L_TAP_FROM_L_DELAY1_B, sr);
        self.l_tap_l_ap = scale_delay(L_TAP_FROM_L_AP, sr);
        self.l_tap_l_delay2 = scale_delay(L_TAP_FROM_L_DELAY2, sr);
        self.l_tap_r_delay1 = scale_delay(L_TAP_FROM_R_DELAY1, sr);
        self.l_tap_r_ap = scale_delay(L_TAP_FROM_R_AP, sr);
        self.l_tap_r_delay2 = scale_delay(L_TAP_FROM_R_DELAY2, sr);

        self.r_tap_r_delay1_a = scale_delay(R_TAP_FROM_R_DELAY1_A, sr);
        self.r_tap_r_delay1_b = scale_delay(R_TAP_FROM_R_DELAY1_B, sr);
        self.r_tap_r_ap = scale_delay(R_TAP_FROM_R_AP, sr);
        self.r_tap_r_delay2 = scale_delay(R_TAP_FROM_R_DELAY2, sr);
        self.r_tap_l_delay1 = scale_delay(R_TAP_FROM_L_DELAY1, sr);
        self.r_tap_l_ap = scale_delay(R_TAP_FROM_L_AP, sr);
        self.r_tap_l_delay2 = scale_delay(R_TAP_FROM_L_DELAY2, sr);
    }

    /// Update predelay from ms to samples.
    fn update_predelay(&mut self) {
        let samples =
            (self.predelay_ms / 1000.0 * self.sample_rate as f64).round() as usize;
        self.predelay.set_delay(samples);
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
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
        // Clear all delay buffers
        self.predelay.clear();
        self.input_ap1.clear();
        self.input_ap2.clear();
        self.input_ap3.clear();
        self.input_ap4.clear();
        self.l_mod_ap.clear();
        self.l_delay1.clear();
        self.l_damp.clear();
        self.l_ap.clear();
        self.l_delay2.clear();
        self.r_mod_ap.clear();
        self.r_delay1.clear();
        self.r_damp.clear();
        self.r_ap.clear();
        self.r_delay2.clear();
        self.l_tank_out = 0.0;
        self.r_tank_out = 0.0;
        self.lfo_phase = 0.0;
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

        // Bypass: bit-exact copy
        if self.bypass {
            out_l[..len].copy_from_slice(&in_l[..len]);
            out_r[..len].copy_from_slice(&in_r[..len]);
            return;
        }

        // dry_wet=0 means 100% dry: bit-exact copy
        if self.dry_wet == 0.0 {
            out_l[..len].copy_from_slice(&in_l[..len]);
            out_r[..len].copy_from_slice(&in_r[..len]);
            return;
        }

        let decay = self.decay as f32;
        let wet = self.dry_wet as f32;
        let dry = 1.0 - wet;
        let width = self.stereo_width as f32;

        // LFO parameters: mod_depth is 0-1, scaled to 0-16 samples max excursion
        let mod_depth = self.mod_depth as f32 * 16.0;
        let lfo_inc = self.mod_rate / self.sample_rate as f64;

        // Update damping coefficient
        let damp_coeff = self.damping as f32;
        self.l_damp.damp = damp_coeff;
        self.r_damp.damp = damp_coeff;

        // Update input diffusion coefficients
        let diff1 = self.diffusion as f32;
        let diff2 = self.input_diffusion_2 as f32;
        self.input_ap1.set_feedback(diff1);
        self.input_ap2.set_feedback(diff1);
        self.input_ap3.set_feedback(diff2);
        self.input_ap4.set_feedback(diff2);

        // Update decay diffusion coefficients
        let dd1 = self.decay_diffusion_1 as f32;
        let dd2 = self.decay_diffusion_2 as f32;
        self.l_mod_ap.set_feedback(dd1);
        self.r_mod_ap.set_feedback(dd1);
        self.l_ap.set_feedback(dd2);
        self.r_ap.set_feedback(dd2);

        for i in 0..len {
            // 1. Mono sum input
            let mono_in = (in_l[i] + in_r[i]) * 0.5;

            // 2. Pre-delay
            let pre_delayed = self.predelay.process(mono_in);

            // 3. Input diffusion: 4 allpasses in series
            let d1 = self.input_ap1.process(pre_delayed);
            let d2 = self.input_ap2.process(d1);
            let d3 = self.input_ap3.process(d2);
            let diffused = self.input_ap4.process(d3);

            // 4. LFO for tank modulation
            let lfo_l = (self.lfo_phase * std::f64::consts::TAU).sin() as f32 * mod_depth;
            let lfo_r =
                ((self.lfo_phase + 0.5) * std::f64::consts::TAU).sin() as f32 * mod_depth;
            self.lfo_phase += lfo_inc;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }

            // 5. Left tank: input = diffused + decay * right_tank_out (cross-feedback)
            let l_in = diffused + decay * self.r_tank_out;
            let l_mod = self.l_mod_ap.process(l_in, lfo_l);
            let l_d1 = self.l_delay1.process(l_mod);
            let l_damped = self.l_damp.process(l_d1);
            let l_decayed = l_damped * decay;
            let l_ap_out = self.l_ap.process(l_decayed);
            let l_d2 = self.l_delay2.process(l_ap_out);
            // Store for cross-feedback next sample
            let new_l_tank_out = l_d2;

            // 6. Right tank: input = diffused + decay * left_tank_out (cross-feedback)
            let r_in = diffused + decay * self.l_tank_out;
            let r_mod = self.r_mod_ap.process(r_in, lfo_r);
            let r_d1 = self.r_delay1.process(r_mod);
            let r_damped = self.r_damp.process(r_d1);
            let r_decayed = r_damped * decay;
            let r_ap_out = self.r_ap.process(r_decayed);
            let r_d2 = self.r_delay2.process(r_ap_out);
            let new_r_tank_out = r_d2;

            // Update cross-feedback state
            self.l_tank_out = new_l_tank_out;
            self.r_tank_out = new_r_tank_out;

            // 7. Output taps (Dattorro 1997, Table 1)
            // Left output = sum of taps from both tanks
            let tap_l = self.l_delay1.read_at(self.l_tap_l_delay1_a)
                + self.l_delay1.read_at(self.l_tap_l_delay1_b)
                - self.l_ap.read_at(self.l_tap_l_ap)
                + self.l_delay2.read_at(self.l_tap_l_delay2)
                - self.r_delay1.read_at(self.l_tap_r_delay1)
                - self.r_ap.read_at(self.l_tap_r_ap)
                - self.r_delay2.read_at(self.l_tap_r_delay2);

            // Right output = sum of taps from both tanks (mirror)
            let tap_r = self.r_delay1.read_at(self.r_tap_r_delay1_a)
                + self.r_delay1.read_at(self.r_tap_r_delay1_b)
                - self.r_ap.read_at(self.r_tap_r_ap)
                + self.r_delay2.read_at(self.r_tap_r_delay2)
                - self.l_delay1.read_at(self.r_tap_l_delay1)
                - self.l_ap.read_at(self.r_tap_l_ap)
                - self.l_delay2.read_at(self.r_tap_l_delay2);

            // 8. Stereo width crossfade
            let wet_l = tap_l * (0.5 + 0.5 * width) + tap_r * (0.5 - 0.5 * width);
            let wet_r = tap_r * (0.5 + 0.5 * width) + tap_l * (0.5 - 0.5 * width);

            // 9. Dry/wet mix
            out_l[i] = in_l[i] * dry + wet_l * wet;
            out_r[i] = in_r[i] * dry + wet_r * wet;
        }
    }

    fn set_volume(&mut self, _volume: f32) {}

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
            PARAM_PREDELAY => {
                self.predelay_ms = value.clamp(0.0, 200.0);
                self.update_predelay();
            }
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
