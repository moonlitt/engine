//! Shared oversampling processor using linear-phase half-band FIR filters.
//!
//! Supports 1x (bypass), 2x, 4x, 8x via cascaded 2x stages. Each stage uses
//! a Kaiser-windowed sinc half-band filter for clean anti-aliasing with ~96 dB
//! stopband attenuation.
//!
//! # Usage
//!
//! ```ignore
//! let mut os = Oversampler::new(2, 512);
//! os.process(input, output, |oversampled_buf| {
//!     // process at 2x sample rate
//! });
//! ```

use std::f64::consts::PI;
use wide::f64x4;

// ---------------------------------------------------------------------------
// Half-band FIR filter design (Kaiser-windowed sinc)
// ---------------------------------------------------------------------------

/// Zeroth-order modified Bessel function of the first kind (I0).
/// Used by the Kaiser window function.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half_x = x / 2.0;
    for k in 1..50 {
        term *= half_x / k as f64;
        let t2 = term * term;
        sum += t2;
        if t2 < sum * 1e-20 {
            break;
        }
    }
    sum
}

/// Design a half-band lowpass FIR filter using Kaiser-windowed sinc.
///
/// Returns the full set of filter taps (length = 4 * half_order + 1).
/// For a half-band filter:
///   - Center tap = 0.5 (before normalization)
///   - Even-indexed taps (except center) = exactly 0.0
///   - Odd-indexed taps are non-zero (symmetric)
///
/// The returned taps are normalized for unity DC gain.
fn design_halfband_taps(half_order: usize, beta: f64) -> Vec<f64> {
    let filter_len = 4 * half_order + 1;
    let center = 2 * half_order;
    let i0_beta = bessel_i0(beta);

    let mut taps = vec![0.0f64; filter_len];

    // Center tap
    taps[center] = 0.5;

    // Odd-indexed taps (symmetric around center)
    for k in 1..=half_order {
        let dist = 2 * k - 1; // odd distance: 1, 3, 5, ...
        let m = dist as f64;

        // sinc(m/2) = sin(pi * m/2) / (pi * m/2)
        let sinc = (PI * m / 2.0).sin() / (PI * m / 2.0);

        // Kaiser window
        let ratio = dist as f64 / center as f64;
        let arg = beta * (1.0 - ratio * ratio).max(0.0).sqrt();
        let window = bessel_i0(arg) / i0_beta;

        let val = sinc * window;
        taps[center - dist] = val;
        taps[center + dist] = val;
    }

    // Normalize for unity DC gain while preserving the half-band property.
    // Center tap is fixed at 0.5. Only scale the odd taps so that
    // 0.5 + 2 * sum(odd_taps) = 1.0, i.e., sum(odd_taps) = 0.25.
    let odd_sum: f64 = (1..=half_order).map(|k| taps[center + (2 * k - 1)]).sum();
    if odd_sum.abs() > 1e-15 {
        let scale = 0.25 / odd_sum;
        for k in 1..=half_order {
            let dist = 2 * k - 1;
            taps[center - dist] *= scale;
            taps[center + dist] *= scale;
        }
    }

    taps
}

// ---------------------------------------------------------------------------
// HalfBandStage — a single 2x up/down sampling stage
// ---------------------------------------------------------------------------

/// A single 2x up/down sampling stage using a half-band FIR filter.
///
/// Half-band filters have the property that every other coefficient (except
/// the center tap) is exactly zero, halving the multiply count.
struct HalfBandStage {
    /// Full FIR taps (most are zero; half-band property).
    taps: Vec<f64>,
    /// Delay line for the filter. Length = `filter_len * 2` (doubled for
    /// contiguous SIMD reads). The first `filter_len` elements are the
    /// canonical ring buffer; the second half mirrors them so that a read
    /// starting at any position can load `filter_len` contiguous elements
    /// without wrapping.
    delay: Vec<f64>,
    /// Write position into delay line (next position to write).
    write_pos: usize,
    /// Total filter length (number of taps).
    filter_len: usize,
}

