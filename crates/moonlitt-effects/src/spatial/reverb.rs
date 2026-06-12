use super::allpass::AllpassFilter;
use super::comb::CombFilter;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};

// ---------------------------------------------------------------------------
// Freeverb tuning constants (at 44100 Hz)
// ---------------------------------------------------------------------------

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const BASE_RATE: f32 = 44100.0;

// ---------------------------------------------------------------------------
// One-pole filter (used for wet LP / HP)
// ---------------------------------------------------------------------------

struct OnePole {
    a: f32,
    b: f32,
    z: f32,
}

impl OnePole {
    fn lowpass(freq: f64, sample_rate: u32) -> Self {
        let a = (-2.0 * std::f64::consts::PI * freq / sample_rate as f64).exp() as f32;
        Self {
            a,
            b: 1.0 - a,
            z: 0.0,
        }
    }

    fn set_freq_lp(&mut self, freq: f64, sample_rate: u32) {
        self.a = (-2.0 * std::f64::consts::PI * freq / sample_rate as f64).exp() as f32;
        self.b = 1.0 - self.a;
    }

    #[inline]
    fn process_lp(&mut self, input: f32) -> f32 {
        self.z = self.a * self.z + self.b * input;
        self.z
    }

    #[inline]
    fn process_hp(&mut self, input: f32) -> f32 {
        self.z = self.a * self.z + self.b * input;
        input - self.z
    }

