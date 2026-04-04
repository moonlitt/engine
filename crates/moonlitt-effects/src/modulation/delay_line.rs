//! Per-sample sinc-interpolated fractional delay line.
//!
//! Core DSP building block for chorus, flanger, and delay effects.
//! Uses a Kaiser-windowed sinc kernel (8-point, beta=6.2, 256× oversampling)
//! for high-quality fractional-sample interpolation.
//!
//! Also provides a linear interpolation fallback for non-critical paths.

use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Bessel I₀ — modified Bessel function of the first kind, order 0
// ---------------------------------------------------------------------------

/// Series expansion of I₀(x).  Convergence is fast for typical Kaiser beta
/// values (< 15).  20 terms more than suffice; we early-exit when a term
/// contributes less than 1e-12.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let x_half = x / 2.0;
    for k in 1..=20 {
        term *= (x_half / k as f64).powi(2);
        sum += term;
        if term < 1e-12 {
            break;
        }
    }
    sum
}

/// Normalized sinc: sin(πx) / (πx), with sinc(0) = 1.
fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-10 {
        1.0
    } else {
        let px = PI * x;
        px.sin() / px
    }
}

/// Kaiser window evaluated at position `n` with the given `half_len` and
/// `beta`.  Returns 0 when |n| > half_len.
fn kaiser(n: f64, half_len: f64, beta: f64) -> f64 {
    if n.abs() > half_len {
        return 0.0;
    }
    let ratio = n / half_len;
    let arg = beta * (1.0 - ratio * ratio).max(0.0).sqrt();
    bessel_i0(arg) / bessel_i0(beta)
}

// ---------------------------------------------------------------------------
// SincTable — pre-computed interpolation kernel
// ---------------------------------------------------------------------------

/// Pre-computed windowed sinc table for fractional delay interpolation.
///
/// Layout: `table[frac_idx * num_points + j]` stores the kernel weight for
/// fractional offset `frac_idx / oversample` at tap `j`.
struct SincTable {
    table: Vec<f32>,
    oversample: usize,
}

impl SincTable {
    /// Build a new sinc table.
    ///
    /// - `num_points`: kernel width (8 = high quality for modulation effects)
    /// - `oversample`: sub-sample fractional resolution (256)
    fn new(num_points: usize, oversample: usize) -> Self {
        let half_len = (num_points / 2) as f64;
        let beta = 6.2; // matches moonlitt-resampler Sinc8 quality band

        let mut table = vec![0.0f32; oversample * num_points];

        for frac_idx in 0..oversample {
            let frac = frac_idx as f64 / oversample as f64;

            for j in 0..num_points {
                let x = j as f64 - (num_points / 2 - 1) as f64 - frac;
                let s = sinc(x);
                let w = kaiser(x, half_len, beta);
                table[frac_idx * num_points + j] = (s * w) as f32;
            }
        }

        Self {
            table,
            oversample,
        }
    }
}

// ---------------------------------------------------------------------------
// FractionalDelayLine
// ---------------------------------------------------------------------------

/// Per-sample fractional delay line with sinc interpolation.
///
/// Writes one sample at a time and reads at arbitrary (fractional) delay
/// positions.  The sinc kernel provides much lower interpolation error than
/// linear interpolation, which matters for modulation effects where the delay
/// time sweeps continuously.
pub struct FractionalDelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    max_delay_samples: usize,
    sinc_table: SincTable,
    num_points: usize,
}

impl FractionalDelayLine {
    /// Create a new delay line.
    ///
    /// - `max_delay_ms`: maximum delay in milliseconds
    /// - `sample_rate`: audio sample rate in Hz
    /// - `sinc_points`: kernel width (8 recommended)
    pub fn new(max_delay_ms: f64, sample_rate: u32, sinc_points: usize) -> Self {
        let max_delay_samples =
            (max_delay_ms * 0.001 * sample_rate as f64).ceil() as usize;
        // Buffer needs extra room for the sinc kernel
        let buffer_size = max_delay_samples + sinc_points + 1;
        let sinc_table = SincTable::new(sinc_points, 256);

        Self {
            buffer: vec![0.0; buffer_size],
            write_pos: 0,
            max_delay_samples,
            sinc_table,
            num_points: sinc_points,
        }
    }

    /// Write one sample into the delay line, advancing the write head.
    pub fn write(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos += 1;
        if self.write_pos >= self.buffer.len() {
            self.write_pos = 0;
        }
    }