impl HalfBandStage {
    /// Create a new half-band stage.
    ///
    /// `half_order` controls filter length: total taps = 4 * half_order + 1.
    /// Higher half_order = better stopband rejection but more latency.
    /// beta controls Kaiser window sidelobe attenuation (~10 for 96 dB).
    fn new(half_order: usize, beta: f64) -> Self {
        let taps = design_halfband_taps(half_order, beta);
        let filter_len = taps.len();

        Self {
            taps,
            // Doubled buffer: canonical region [0..filter_len) is mirrored
            // at [filter_len..filter_len*2).
            delay: vec![0.0; filter_len * 2],
            write_pos: 0,
            filter_len,
        }
    }

    /// Push a sample into the delay line.
    #[inline]
    fn push(&mut self, sample: f64) {
        // Write to both canonical and mirror regions so that any contiguous
        // read of `filter_len` elements starting in [0..filter_len) is valid.
        self.delay[self.write_pos] = sample;
        self.delay[self.write_pos + self.filter_len] = sample;
        self.write_pos += 1;
        if self.write_pos >= self.filter_len {
            self.write_pos = 0;
        }
    }

    /// Compute one filter output from current delay line contents using SIMD.
    ///
    /// The taps are symmetric (linear-phase half-band), so the inner product
    /// `sum(delay[wp-1-i] * taps[i])` equals `sum(delay[wp+j] * taps[j])`
    /// where j = 0..N-1 (ascending from the oldest sample at `write_pos`).
    ///
    /// With the doubled delay buffer, `delay[write_pos..write_pos+filter_len]`
    /// is always a valid contiguous slice, enabling f64x4 SIMD loads.
    #[inline]
    fn filter_output(&self) -> f64 {
        let base = self.write_pos; // oldest sample
        let n = self.filter_len;

        // Process 4 elements at a time with f64x4 SIMD
        let chunks = n / 4;
        let remainder = n % 4;

        let mut sum = f64x4::ZERO;
        for c in 0..chunks {
            let off = c * 4;
            let d = f64x4::new(self.delay[base + off..base + off + 4].try_into().unwrap());
            let t = f64x4::new(self.taps[off..off + 4].try_into().unwrap());
            sum += d * t;
        }

        let mut result = sum.reduce_add();

        // Handle remainder (e.g. 25 taps = 6*4 + 1)
        let rem_start = chunks * 4;
        for i in 0..remainder {
            result += self.delay[base + rem_start + i] * self.taps[rem_start + i];
        }

        result
    }

    /// Upsample: for each input sample, produce 2 output samples.
    ///
    /// Zero-stuffing: insert input sample, then zero. Apply filter. Scale by 2.
    fn upsample(&mut self, input: &[f32], output: &mut [f32]) {
        let in_len = input.len();
        debug_assert!(output.len() >= in_len * 2);

        for i in 0..in_len {
            // Push input sample
            self.push(input[i] as f64);
            let y0 = self.filter_output() * 2.0;

            // Push zero (zero-stuffing)
            self.push(0.0);
            let y1 = self.filter_output() * 2.0;

            output[i * 2] = y0 as f32;
            output[i * 2 + 1] = y1 as f32;
        }
    }

    /// Downsample: for each pair of input samples, produce 1 output sample.
    ///
    /// Filter at the high rate, then decimate by 2.
    fn downsample(&mut self, input: &[f32], output: &mut [f32]) {
        let out_len = input.len() / 2;
        debug_assert!(output.len() >= out_len);

        for i in 0..out_len {
            // Push two input samples, take output after the second
            self.push(input[i * 2] as f64);
            // Discard this output (decimation)

            self.push(input[i * 2 + 1] as f64);
            let y = self.filter_output();

            output[i] = y as f32;
        }
    }

    fn reset(&mut self) {
        self.delay.fill(0.0);
        self.write_pos = 0;
    }