    fn clear(&mut self) {
        self.z = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Parameter IDs
// ---------------------------------------------------------------------------

const PARAM_PREDELAY: u32 = 0;
const PARAM_ROOM_SIZE: u32 = 1;
const PARAM_DAMPING: u32 = 2;
const PARAM_DIFFUSION: u32 = 3;
const PARAM_WET_LP_FREQ: u32 = 4;
const PARAM_WET_HP_FREQ: u32 = 5;
const PARAM_STEREO_WIDTH: u32 = 6;
const PARAM_DRY_WET: u32 = 7;
const PARAM_BYPASS: u32 = 8;

// ---------------------------------------------------------------------------
// Reverb
// ---------------------------------------------------------------------------

/// Stereo reverb based on the Freeverb algorithm.
///
/// 8 parallel lowpass-feedback comb filters + 4 series allpass filters
/// per channel, with pre-delay, wet EQ, stereo width, and dry/wet mix.
pub struct Reverb {
    sample_rate: u32,

    // Comb + allpass per channel
    combs_left: Vec<CombFilter>,
    combs_right: Vec<CombFilter>,
    allpasses_left: Vec<AllpassFilter>,
    allpasses_right: Vec<AllpassFilter>,

    // Pre-delay
    predelay_buffer_l: Vec<f32>,
    predelay_buffer_r: Vec<f32>,
    predelay_index: usize,
    predelay_samples: usize,

    // Wet signal filters (index 0 = left, 1 = right)
    wet_lp: [OnePole; 2],
    wet_hp: [OnePole; 2],

    // Parameters (user-facing values)
    predelay_ms: f64,
    room_size: f64,
    damping: f64,
    diffusion: f64,
    wet_lp_freq: f64,
    wet_hp_freq: f64,
    stereo_width: f64,
    dry_wet: f64,
    bypass: bool,

    // Derived
    feedback: f32,
    damp1: f32,
    volume: f32,
}

/// Scale a delay length from 44100 Hz to the target sample rate.
fn scale_delay(base: usize, sample_rate: u32) -> usize {
    ((base as f64) * (sample_rate as f64) / (BASE_RATE as f64)).round() as usize
}

/// Maximum pre-delay buffer size for a given sample rate (200 ms).
fn max_predelay_samples(sample_rate: u32) -> usize {
    (0.2 * sample_rate as f64).ceil() as usize
}

impl Reverb {
    /// Create a new stereo reverb at the given sample rate.
    pub fn new(sample_rate: u32) -> Self {
        let combs_left: Vec<CombFilter> = COMB_TUNING
            .iter()
            .map(|&t| CombFilter::new(scale_delay(t, sample_rate)))
            .collect();

        let combs_right: Vec<CombFilter> = COMB_TUNING
            .iter()
            .map(|&t| CombFilter::new(scale_delay(t + STEREO_SPREAD, sample_rate)))
            .collect();

        let allpasses_left: Vec<AllpassFilter> = ALLPASS_TUNING
            .iter()
            .map(|&t| AllpassFilter::new(scale_delay(t, sample_rate)))
            .collect();

        let allpasses_right: Vec<AllpassFilter> = ALLPASS_TUNING
            .iter()
            .map(|&t| AllpassFilter::new(scale_delay(t + STEREO_SPREAD, sample_rate)))
            .collect();

        let pd_size = max_predelay_samples(sample_rate);

        let default_lp = 20000.0;
        let default_hp = 20.0;

        let mut reverb = Self {
            sample_rate,
            combs_left,
            combs_right,
            allpasses_left,
            allpasses_right,
            predelay_buffer_l: vec![0.0; pd_size],
            predelay_buffer_r: vec![0.0; pd_size],
            predelay_index: 0,
            predelay_samples: 0,
            wet_lp: [
                OnePole::lowpass(default_lp, sample_rate),
                OnePole::lowpass(default_lp, sample_rate),
            ],
            wet_hp: [
                OnePole::lowpass(default_hp, sample_rate),
                OnePole::lowpass(default_hp, sample_rate),
            ],
            predelay_ms: 0.0,
            room_size: 0.5,
            damping: 0.5,
            diffusion: 0.5,
            wet_lp_freq: default_lp,
            wet_hp_freq: default_hp,
            stereo_width: 1.0,
            dry_wet: 0.5,
            bypass: false,
            feedback: 0.0,
            damp1: 0.0,
            volume: 1.0,
        };

        reverb.update_derived();
        reverb
    }

    /// Recalculate derived parameters from user-facing values.
    fn update_derived(&mut self) {
        // Room size -> feedback
        self.feedback = (self.room_size * 0.28 + 0.7) as f32;

        // Damping -> damp1
        self.damp1 = (self.damping * 0.4) as f32;

        // Pre-delay in samples
        self.predelay_samples =
            ((self.predelay_ms / 1000.0) * self.sample_rate as f64).round() as usize;
        let max_pd = self.predelay_buffer_l.len();
        if self.predelay_samples >= max_pd {
            self.predelay_samples = max_pd.saturating_sub(1);
        }

        // Push to comb filters
        for comb in self
            .combs_left
            .iter_mut()
            .chain(self.combs_right.iter_mut())
        {
            comb.set_feedback(self.feedback);
            comb.set_damp(self.damp1);
        }

        // Wet EQ
        for pole in &mut self.wet_lp {
            pole.set_freq_lp(self.wet_lp_freq, self.sample_rate);
        }
        for pole in &mut self.wet_hp {
            pole.set_freq_lp(self.wet_hp_freq, self.sample_rate);
        }
    }

    /// Process a single stereo sample pair through the reverb.
    #[inline]
    fn process_sample(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let pd_len = self.predelay_buffer_l.len();

        // --- Pre-delay: write current input, read delayed ---
        self.predelay_buffer_l[self.predelay_index] = in_l;
        self.predelay_buffer_r[self.predelay_index] = in_r;

        let read_idx = if self.predelay_index >= self.predelay_samples {
            self.predelay_index - self.predelay_samples
        } else {
            pd_len + self.predelay_index - self.predelay_samples
        };
        let delayed_l = self.predelay_buffer_l[read_idx];
        let delayed_r = self.predelay_buffer_r[read_idx];

        self.predelay_index = (self.predelay_index + 1) % pd_len;

        // Mono sum for comb input (standard Freeverb uses mono input)
        let mono_in = (delayed_l + delayed_r) * 0.5;

        // --- 8 comb filters in parallel, sum outputs ---
        let mut sum_l = 0.0f32;
        let mut sum_r = 0.0f32;
        for i in 0..8 {
            sum_l += self.combs_left[i].process(mono_in);
            sum_r += self.combs_right[i].process(mono_in);
        }

        // --- 4 allpass filters in series ---
        let mut wet_l = sum_l;
        let mut wet_r = sum_r;
        for i in 0..4 {
            wet_l = self.allpasses_left[i].process(wet_l);
            wet_r = self.allpasses_right[i].process(wet_r);
        }

        // --- Wet LP / HP ---
        wet_l = self.wet_lp[0].process_lp(wet_l);
        wet_r = self.wet_lp[1].process_lp(wet_r);
        wet_l = self.wet_hp[0].process_hp(wet_l);
        wet_r = self.wet_hp[1].process_hp(wet_r);

        // --- Stereo width ---
        let w = self.stereo_width as f32;
        let wet1 = w * 0.5 + 0.5;
        let wet2 = 0.5 - w * 0.5;
        let out_l = wet_l * wet1 + wet_r * wet2;
        let out_r = wet_r * wet1 + wet_l * wet2;

        // --- Dry/wet mix ---
        let dry = (1.0 - self.dry_wet) as f32;
        let wet = self.dry_wet as f32;
        (dry * in_l + wet * out_l, dry * in_r + wet * out_r)
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for Reverb {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "moonlitt-reverb",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        // Clear all internal state.
        for c in self
            .combs_left
            .iter_mut()
            .chain(self.combs_right.iter_mut())
        {
            c.clear();
        }
        for a in self
            .allpasses_left
            .iter_mut()
            .chain(self.allpasses_right.iter_mut())
        {
            a.clear();
        }
        self.predelay_buffer_l.fill(0.0);
        self.predelay_buffer_r.fill(0.0);
        self.predelay_index = 0;
        for p in &mut self.wet_lp {
            p.clear();
        }
        for p in &mut self.wet_hp {
            p.clear();
        }
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

    fn process_effect(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let len = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());

        if self.bypass || self.dry_wet == 0.0 {
            out_l[..len].copy_from_slice(&in_l[..len]);
            out_r[..len].copy_from_slice(&in_r[..len]);
            return;
        }

        for i in 0..len {
            let (ol, or) = self.process_sample(in_l[i], in_r[i]);
            out_l[i] = ol;
            out_r[i] = or;
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        0
    }

    fn param_count(&self) -> u32 {
        9
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
            PARAM_ROOM_SIZE => ParamInfo {
                id: PARAM_ROOM_SIZE,
                name: "Room Size".into(),
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
                name: "Diffusion".into(),
                group: "Reverb".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
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
            _ => return None,
        };
        Some(info)
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            PARAM_PREDELAY => Some(self.predelay_ms),
            PARAM_ROOM_SIZE => Some(self.room_size),
            PARAM_DAMPING => Some(self.damping),
            PARAM_DIFFUSION => Some(self.diffusion),
            PARAM_WET_LP_FREQ => Some(self.wet_lp_freq),
            PARAM_WET_HP_FREQ => Some(self.wet_hp_freq),
            PARAM_STEREO_WIDTH => Some(self.stereo_width),
            PARAM_DRY_WET => Some(self.dry_wet),
            PARAM_BYPASS => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            PARAM_PREDELAY => self.predelay_ms = value.clamp(0.0, 200.0),
            PARAM_ROOM_SIZE => self.room_size = value.clamp(0.0, 1.0),
            PARAM_DAMPING => self.damping = value.clamp(0.0, 1.0),
            PARAM_DIFFUSION => self.diffusion = value.clamp(0.0, 1.0),
            PARAM_WET_LP_FREQ => self.wet_lp_freq = value.clamp(200.0, 20000.0),
            PARAM_WET_HP_FREQ => self.wet_hp_freq = value.clamp(20.0, 2000.0),
            PARAM_STEREO_WIDTH => self.stereo_width = value.clamp(0.0, 1.0),
            PARAM_DRY_WET => self.dry_wet = value.clamp(0.0, 1.0),
            PARAM_BYPASS => self.bypass = value >= 0.5,
            _ => return,
        }
        self.update_derived();
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            PARAM_PREDELAY => Some(format!("{:.1} ms", value)),
            PARAM_ROOM_SIZE => Some(format!("{:.0}%", value * 100.0)),
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
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44100;
    const BLOCK: usize = 512;

    fn make_reverb() -> Reverb {
        Reverb::new(SR)
    }

    fn process_impulse(reverb: &mut Reverb, length: usize) -> (Vec<f32>, Vec<f32>) {
        let mut in_l = vec![0.0f32; length];
        let mut in_r = vec![0.0f32; length];
        in_l[0] = 1.0;
        in_r[0] = 1.0;

        let mut out_l = vec![0.0f32; length];
        let mut out_r = vec![0.0f32; length];
        reverb.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);
        (out_l, out_r)
    }

    #[test]
    fn test_bypass_is_bitexact() {
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

    #[test]
    fn test_dry_100_is_bitexact() {
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

    #[test]
    fn test_wet_100_no_dry_leak() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0); // 100% wet
        rev.set_param(PARAM_PREDELAY, 0.0);

        let len = 4096;
        let (out_l, _out_r) = process_impulse(&mut rev, len);

        // At sample 0, the comb filters read from zeroed buffers,
        // so the output should be zero or near-zero (no dry leak).
        assert!(
            out_l[0].abs() < 1e-6,
            "sample 0 should have no dry signal, got {}",
            out_l[0]
        );
    }

    #[test]
    fn test_impulse_response_has_tail() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0);
        rev.set_param(PARAM_ROOM_SIZE, 0.8);

        let len = SR as usize * 3; // 3 seconds
        let (out_l, _out_r) = process_impulse(&mut rev, len);

        // Find last sample above -60 dB (threshold = 10^(-60/20) = 0.001).
        let threshold = 0.001f32;
        let last_above = out_l
            .iter()
            .rposition(|&s| s.abs() > threshold)
            .unwrap_or(0);

        // With room_size=0.8, the tail should be well over 0.5 seconds.
        let min_tail_samples = (SR as f64 * 0.5) as usize;
        assert!(
            last_above > min_tail_samples,
            "Tail too short: last sample above -60dB at {last_above}, expected > {min_tail_samples}"
        );
    }

    #[test]
    fn test_stereo_width_mono() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0);
        rev.set_param(PARAM_STEREO_WIDTH, 0.0); // mono

