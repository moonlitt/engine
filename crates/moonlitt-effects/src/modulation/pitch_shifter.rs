//! Dual-mode pitch shifter: Granular synthesis + Phase Vocoder.
//!
//! Implements the `AudioBackend` trait from `moonlitt-core` as an audio
//! effect processor. All internal arithmetic is f64; only the audio I/O
//! boundary touches f32.
//!
//! ## Granular Mode
//!
//! ```text
//! input -> [circular buffer] -> [extract overlapping grains with Hann window]
//!       -> [resample at playback_rate] -> [overlap-add] -> output
//! ```
//!
//! 4 overlapping grains with Hann windows and random position jitter.
//!
//! ## Phase Vocoder Mode
//!
//! ```text
//! input -> [STFT: windowed FFT] -> [phase processing] -> [bin resampling]
//!       -> [ISTFT: IFFT + overlap-add] -> output
//! ```
//!
//! FFT sizes: 1024, 2048, 4096 with 4x overlap.

use std::f64::consts::PI;

use crate::common::ParamSmoother;
use moonlitt_core::{AudioBackend, BackendInfo, BackendType, ParamFlags, ParamInfo};
use rustfft::{num_complex::Complex, FftPlanner};

/// Maximum circular buffer length in samples (~100ms at 192kHz).
const MAX_BUFFER_SAMPLES: usize = 19200;

/// Number of overlapping grains in granular mode.
const GRAIN_COUNT: usize = 4;

/// Smoothing ramp time in milliseconds for parameter changes.
const SMOOTH_MS: f64 = 5.0;

// ---------------------------------------------------------------------------
// Hann window generation
// ---------------------------------------------------------------------------

/// Generate a Hann window of the given length.
fn hann_window(len: usize) -> Vec<f64> {
    if len <= 1 {
        return vec![1.0; len];
    }
    (0..len)
        .map(|n| 0.5 * (1.0 - (2.0 * PI * n as f64 / (len - 1) as f64).cos()))
        .collect()
}

// ---------------------------------------------------------------------------
// Simple xorshift64 PRNG for grain jitter
// ---------------------------------------------------------------------------

struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// Return a random f64 in [0, 1).
    fn next_f64(&mut self) -> f64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        (self.state as f64) / (u64::MAX as f64)
    }
}

// ---------------------------------------------------------------------------
// Grain — one grain voice for the granular mode
// ---------------------------------------------------------------------------

struct Grain {
    /// Fractional read position into the circular buffer.
    read_pos: f64,
    /// How many samples this grain has been active.
    samples_elapsed: usize,
    /// Total grain length in samples.
    grain_length: usize,
    /// Precomputed Hann window for this grain.
    window: Vec<f64>,
    /// Whether this grain is currently producing output.
    active: bool,
}

impl Grain {
    fn new() -> Self {
        Self {
            read_pos: 0.0,
            samples_elapsed: 0,
            grain_length: 0,
            window: Vec::new(),
            active: false,
        }
    }

    /// Start (or restart) this grain at the given position with the given length.
    fn start(&mut self, read_pos: f64, grain_length: usize) {
        self.read_pos = read_pos;
        self.samples_elapsed = 0;
        self.grain_length = grain_length;
        if self.window.len() != grain_length {
            self.window = hann_window(grain_length);
        }
        self.active = true;
    }

    /// Read one sample from the circular buffer, applying the Hann window
    /// and advancing the read pointer by `playback_rate`.
    /// Returns the windowed sample value, or 0 if the grain is inactive.
    #[inline]
    fn read_sample(&mut self, buffer: &[f64], playback_rate: f64) -> f64 {
        if !self.active || self.grain_length == 0 {
            return 0.0;
        }

        let buf_len = buffer.len();
        let window_val = self.window[self.samples_elapsed];

        // Linear interpolation in the circular buffer
        let pos = self.read_pos.rem_euclid(buf_len as f64);
        let idx0 = pos as usize % buf_len;
        let idx1 = (idx0 + 1) % buf_len;
        let frac = pos - pos.floor();
        let sample = buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac;

        self.read_pos += playback_rate;
        self.samples_elapsed += 1;

        if self.samples_elapsed >= self.grain_length {
            self.active = false;
        }

        sample * window_val
    }
}