    /// Scalar reference implementation of `filter_output` for testing.
    ///
    /// Uses the original modular-indexing algorithm. Kept as a reference to
    /// verify SIMD correctness.
    #[cfg(test)]
    fn filter_output_scalar(&self) -> f64 {
        let mut out = 0.0;
        let mut pos = if self.write_pos == 0 {
            self.filter_len - 1
        } else {
            self.write_pos - 1
        };

        for tap in &self.taps {
            out += self.delay[pos] * tap;
            if pos == 0 {
                pos = self.filter_len - 1;
            } else {
                pos -= 1;
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Oversampler — cascaded 2x stages
// ---------------------------------------------------------------------------

/// Shared oversampling processor supporting 1x (bypass), 2x, 4x, 8x.
///
/// Uses cascaded half-band FIR filter stages. Linear phase, suitable for
/// dynamics processors that need true-peak detection.
pub struct Oversampler {
    factor: usize,
    up_stages: Vec<HalfBandStage>,
    down_stages: Vec<HalfBandStage>,
    work_buffers: Vec<Vec<f32>>,
}

impl Oversampler {
    /// Create a new oversampler.
    ///
    /// `factor` must be 1, 2, 4, or 8 (rounds to nearest power of 2).
    /// `max_block_size` is the maximum number of input samples per call.
    pub fn new(factor: usize, max_block_size: usize) -> Self {
        let factor = factor.next_power_of_two().max(1);
        let num_stages = if factor <= 1 {
            0
        } else {
            (factor as f64).log2() as usize
        };

        // Half-order 6 with beta=10 gives 25-tap filter, ~96dB rejection
        let half_order = 6;
        let beta = 10.0;

        let up_stages: Vec<HalfBandStage> = (0..num_stages)
            .map(|_| HalfBandStage::new(half_order, beta))
            .collect();
        let down_stages: Vec<HalfBandStage> = (0..num_stages)
            .map(|_| HalfBandStage::new(half_order, beta))
            .collect();

        // Work buffers: one for each intermediate rate
        // Stage 0: input_len * 2
        // Stage 1: input_len * 4
        // etc.
        let work_buffers: Vec<Vec<f32>> = (0..num_stages)
            .map(|s| vec![0.0f32; max_block_size * (2 << s)])
            .collect();

        Self {
            factor,
            up_stages,
            down_stages,
            work_buffers,
        }
    }

    /// Process a block through the oversampler.
    ///
    /// 1. Upsamples `input` to `factor`x rate
    /// 2. Calls `callback` on the oversampled buffer
    /// 3. Downsamples back to original rate into `output`
    ///
    /// For factor=1, copies input to output buffer, calls callback on it.
    pub fn process<F>(&mut self, input: &[f32], output: &mut [f32], mut callback: F)
    where
        F: FnMut(&mut [f32]),
    {
        let in_len = input.len();
        debug_assert!(output.len() >= in_len);

        if self.factor <= 1 {
            // Bypass: copy input, process in-place, result in output
            output[..in_len].copy_from_slice(&input[..in_len]);
            callback(&mut output[..in_len]);
            return;
        }

        let num_stages = self.up_stages.len();

        // --- Upsample cascade ---
        // Stage 0: input -> work_buffers[0] (2x)
        self.up_stages[0].upsample(input, &mut self.work_buffers[0]);

        // Subsequent stages
        for s in 1..num_stages {
            let prev_len = in_len * (1 << s);
            let (left, right) = self.work_buffers.split_at_mut(s);
            let src = &left[s - 1][..prev_len];
            self.up_stages[s].upsample(src, &mut right[0]);
        }

        // --- Callback at highest rate ---
        let top_len = in_len * self.factor;
        let top_buf = &mut self.work_buffers[num_stages - 1][..top_len];
        callback(top_buf);

        // --- Downsample cascade (reverse order) ---
        for s in (1..num_stages).rev() {
            let out_len = in_len * (1 << s);
            let (left, right) = self.work_buffers.split_at_mut(s);
            self.down_stages[s].downsample(&right[0][..out_len * 2], &mut left[s - 1][..out_len]);
        }

        // Final stage: work_buffers[0] -> output
        let final_in_len = in_len * 2;
        self.down_stages[0].downsample(&self.work_buffers[0][..final_in_len], output);
    }

    /// Upsample `input` into `output`. `output` must have length >= input.len() * factor.
    /// For factor=1, copies input to output.
    pub fn upsample(&mut self, input: &[f32], output: &mut [f32]) {
        let in_len = input.len();
        if self.factor <= 1 {
            output[..in_len].copy_from_slice(&input[..in_len]);
            return;
        }

        let num_stages = self.up_stages.len();

        // Stage 0: input -> work_buffers[0] (2x)
        self.up_stages[0].upsample(input, &mut self.work_buffers[0]);

        // Subsequent stages use work_buffers
        for s in 1..num_stages {
            let prev_len = in_len * (1 << s);
            let (left, right) = self.work_buffers.split_at_mut(s);
            let src = &left[s - 1][..prev_len];
            self.up_stages[s].upsample(src, &mut right[0]);
        }

        // Copy top buffer to output
        let top_len = in_len * self.factor;
        let top = &self.work_buffers[num_stages - 1][..top_len];
        output[..top_len].copy_from_slice(top);
    }

    /// Downsample `input` (at oversampled rate) into `output` (at original rate).
    /// `input` length must be `output.len() * factor`.
    /// For factor=1, copies input to output.
    pub fn downsample(&mut self, input: &[f32], output: &mut [f32]) {
        let out_len = output.len();
        if self.factor <= 1 {
            output[..out_len].copy_from_slice(&input[..out_len]);
            return;
        }

        let num_stages = self.down_stages.len();

        // Copy input into the top work buffer
        let top_len = out_len * self.factor;
        self.work_buffers[num_stages - 1][..top_len].copy_from_slice(&input[..top_len]);

        // Downsample cascade (reverse order)
        for s in (1..num_stages).rev() {
            let ds_out_len = out_len * (1 << s);
            let (left, right) = self.work_buffers.split_at_mut(s);
            self.down_stages[s]
                .downsample(&right[0][..ds_out_len * 2], &mut left[s - 1][..ds_out_len]);
        }

        // Final stage: work_buffers[0] -> output
        let final_in_len = out_len * 2;
        self.down_stages[0].downsample(&self.work_buffers[0][..final_in_len], output);
    }

    /// Total latency in samples at the input rate.
    ///
    /// Each stage pair (up + down) introduces FIR group delay. For a stage
    /// at level `s`, the filter runs at `2^(s+1)` times the input rate.
    /// Group delay of an `N`-tap linear-phase FIR is `(N-1)/2` samples at
    /// the filter's operating rate.
    ///
    /// Per stage pair (up filter + down filter):
    ///   - Up filter delay: `(N-1)/2` samples at `2^(s+1)` rate
    ///     = `(N-1) / (2^(s+2))` at input rate
    ///   - Down filter delay: same
    ///   - Total per stage: `(N-1) / 2^(s+1)` at input rate
    pub fn latency(&self) -> usize {
        if self.factor <= 1 {
            return 0;
        }

        let mut total = 0usize;
        for s in 0..self.up_stages.len() {
            let n = self.up_stages[s].filter_len;
            let delay_at_filter_rate = (n - 1) / 2;
            // Two filters per stage, each at 2^(s+1) rate
            let divisor = 1usize << (s + 1);
            total += 2 * delay_at_filter_rate / divisor;
        }
        total
    }

    /// Reset all internal filter state.
    pub fn reset(&mut self) {
        for stage in &mut self.up_stages {
            stage.reset();
        }
        for stage in &mut self.down_stages {
            stage.reset();
        }
    }

    /// The oversampling factor.
    pub fn factor(&self) -> usize {
        self.factor
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_one_is_passthrough() {
        let mut os = Oversampler::new(1, 64);
        let input: Vec<f32> = (0..64).map(|i| (i as f32) * 0.1).collect();
        let mut output = vec![0.0f32; 64];
        os.process(&input, &mut output, |_buf| {
            // identity -- do nothing
        });
        assert_eq!(input, output, "factor=1 should be bit-exact passthrough");
    }

    #[test]
    fn upsample_preserves_dc() {
        // Feed constant 1.0 through 2x. After the filter settles, output
        // should converge to 1.0.
        let mut os = Oversampler::new(2, 64);
        let input = vec![1.0f32; 64];
        let mut output = vec![0.0f32; 64];

        // Run many blocks to let the filter settle
        for _ in 0..100 {
            os.process(&input, &mut output, |_buf| {
                // identity callback
            });
        }

        // Check the last few output samples
        for &s in &output[32..] {
            assert!(
                (s - 1.0).abs() < 0.01,
                "DC should be preserved through 2x oversampling, got {s}"
            );
        }
    }

    #[test]
    fn downsample_recovers_original() {
        // Feed 100 Hz sine through up -> identity -> down at 44100 Hz.
        // Verify amplitude and frequency are preserved by measuring the
        // peak amplitude and comparing to input peak. Use cross-correlation
        // to find true delay.
        let sample_rate = 44100.0f32;
        let freq = 100.0f32;
        let block_size = 256;
        let mut os = Oversampler::new(2, block_size);

        // Generate a long continuous signal and collect all output
        let num_blocks = 200;
        let total_samples = num_blocks * block_size;
        let mut all_output = Vec::with_capacity(total_samples);

        for b in 0..num_blocks {
            let offset = b * block_size;
            let input: Vec<f32> = (0..block_size)
                .map(|i| {
                    let t = (offset + i) as f32 / sample_rate;
                    (2.0 * std::f32::consts::PI * freq * t).sin()
                })
                .collect();
            let mut output = vec![0.0f32; block_size];
            os.process(&input, &mut output, |_buf| {});
            all_output.extend_from_slice(&output);
        }

        // Use cross-correlation to find the true delay (integer sample)
        let start = total_samples / 2;
        let end = total_samples;
        let search_range = 50usize; // search +-50 samples around reported latency
        let reported = os.latency();
        let mut best_corr = f64::NEG_INFINITY;
        let mut best_delay = reported;

        for d in reported.saturating_sub(search_range)..=(reported + search_range) {
            let mut corr = 0.0f64;
            for i in start..end {
                let t = (i as isize - d as isize) as f32 / sample_rate;
                let reference = (2.0 * std::f32::consts::PI * freq * t).sin();
                corr += all_output[i] as f64 * reference as f64;
            }
            if corr > best_corr {
                best_corr = corr;
                best_delay = d;
            }
        }

        // Now compute RMS error with the best-fit delay
        let mut sum_sq = 0.0f64;
        let mut sum_sig = 0.0f64;
        for i in start..end {
            let t = (i as isize - best_delay as isize) as f32 / sample_rate;
            let reference = (2.0 * std::f32::consts::PI * freq * t).sin();
            let diff = (all_output[i] - reference) as f64;
            sum_sq += diff * diff;
            sum_sig += (reference as f64) * (reference as f64);
        }
        let n = (end - start) as f64;
        let rms_error = (sum_sq / n).sqrt();
        let rms_signal = (sum_sig / n).sqrt();
        let relative_error = rms_error / rms_signal.max(1e-10);

        // Allow up to 1% relative error. The residual comes from sub-sample
        // delay (cross-correlation has integer resolution) and passband ripple.
        assert!(
            relative_error < 0.01,
            "RMS error {rms_error:.6} (relative {relative_error:.6}) too high for \
             100Hz sine through 2x (delay={best_delay}, reported={reported})"
        );
    }

    #[test]
    fn latency_correct() {
        // Feed an impulse through 2x and find where the peak appears.
        let block_size = 256;
        let mut os = Oversampler::new(2, block_size);
        let reported_latency = os.latency();

        // Create impulse: 1.0 followed by zeros
        let mut impulse = vec![0.0f32; block_size];
        impulse[0] = 1.0;
        let zeros = vec![0.0f32; block_size];

        // Collect several blocks of output
        let num_blocks = 10;
        let mut all_output = Vec::with_capacity(block_size * num_blocks);

        let mut output = vec![0.0f32; block_size];
        os.process(&impulse, &mut output, |_buf| {});
        all_output.extend_from_slice(&output);

        for _ in 1..num_blocks {
            os.process(&zeros, &mut output, |_buf| {});
            all_output.extend_from_slice(&output);
        }

        // Find the peak
        let (peak_idx, peak_val) = all_output
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
            .unwrap();

        assert!(
            peak_val.abs() > 0.01,
            "Impulse response peak too small: {peak_val}"
        );

        let diff = (peak_idx as i64 - reported_latency as i64).unsigned_abs() as usize;
        assert!(
            diff <= 1,
            "Peak at index {peak_idx}, reported latency {reported_latency} (diff {diff} > 1)"
        );
    }

    #[test]
    fn reset_clears_state() {
        let block_size = 64;
        let mut os = Oversampler::new(2, block_size);

        // Process some audio (loud sine)
        let input: Vec<f32> = (0..block_size).map(|i| (i as f32 * 0.3).sin()).collect();
        let mut output = vec![0.0f32; block_size];
        for _ in 0..10 {
            os.process(&input, &mut output, |_buf| {});
        }

        // Reset
        os.reset();

        // Process zeros
        let zeros = vec![0.0f32; block_size];
        os.process(&zeros, &mut output, |_buf| {});

        // Output should be all zeros (within floating-point tolerance)
        for (i, &s) in output.iter().enumerate() {
            assert!(
                s.abs() < 1e-6,
                "After reset, sample {i} should be ~0.0, got {s}"
            );
        }
    }

    #[test]
    fn factor_4x_works() {
        // Feed 100 Hz sine through 4x and verify signal fidelity.
        let sample_rate = 44100.0f32;
        let freq = 100.0f32;
        let block_size = 256;
        let mut os = Oversampler::new(4, block_size);

        let num_blocks = 300;
        let total_samples = num_blocks * block_size;
        let mut all_output = Vec::with_capacity(total_samples);

        for b in 0..num_blocks {
            let offset = b * block_size;
            let input: Vec<f32> = (0..block_size)
                .map(|i| {
                    let t = (offset + i) as f32 / sample_rate;
                    (2.0 * std::f32::consts::PI * freq * t).sin()
                })
                .collect();
            let mut output = vec![0.0f32; block_size];
            os.process(&input, &mut output, |_buf| {});
            all_output.extend_from_slice(&output);
        }

        // Cross-correlate to find true delay
        let start = total_samples / 2;
        let end = total_samples;
        let reported = os.latency();
        let mut best_corr = f64::NEG_INFINITY;
        let mut best_delay = reported;

        for d in reported.saturating_sub(50)..=(reported + 50) {
            let mut corr = 0.0f64;
            for i in start..end {
                let t = (i as isize - d as isize) as f32 / sample_rate;
                let reference = (2.0 * std::f32::consts::PI * freq * t).sin();
                corr += all_output[i] as f64 * reference as f64;
            }
            if corr > best_corr {
                best_corr = corr;
                best_delay = d;
            }
        }

        let mut sum_sq = 0.0f64;
        let mut sum_sig = 0.0f64;
        for i in start..end {
            let t = (i as isize - best_delay as isize) as f32 / sample_rate;
            let reference = (2.0 * std::f32::consts::PI * freq * t).sin();
            let diff = (all_output[i] - reference) as f64;
            sum_sq += diff * diff;
            sum_sig += (reference as f64) * (reference as f64);
        }
        let n = (end - start) as f64;
        let rms_error = (sum_sq / n).sqrt();
        let rms_signal = (sum_sig / n).sqrt();
        let relative_error = rms_error / rms_signal.max(1e-10);

        assert!(
            relative_error < 0.01,
            "4x oversampling: relative RMS error {relative_error:.6} too high for \
             100Hz sine (delay={best_delay}, reported={reported})"
        );
    }

    #[test]
    fn simd_fir_matches_scalar() {
        // Feed diverse samples through a HalfBandStage and verify the SIMD
        // filter_output matches the scalar reference at every step.
        let half_order = 6;
        let beta = 10.0;

        let mut stage_simd = HalfBandStage::new(half_order, beta);
        let mut stage_scalar = HalfBandStage::new(half_order, beta);

        // Push samples and compare filter output at each step
        let num_samples = 200;
        for i in 0..num_samples {
            let sample = (i as f64 * 0.37).sin() + (i as f64 * 1.13).cos() * 0.5;
            stage_simd.push(sample);
            stage_scalar.push(sample);

            let simd_out = stage_simd.filter_output();
            let scalar_out = stage_scalar.filter_output_scalar();

            let diff = (simd_out - scalar_out).abs();
            assert!(
                diff < 1e-12,
                "SIMD/scalar mismatch at sample {i}: simd={simd_out}, \
                 scalar={scalar_out}, diff={diff}"
            );
        }
    }
}