    /// Read from the delay line at a fractional delay using sinc interpolation.
    ///
    /// `delay_samples` is clamped to `[0, max_delay_samples]`.
    pub fn read(&self, delay_samples: f64) -> f32 {
        let delay = delay_samples.max(0.0).min(self.max_delay_samples as f64);
        let delay_int = delay.floor() as usize;
        let frac = delay - delay_int as f64;

        // Quantize fractional part to table resolution
        let frac_idx = (frac * self.sinc_table.oversample as f64) as usize;
        let frac_idx = frac_idx.min(self.sinc_table.oversample - 1);

        let kernel =
            &self.sinc_table.table[frac_idx * self.num_points..][..self.num_points];
        let half = self.num_points / 2;
        let buf_len = self.buffer.len();

        let mut sum = 0.0f32;
        for (j, &k) in kernel.iter().enumerate() {
            // Most recent sample is at write_pos - 1 (wrapped).
            // A delay of N reads from write_pos - 1 - N (wrapped).
            // The sinc kernel is indexed so that increasing j corresponds to
            // increasing delay (further back in time).  j = half-1 is the
            // centre tap for frac=0; when frac > 0 the peak shifts to j > half-1,
            // reading from an older sample as intended.
            let pos =
                (self.write_pos + buf_len - 1 - delay_int + half - 1 - j) % buf_len;
            sum += self.buffer[pos] * k;
        }
        sum
    }

    /// Read from the delay line using cheap linear interpolation.
    ///
    /// Suitable for non-critical paths or when CPU is tight.
    pub fn read_linear(&self, delay_samples: f64) -> f32 {
        let delay = delay_samples.max(0.0).min(self.max_delay_samples as f64);
        let delay_int = delay.floor() as usize;
        let frac = (delay - delay_int as f64) as f32;
        let buf_len = self.buffer.len();

        let pos0 = (self.write_pos + buf_len - 1 - delay_int) % buf_len;
        let pos1 = (pos0 + buf_len - 1) % buf_len;

        self.buffer[pos0] * (1.0 - frac) + self.buffer[pos1] * frac
    }

    /// Zero the buffer and reset the write position.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }

    /// Maximum delay in samples (as configured at construction time).
    pub fn max_delay_samples(&self) -> usize {
        self.max_delay_samples
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_delay_is_exact() {
        let mut dl = FractionalDelayLine::new(100.0, 44100, 8);
        let delay: usize = 50;
        // Place the impulse so that it lands exactly `delay` samples behind
        // the most recent write.  Strategy: write `delay` zeros, then the
        // impulse, then `delay` more zeros.  The last zero is the most
        // recent sample (delay 0), so the impulse is at delay `delay`.
        for _ in 0..delay {
            dl.write(0.0);
        }
        dl.write(1.0);
        for _ in 0..delay {
            dl.write(0.0);
        }
        let val = dl.read(delay as f64);
        assert!(
            (val - 1.0).abs() < 0.01,
            "Integer delay should be near-exact, got {val}"
        );
    }

    #[test]
    fn zero_delay_returns_current() {
        let mut dl = FractionalDelayLine::new(10.0, 44100, 8);
        dl.write(0.5);
        let val = dl.read(0.0);
        assert!(
            (val - 0.5).abs() < 0.1,
            "Zero delay should return recent sample, got {val}"
        );
    }

    #[test]
    fn fractional_delay_interpolates() {
        let mut dl = FractionalDelayLine::new(100.0, 44100, 8);
        for i in 0..100 {
            dl.write(i as f32 / 100.0);
        }
        let val = dl.read(10.5);
        let v10 = dl.read(10.0);
        let v11 = dl.read(11.0);
        assert!(
            (val > v10.min(v11) - 0.01) && (val < v10.max(v11) + 0.01),
            "Fractional should interpolate between {v10} and {v11}, got {val}"
        );
    }

    #[test]
    fn sinc_quality_exceeds_linear() {
        let sr = 44100u32;
        let freq = 1000.0f64;
        let mut dl_sinc = FractionalDelayLine::new(50.0, sr, 8);
        let mut dl_lin = FractionalDelayLine::new(50.0, sr, 8);
        let delay_frac = 20.7;
        let num_samples = 4410;

        let mut sinc_error = 0.0f64;
        let mut linear_error = 0.0f64;

        for i in 0..num_samples {
            let sample =
                (2.0 * std::f64::consts::PI * freq * i as f64 / sr as f64).sin() as f32;
            dl_sinc.write(sample);
            dl_lin.write(sample);

            if i >= delay_frac as usize + 10 {
                let expected = (2.0 * std::f64::consts::PI * freq
                    * (i as f64 - delay_frac)
                    / sr as f64)
                    .sin() as f32;
                sinc_error += (dl_sinc.read(delay_frac) - expected).powi(2) as f64;
                linear_error +=
                    (dl_lin.read_linear(delay_frac) - expected).powi(2) as f64;
            }
        }
        assert!(
            sinc_error < linear_error,
            "Sinc error ({sinc_error:.8}) should be < linear error ({linear_error:.8})"
        );
    }

    #[test]
    fn clear_resets_buffer() {
        let mut dl = FractionalDelayLine::new(10.0, 44100, 8);
        dl.write(1.0);
        dl.clear();
        assert_eq!(dl.read(0.0), 0.0);
    }

    #[test]
    fn max_delay_correct() {
        let dl = FractionalDelayLine::new(10.0, 44100, 8);
        assert!(
            dl.max_delay_samples() >= 441,
            "10ms @ 44100 = 441 samples"
        );
    }
}