// ---------------------------------------------------------------------------
// GranularEngine — manages 4 overlapping grains
// ---------------------------------------------------------------------------

struct GranularEngine {
    /// Circular input buffer (mono).
    buffer: Vec<f64>,
    /// Current write position into the circular buffer.
    write_pos: usize,
    /// The 4 grain voices.
    grains: [Grain; GRAIN_COUNT],
    /// Sample counter for scheduling new grains.
    sample_counter: usize,
    /// Interval between grain starts (in samples).
    grain_interval: usize,
    /// Current grain length in samples.
    grain_length: usize,
    /// PRNG for position jitter.
    rng: Xorshift64,
}

impl GranularEngine {
    fn new() -> Self {
        Self {
            buffer: vec![0.0; MAX_BUFFER_SAMPLES],
            write_pos: 0,
            grains: [Grain::new(), Grain::new(), Grain::new(), Grain::new()],
            sample_counter: 0,
            grain_interval: 441,
            grain_length: 882,
            rng: Xorshift64::new(0xDEAD_BEEF_CAFE_BABE),
        }
    }

    /// Update grain parameters when sample rate or grain_size_ms changes.
    fn update_params(&mut self, sample_rate: f64, grain_size_ms: f64) {
        let grain_length = ((grain_size_ms * 0.001 * sample_rate) as usize).max(2);
        let grain_interval = (grain_length / GRAIN_COUNT).max(1);
        self.grain_length = grain_length;
        self.grain_interval = grain_interval;
    }