        let len = 2048;
        // Feed signal only on L channel.
        let mut in_l = vec![0.0f32; len];
        let in_r = vec![0.0f32; len];
        in_l[0] = 1.0;

        let mut out_l = vec![0.0f32; len];
        let mut out_r = vec![0.0f32; len];
        rev.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        // With width=0 and mono input, output L and R should be identical.
        // The comb filters have different delay lengths for L/R (stereo spread),
        // so the wet signals are different. But with width=0:
        //   out_l = wet_l * 0.5 + wet_r * 0.5
        //   out_r = wet_r * 0.5 + wet_l * 0.5
        // These are identical.
        for i in 0..len {
            assert!(
                (out_l[i] - out_r[i]).abs() < 1e-6,
                "width=0 should give mono output, sample {i}: L={} R={}",
                out_l[i],
                out_r[i]
            );
        }
    }

    #[test]
    fn test_stereo_width_full() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0);
        rev.set_param(PARAM_STEREO_WIDTH, 1.0); // full stereo

        let len = 4096;
        // Feed signal only on L channel.
        let mut in_l = vec![0.0f32; len];
        let in_r = vec![0.0f32; len];
        in_l[0] = 1.0;

        let mut out_l = vec![0.0f32; len];
        let mut out_r = vec![0.0f32; len];
        rev.process_effect(&in_l, &in_r, &mut out_l, &mut out_r);

        // With width=1.0 and stereo spread, L and R should differ.
        let mut found_diff = false;
        for i in 0..len {
            if (out_l[i] - out_r[i]).abs() > 1e-6 {
                found_diff = true;
                break;
            }
        }
        assert!(found_diff, "width=1.0 should produce stereo separation");
    }

    #[test]
    fn test_predelay() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_DRY_WET, 1.0);
        rev.set_param(PARAM_PREDELAY, 10.0); // 10 ms

        let predelay_samples = ((10.0 / 1000.0) * SR as f64).round() as usize; // 441

        let len = 2048;
        let (out_l, _out_r) = process_impulse(&mut rev, len);

        // The first `predelay_samples` of the wet output should be zero
        // because the comb filters haven't received any input yet.
        for i in 0..predelay_samples {
            assert!(
                out_l[i].abs() < 1e-10,
                "sample {i} should be zero during predelay, got {}",
                out_l[i]
            );
        }
    }

    #[test]
    fn test_room_size_affects_decay() {
        // Small room.
        let mut rev_small = make_reverb();
        rev_small.set_param(PARAM_DRY_WET, 1.0);
        rev_small.set_param(PARAM_ROOM_SIZE, 0.2);

        // Large room.
        let mut rev_large = make_reverb();
        rev_large.set_param(PARAM_DRY_WET, 1.0);
        rev_large.set_param(PARAM_ROOM_SIZE, 0.9);

        let len = SR as usize * 5; // 5 seconds
        let (out_small, _) = process_impulse(&mut rev_small, len);
        let (out_large, _) = process_impulse(&mut rev_large, len);

        let threshold = 0.001f32; // -60 dB

        let decay_small = out_small
            .iter()
            .rposition(|&s| s.abs() > threshold)
            .unwrap_or(0);

        let decay_large = out_large
            .iter()
            .rposition(|&s| s.abs() > threshold)
            .unwrap_or(0);

        assert!(
            decay_large > decay_small,
            "Larger room should decay slower: small={decay_small} large={decay_large}"
        );
    }

    #[test]
    fn test_param_count_and_info() {
        let rev = make_reverb();
        assert_eq!(rev.param_count(), 9);

        for i in 0..9 {
            let info = rev.param_info(i);
            assert!(info.is_some(), "param_info({i}) should exist");
        }
        assert!(rev.param_info(9).is_none());
    }

    #[test]
    fn test_param_roundtrip() {
        let mut rev = make_reverb();
        rev.set_param(PARAM_ROOM_SIZE, 0.75);
        assert_eq!(rev.get_param(PARAM_ROOM_SIZE), Some(0.75));

        rev.set_param(PARAM_PREDELAY, 42.0);
        assert_eq!(rev.get_param(PARAM_PREDELAY), Some(42.0));

        rev.set_param(PARAM_BYPASS, 1.0);
        assert_eq!(rev.get_param(PARAM_BYPASS), Some(1.0));
    }
}
