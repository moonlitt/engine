//! IR partitioning and FFT pre-computation.
//!
//! Reference: Overlap-add method for fast convolution.
//! The IR is split into blocks of size B, each zero-padded to 2B,
//! and pre-transformed via FFT. During real-time processing,
//! each input block is similarly transformed and multiplied with
//! all IR partitions in the frequency domain.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Pre-computed FFT partitions of an impulse response.
pub struct IrPartitions {
    /// FFT of each IR partition (zero-padded to 2*block_size).
    pub partitions: Vec<Vec<Complex<f32>>>,
    /// Block size used for partitioning.
    #[allow(dead_code)]
    pub block_size: usize,
    /// FFT size = 2 * block_size.
    pub fft_size: usize,
}

impl IrPartitions {
    /// Split IR into blocks of `block_size`, zero-pad each to `2*block_size`,
    /// and pre-compute their FFTs.
    pub fn new(ir: &[f32], block_size: usize) -> Self {
        assert!(block_size > 0, "block_size must be positive");
        let fft_size = 2 * block_size;
        let num_partitions = if ir.is_empty() {
            1
        } else {
            ir.len().div_ceil(block_size)
        };

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);

        let mut partitions = Vec::with_capacity(num_partitions);

        for p in 0..num_partitions {
            let start = p * block_size;
            let mut buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); fft_size];

            // Copy IR samples into real part
            let end = (start + block_size).min(ir.len());
            for (i, &sample) in ir[start..end].iter().enumerate() {
                buf[i] = Complex::new(sample, 0.0);
            }
            // Remaining samples are already zero (zero-padding)

            fft.process(&mut buf);
            partitions.push(buf);
        }

        Self {
            partitions,
            block_size,
            fft_size,
        }
    }

    /// Number of partitions.
    pub fn num_partitions(&self) -> usize {
        self.partitions.len()
    }
}