    /// Process one mono sample. Returns the pitch-shifted output.
    fn process_sample(&mut self, input: f64, playback_rate: f64) -> f64 {
        let buf_len = self.buffer.len();

        // Write input to circular buffer
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % buf_len;

        // Schedule new grains at regular intervals
        if self.sample_counter.is_multiple_of(self.grain_interval) {
            // Find a free grain slot
            if let Some(grain) = self.grains.iter_mut().find(|g| !g.active) {
                // Start position: current write_pos minus grain_length, with jitter
                let jitter = (self.rng.next_f64() - 0.5) * 0.2 * self.grain_length as f64;
                let start_pos =
                    (self.write_pos as f64 - self.grain_length as f64 + jitter)
                        .rem_euclid(buf_len as f64);
                grain.start(start_pos, self.grain_length);
            }
        }
        self.sample_counter += 1;

        // Sum output from all active grains
        let mut output = 0.0;
        for grain in &mut self.grains {
            output += grain.read_sample(&self.buffer, playback_rate);
        }

        // Normalize by overlap count to prevent level buildup
        output / (GRAIN_COUNT as f64 * 0.5)
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
        self.sample_counter = 0;
        for grain in &mut self.grains {
            grain.active = false;
            grain.samples_elapsed = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// PhaseVocoderEngine — STFT-based pitch shifting
// ---------------------------------------------------------------------------

struct PhaseVocoderEngine {
    /// FFT size in samples.
    fft_size: usize,
    /// Hop size = fft_size / 4 (4x overlap).
    hop_size: usize,

    /// Circular input buffer for accumulating input samples.
    input_buffer: Vec<f64>,
    /// Write position into input_buffer.
    input_write_pos: usize,
    /// How many new input samples accumulated since last analysis hop.
    input_samples_since_hop: usize,

    /// Circular output buffer for overlap-add synthesis.
    output_buffer: Vec<f64>,
    /// Read position into output_buffer.
    output_read_pos: usize,

    /// Analysis window (Hann).
    analysis_window: Vec<f64>,

    /// Previous analysis phase per bin.
    prev_analysis_phase: Vec<f64>,
    /// Accumulated synthesis phase per bin.
    synthesis_phase: Vec<f64>,

    /// Scratch buffer for FFT input.
    fft_scratch: Vec<Complex<f64>>,
    /// Scratch buffer for IFFT output.
    ifft_scratch: Vec<Complex<f64>>,
    /// General FFT scratch space.
    fft_work: Vec<Complex<f64>>,

    /// Expected phase advance per bin per hop (2*PI*bin*hop_size/fft_size).
    expected_phase_advance: Vec<f64>,

    /// Whether we have enough data for the first hop.
    primed: bool,

    /// FFT planner (lazily creates plans).
    fft_planner: FftPlanner<f64>,
}

impl PhaseVocoderEngine {
    fn new(fft_size: usize) -> Self {
        let hop_size = fft_size / 4;
        let analysis_window = hann_window(fft_size);

        let expected_phase_advance: Vec<f64> = (0..fft_size)
            .map(|k| 2.0 * PI * k as f64 * hop_size as f64 / fft_size as f64)
            .collect();

        Self {
            fft_size,
            hop_size,
            input_buffer: vec![0.0; fft_size * 2],
            input_write_pos: 0,
            input_samples_since_hop: 0,
            output_buffer: vec![0.0; fft_size * 2],
            output_read_pos: 0,
            analysis_window,
            prev_analysis_phase: vec![0.0; fft_size],
            synthesis_phase: vec![0.0; fft_size],
            fft_scratch: vec![Complex::new(0.0, 0.0); fft_size],
            ifft_scratch: vec![Complex::new(0.0, 0.0); fft_size],
            fft_work: Vec::new(),
            expected_phase_advance,
            primed: false,
            fft_planner: FftPlanner::new(),
        }
    }

    /// Reinitialize for a new FFT size.
    fn resize(&mut self, fft_size: usize) {
        if fft_size == self.fft_size {
            return;
        }
        *self = Self::new(fft_size);
    }

    /// Process one input sample, returning one output sample.
    fn process_sample(&mut self, input: f64, pitch_ratio: f64) -> f64 {
        let buf_len = self.input_buffer.len();

        // Write input into circular input buffer
        self.input_buffer[self.input_write_pos % buf_len] = input;
        self.input_write_pos += 1;
        self.input_samples_since_hop += 1;

        // When we have accumulated a hop's worth of new samples, run analysis+synthesis
        if self.input_samples_since_hop >= self.hop_size {
            self.input_samples_since_hop = 0;
            self.process_hop(pitch_ratio);
            self.primed = true;
        }

        // Read from output buffer
        if !self.primed {
            return 0.0;
        }

        let out_buf_len = self.output_buffer.len();
        let out = self.output_buffer[self.output_read_pos % out_buf_len];
        // Clear after reading so overlap-add works correctly
        self.output_buffer[self.output_read_pos % out_buf_len] = 0.0;
        self.output_read_pos += 1;

        out
    }

    /// Run one hop of analysis, pitch shifting, and synthesis.
    fn process_hop(&mut self, pitch_ratio: f64) {
        let n = self.fft_size;
        let in_buf_len = self.input_buffer.len();

        // 1. Extract and window the input frame
        let frame_start = self.input_write_pos.wrapping_sub(n);
        for i in 0..n {
            let idx = (frame_start + i) % in_buf_len;
            self.fft_scratch[i] = Complex::new(
                self.input_buffer[idx] * self.analysis_window[i],
                0.0,
            );
        }

        // 2. Forward FFT
        let fft = self.fft_planner.plan_fft_forward(n);
        fft.process_with_scratch(&mut self.fft_scratch, &mut self.fft_work);

        // 3. Analysis: compute magnitude and instantaneous frequency per bin
        // Then do bin resampling for pitch shift

        // Clear IFFT scratch
        for c in self.ifft_scratch.iter_mut() {
            *c = Complex::new(0.0, 0.0);
        }

        let half_n = n / 2 + 1;
        for out_bin in 0..half_n {
            // Which input bin does this output bin read from?
            let src_bin_f = out_bin as f64 / pitch_ratio;
            let src_bin_lo = src_bin_f.floor() as usize;
            let src_bin_hi = src_bin_lo + 1;
            let frac = src_bin_f - src_bin_lo as f64;

            if src_bin_lo >= half_n {
                break;
            }

            // Interpolate magnitude
            let mag_lo = self.fft_scratch[src_bin_lo].norm();
            let mag_hi = if src_bin_hi < half_n {
                self.fft_scratch[src_bin_hi].norm()
            } else {
                0.0
            };
            let mag = mag_lo * (1.0 - frac) + mag_hi * frac;

            // Compute instantaneous frequency from the primary source bin
            let phase = self.fft_scratch[src_bin_lo].arg();
            let phase_diff = phase - self.prev_analysis_phase[src_bin_lo];
            let expected = self.expected_phase_advance[src_bin_lo];

            // Deviation from expected advance, wrapped to [-PI, PI]
            let deviation = wrap_phase(phase_diff - expected);
            // True frequency of this bin in radians per hop
            let true_freq = expected + deviation;

            // Scale by pitch ratio for the output bin
            let shifted_freq = true_freq * pitch_ratio;

            // Accumulate synthesis phase
            self.synthesis_phase[out_bin] += shifted_freq;

            // Reconstruct complex value from magnitude and synthesis phase
            let synth_phase = self.synthesis_phase[out_bin];
            self.ifft_scratch[out_bin] = Complex::new(
                mag * synth_phase.cos(),
                mag * synth_phase.sin(),
            );

            // Mirror for negative frequencies (except DC and Nyquist)
            if out_bin > 0 && out_bin < n / 2 {
                self.ifft_scratch[n - out_bin] = self.ifft_scratch[out_bin].conj();
            }
        }

        // Save analysis phases for next hop
        for bin in 0..half_n {
            self.prev_analysis_phase[bin] = self.fft_scratch[bin].arg();
        }

        // 4. Inverse FFT
        let ifft = self.fft_planner.plan_fft_inverse(n);
        ifft.process_with_scratch(&mut self.ifft_scratch, &mut self.fft_work);

        // 5. Window and overlap-add to output buffer
        let scale = 1.0 / n as f64;
        let out_buf_len = self.output_buffer.len();
        let out_start = self.output_read_pos;

        for i in 0..n {
            let sample = self.ifft_scratch[i].re * scale * self.analysis_window[i];
            let idx = (out_start + i) % out_buf_len;
            self.output_buffer[idx] += sample;
        }
    }

    fn reset(&mut self) {
        self.input_buffer.fill(0.0);
        self.input_write_pos = 0;
        self.input_samples_since_hop = 0;
        self.output_buffer.fill(0.0);
        self.output_read_pos = 0;
        self.prev_analysis_phase.fill(0.0);
        self.synthesis_phase.fill(0.0);
        self.primed = false;
    }
}

/// Wrap a phase angle into [-PI, PI].
#[inline]
fn wrap_phase(phase: f64) -> f64 {
    let mut p = phase;
    while p > PI {
        p -= 2.0 * PI;
    }
    while p < -PI {
        p += 2.0 * PI;
    }
    p
}

// ---------------------------------------------------------------------------
// PitchShifter — dual-mode pitch shifter
// ---------------------------------------------------------------------------

/// Dual-mode pitch shifter supporting granular synthesis and phase vocoder.
///
/// ## Modes
///
/// - **Granular** (mode=0): 4-voice overlap-add with Hann-windowed grains.
///   Best for creative/large pitch shifts.
/// - **Phase Vocoder** (mode=1): STFT-based bin resampling with phase
///   accumulation. Best for transparent/small shifts.
pub struct PitchShifter {
    sample_rate: u32,

    // Parameters
    semitones: f64,
    cents: f64,
    mode: u32,           // 0 = Granular, 1 = Vocoder
    grain_size_ms: f64,
    fft_size_index: u32, // 0=1024, 1=2048, 2=4096
    dry_wet: f64,
    formant_preserve: bool,
    bypass: bool,

    // Engines (stereo: left + right)
    granular_l: GranularEngine,
    granular_r: GranularEngine,
    vocoder_l: PhaseVocoderEngine,
    vocoder_r: PhaseVocoderEngine,

    // Smoother
    mix_smoother: ParamSmoother,
}

/// Map FFT size index (0/1/2) to actual FFT size.
fn fft_size_from_index(index: u32) -> usize {
    match index {
        0 => 1024,
        1 => 2048,
        _ => 4096,
    }
}

impl PitchShifter {
    /// Create a new pitch shifter with sensible defaults.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        let default_grain_ms = 20.0;
        let default_fft_index = 1u32;
        let fft_size = fft_size_from_index(default_fft_index);

        let mut granular_l = GranularEngine::new();
        let mut granular_r = GranularEngine::new();
        granular_l.update_params(sr, default_grain_ms);
        granular_r.update_params(sr, default_grain_ms);

        Self {
            sample_rate,
            semitones: 0.0,
            cents: 0.0,
            mode: 0,
            grain_size_ms: default_grain_ms,
            fft_size_index: default_fft_index,
            dry_wet: 1.0,
            formant_preserve: false,
            bypass: false,

            granular_l,
            granular_r,
            vocoder_l: PhaseVocoderEngine::new(fft_size),
            vocoder_r: PhaseVocoderEngine::new(fft_size),

            mix_smoother: ParamSmoother::new(1.0, sr, SMOOTH_MS),
        }
    }

    /// Compute the pitch ratio from current semitones + cents.
    #[inline]
    fn pitch_ratio(&self) -> f64 {
        let total_semitones = self.semitones + self.cents / 100.0;
        2.0_f64.powf(total_semitones / 12.0)
    }

    /// Current FFT size in samples.
    fn current_fft_size(&self) -> usize {
        fft_size_from_index(self.fft_size_index)
    }
}

// ---------------------------------------------------------------------------
// AudioBackend implementation
// ---------------------------------------------------------------------------

impl AudioBackend for PitchShifter {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Pitch Shifter",
            backend_type: BackendType::PluginHost,
            extensions: &[],
        }
    }

    fn load(&mut self, _path: &str) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn unload(&mut self) {
        self.granular_l.reset();
        self.granular_r.reset();
        self.vocoder_l.reset();
        self.vocoder_r.reset();
        self.mix_smoother.reset(self.dry_wet);
    }

    // -- MIDI: no-op for a pitch shifter effect --
    fn note_on(&mut self, _channel: u8, _note: u8, _velocity: u8) {}
    fn note_off(&mut self, _channel: u8, _note: u8) {}
    fn cc(&mut self, _channel: u8, _cc: u8, _value: u8) {}
    fn pitch_bend(&mut self, _channel: u8, _value: i16) {}
    fn program_change(&mut self, _channel: u8, _program: u8) {}
    fn all_notes_off(&mut self) {}

    // -- Audio: generator render is a no-op (this is an effect) --
    fn render(&mut self, _left: &mut [f32], _right: &mut [f32]) {}

    fn process_effect(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        let len = in_l.len();

        // Bypass: bit-exact copy
        if self.bypass {
            out_l[..len].copy_from_slice(&in_l[..len]);
            out_r[..len].copy_from_slice(&in_r[..len]);
            return;
        }

        let pitch_ratio = self.pitch_ratio();

        match self.mode {
            0 => {
                // Granular mode
                for n in 0..len {
                    let mix = self.mix_smoother.next();
                    let dry = 1.0 - mix;

                    let wet_l = self.granular_l.process_sample(in_l[n] as f64, pitch_ratio);
                    let wet_r = self.granular_r.process_sample(in_r[n] as f64, pitch_ratio);

                    out_l[n] = (dry * in_l[n] as f64 + mix * wet_l) as f32;
                    out_r[n] = (dry * in_r[n] as f64 + mix * wet_r) as f32;
                }
            }
            _ => {
                // Phase vocoder mode
                for n in 0..len {
                    let mix = self.mix_smoother.next();
                    let dry = 1.0 - mix;

                    let wet_l = self.vocoder_l.process_sample(in_l[n] as f64, pitch_ratio);
                    let wet_r = self.vocoder_r.process_sample(in_r[n] as f64, pitch_ratio);

                    out_l[n] = (dry * in_l[n] as f64 + mix * wet_l) as f32;
                    out_r[n] = (dry * in_r[n] as f64 + mix * wet_r) as f32;
                }
            }
        }
    }

    fn set_volume(&mut self, _volume: f32) {
        // Pitch shifter does not have a volume control.
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn latency(&self) -> u32 {
        match self.mode {
            0 => {
                // Granular: half a grain
                let sr = self.sample_rate as f64;
                ((self.grain_size_ms * sr / 1000.0 / 2.0) as u32).max(1)
            }
            _ => {
                // Phase vocoder: half the FFT size
                (self.current_fft_size() / 2) as u32
            }
        }
    }

    // -- Parameters --
    // 0: semitones       (-24..24, stepped, step_count=48)
    // 1: cents           (-100..100)
    // 2: mode            (0/1, stepped)
    // 3: grain_size_ms   (5..50)
    // 4: fft_size        (0..2, stepped, step_count=2)
    // 5: dry_wet         (0..1)
    // 6: formant_preserve (0/1, stepped)
    // 7: bypass          (0/1, stepped)

    fn param_count(&self) -> u32 {
        8
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        match index {
            0 => Some(ParamInfo {
                id: 0,
                name: "Semitones".into(),
                group: "Pitch".into(),
                min: -24.0,
                max: 24.0,
                default: 0.0,
                step_count: 48,
                flags: ParamFlags::STEPPED,
            }),
            1 => Some(ParamInfo {
                id: 1,
                name: "Cents".into(),
                group: "Pitch".into(),
                min: -100.0,
                max: 100.0,
                default: 0.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            2 => Some(ParamInfo {
                id: 2,
                name: "Mode".into(),
                group: "Engine".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            3 => Some(ParamInfo {
                id: 3,
                name: "Grain Size".into(),
                group: "Granular".into(),
                min: 5.0,
                max: 50.0,
                default: 20.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            4 => Some(ParamInfo {
                id: 4,
                name: "FFT Size".into(),
                group: "Vocoder".into(),
                min: 0.0,
                max: 2.0,
                default: 1.0,
                step_count: 2,
                flags: ParamFlags::STEPPED,
            }),
            5 => Some(ParamInfo {
                id: 5,
                name: "Dry/Wet".into(),
                group: "Mix".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step_count: 0,
                flags: ParamFlags::empty(),
            }),
            6 => Some(ParamInfo {
                id: 6,
                name: "Formant Preserve".into(),
                group: "Vocoder".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            7 => Some(ParamInfo {
                id: 7,
                name: "Bypass".into(),
                group: "Global".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                step_count: 1,
                flags: ParamFlags::STEPPED,
            }),
            _ => None,
        }
    }

    fn get_param(&self, id: u32) -> Option<f64> {
        match id {
            0 => Some(self.semitones),
            1 => Some(self.cents),
            2 => Some(self.mode as f64),
            3 => Some(self.grain_size_ms),
            4 => Some(self.fft_size_index as f64),
            5 => Some(self.dry_wet),
            6 => Some(if self.formant_preserve { 1.0 } else { 0.0 }),
            7 => Some(if self.bypass { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            0 => {
                self.semitones = value.clamp(-24.0, 24.0).round();
            }
            1 => {
                self.cents = value.clamp(-100.0, 100.0);
            }
            2 => {
                self.mode = if value >= 0.5 { 1 } else { 0 };
            }
            3 => {
                self.grain_size_ms = value.clamp(5.0, 50.0);
                let sr = self.sample_rate as f64;
                self.granular_l.update_params(sr, self.grain_size_ms);
                self.granular_r.update_params(sr, self.grain_size_ms);
            }
            4 => {
                let idx = (value.round() as u32).clamp(0, 2);
                self.fft_size_index = idx;
                let fft_size = fft_size_from_index(idx);
                self.vocoder_l.resize(fft_size);
                self.vocoder_r.resize(fft_size);
            }
            5 => {
                self.dry_wet = value.clamp(0.0, 1.0);
                self.mix_smoother.set_target(self.dry_wet);
            }
            6 => {
                self.formant_preserve = value >= 0.5;
            }
            7 => {
                self.bypass = value >= 0.5;
            }
            _ => {}
        }
    }

    fn param_display(&self, id: u32, value: f64) -> Option<String> {
        match id {
            0 => Some(format!("{:+}", value as i32)),
            1 => Some(format!("{:+} ct", value as i32)),
            2 => Some(if value >= 0.5 { "Vocoder" } else { "Granular" }.into()),
            3 => Some(format!("{:.0} ms", value)),
            4 => {
                let size = fft_size_from_index(value.round() as u32);
                Some(format!("{}", size))
            }
            5 => Some(format!("{:.0}%", value * 100.0)),
            6 => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
            7 => Some(if value >= 0.5 { "On" } else { "Off" }.into()),
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
    use std::f64::consts::PI;

    /// Generate a mono sine wave at the given frequency and amplitude.
    fn sine_wave(freq: f64, amplitude: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (amplitude * (2.0 * PI * freq * t).sin()) as f32
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // test_bypass_is_bitexact
    // -----------------------------------------------------------------------

    #[test]
    fn test_bypass_is_bitexact() {
        let mut ps = PitchShifter::new(44100);
        ps.set_param(7, 1.0); // bypass on

        let input: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.256).collect();
        let silent = vec![0.0f32; 512];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        ps.process_effect(&input, &silent, &mut out_l, &mut out_r);

        for i in 0..512 {
            assert_eq!(
                out_l[i].to_bits(),
                input[i].to_bits(),
                "bypass left sample {} not bit-exact",
                i
            );
            assert_eq!(
                out_r[i].to_bits(),
                silent[i].to_bits(),
                "bypass right sample {} not bit-exact",
                i
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_param_round_trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_round_trip() {
        let mut ps = PitchShifter::new(44100);

        // semitones
        ps.set_param(0, 7.0);
        assert_eq!(ps.get_param(0), Some(7.0));

        ps.set_param(0, -12.0);
        assert_eq!(ps.get_param(0), Some(-12.0));

        // cents
        ps.set_param(1, 50.0);
        assert_eq!(ps.get_param(1), Some(50.0));

        // mode
        ps.set_param(2, 1.0);
        assert_eq!(ps.get_param(2), Some(1.0));

        ps.set_param(2, 0.0);
        assert_eq!(ps.get_param(2), Some(0.0));

        // grain_size_ms
        ps.set_param(3, 30.0);
        assert_eq!(ps.get_param(3), Some(30.0));

        // fft_size
        ps.set_param(4, 2.0);
        assert_eq!(ps.get_param(4), Some(2.0));

        // dry_wet
        ps.set_param(5, 0.75);
        assert_eq!(ps.get_param(5), Some(0.75));

        // formant_preserve
        ps.set_param(6, 1.0);
        assert_eq!(ps.get_param(6), Some(1.0));

        // bypass
        ps.set_param(7, 1.0);
        assert_eq!(ps.get_param(7), Some(1.0));

        // Clamping
        ps.set_param(0, -30.0);
        assert_eq!(ps.get_param(0), Some(-24.0));

        ps.set_param(0, 30.0);
        assert_eq!(ps.get_param(0), Some(24.0));

        ps.set_param(1, -200.0);
        assert_eq!(ps.get_param(1), Some(-100.0));

        ps.set_param(1, 200.0);
        assert_eq!(ps.get_param(1), Some(100.0));

        ps.set_param(3, 1.0);
        assert_eq!(ps.get_param(3), Some(5.0));

        ps.set_param(3, 100.0);
        assert_eq!(ps.get_param(3), Some(50.0));

        // Invalid param
        assert_eq!(ps.get_param(99), None);
        assert!(ps.param_info(8).is_none());

        // Param count
        assert_eq!(ps.param_count(), 8);
    }

    // -----------------------------------------------------------------------
    // test_semitones_zero_near_passthrough
    // -----------------------------------------------------------------------

    #[test]
    fn test_semitones_zero_near_passthrough() {
        let sr = 44100u32;
        let num_samples = sr as usize; // 1 second

        // Test granular mode (mode=0) with shift=0
        let mut ps = PitchShifter::new(sr);
        ps.set_param(0, 0.0); // semitones = 0
        ps.set_param(1, 0.0); // cents = 0
        ps.set_param(2, 0.0); // granular
        ps.set_param(5, 1.0); // 100% wet
        // Jump smoother
        ps.mix_smoother.reset(1.0);

        let input = sine_wave(440.0, 0.5, sr, num_samples);
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        ps.process_effect(&input, &input, &mut out_l, &mut out_r);

        // Skip first ~50ms for settling, then compare RMS
        let skip = (sr as f64 * 0.05) as usize;
        let input_rms: f64 = input[skip..]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / (num_samples - skip) as f64;
        let output_rms: f64 = out_l[skip..]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / (num_samples - skip) as f64;

        let input_rms_db = 10.0 * input_rms.log10();
        let output_rms_db = 10.0 * output_rms.log10();
        let error_db = (output_rms_db - input_rms_db).abs();

        assert!(
            error_db < 6.0,
            "shift=0 granular: output RMS should be close to input RMS, error = {:.2} dB \
             (input={:.2} dB, output={:.2} dB)",
            error_db,
            input_rms_db,
            output_rms_db,
        );
    }

    // -----------------------------------------------------------------------
    // test_pitch_up_12_doubles_frequency
    // -----------------------------------------------------------------------

    #[test]
    fn test_pitch_up_12_doubles_frequency() {
        let sr = 44100u32;
        let freq = 440.0;
        let num_samples = sr as usize * 2; // 2 seconds for better measurement

        let mut ps = PitchShifter::new(sr);
        ps.set_param(0, 12.0); // +12 semitones = 1 octave up
        ps.set_param(1, 0.0);  // cents = 0
        ps.set_param(2, 0.0);  // granular mode
        ps.set_param(5, 1.0);  // 100% wet
        // Jump smoother
        ps.mix_smoother.reset(1.0);

        let input = sine_wave(freq, 0.5, sr, num_samples);
        let mut out_l = vec![0.0f32; num_samples];
        let mut out_r = vec![0.0f32; num_samples];

        ps.process_effect(&input, &input, &mut out_l, &mut out_r);

        // Count zero crossings in the output (skip first 0.2s for settling)
        let skip = (sr as f64 * 0.2) as usize;
        let output_crossings = count_zero_crossings(&out_l[skip..]);
        let input_crossings = count_zero_crossings(&input[skip..]);

        // +12 semitones should roughly double the frequency,
        // so zero crossings should be roughly 2x.
        let ratio = output_crossings as f64 / input_crossings as f64;

        assert!(
            ratio > 1.5 && ratio < 2.5,
            "+12 semitones: zero-crossing ratio should be ~2.0, got {:.2} \
             (input crossings={}, output crossings={})",
            ratio,
            input_crossings,
            output_crossings,
        );
    }

    // -----------------------------------------------------------------------
    // test_param_info_complete
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_info_complete() {
        let ps = PitchShifter::new(44100);
        assert_eq!(ps.param_count(), 8);

        for i in 0..8 {
            let info = ps.param_info(i);
            assert!(
                info.is_some(),
                "param_info({}) should return Some",
                i
            );
            let info = info.unwrap();
            assert_eq!(info.id, i);
            assert!(!info.name.is_empty(), "param {} name should not be empty", i);
            assert!(
                !info.group.is_empty(),
                "param {} group should not be empty",
                i
            );
        }

        // No param beyond 7
        assert!(ps.param_info(8).is_none());
    }

    // -----------------------------------------------------------------------
    // Helper: count zero crossings
    // -----------------------------------------------------------------------

    fn count_zero_crossings(buf: &[f32]) -> usize {
        let mut count = 0;
        for i in 1..buf.len() {
            if (buf[i - 1] >= 0.0 && buf[i] < 0.0) || (buf[i - 1] < 0.0 && buf[i] >= 0.0) {
                count += 1;
            }
        }
        count
    }
}
