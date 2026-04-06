//! SIMD-friendly buffer utilities for the audio thread.
//!
//! These are simple loops that LLVM auto-vectorizes well with
//! `-C target-cpu=native`. The `#[inline]` hints ensure they get inlined
//! into the caller's hot loop, enabling the auto-vectorizer to see the
//! full picture.
//!
//! Manual SIMD (via `wide`) should only be added if benchmarks show
//! auto-vectorization is insufficient.

/// Multiply every sample in `buf` by `gain`.
#[inline]
pub fn apply_gain(buf: &mut [f32], gain: f32) {
    for s in buf.iter_mut() {
        *s *= gain;
    }
}

/// Add `src` into `dst` element-wise.
///
/// Only processes `min(dst.len(), src.len())` elements.
#[inline]
pub fn accumulate(dst: &mut [f32], src: &[f32]) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d += *s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simd_apply_gain_matches_scalar() {
        let original: Vec<f32> = (0..513).map(|i| (i as f32 * 0.01).sin()).collect();
        let gain = 0.75f32;

        // Scalar reference
        let expected: Vec<f32> = original.iter().map(|&s| s * gain).collect();

        // SIMD path
        let mut buf = original.clone();
        apply_gain(&mut buf, gain);

        for (i, (&e, &b)) in expected.iter().zip(buf.iter()).enumerate() {
            assert!(
                (e - b).abs() < 1e-7,
                "Mismatch at index {i}: expected {e}, got {b}"
            );
        }
    }

    #[test]
    fn simd_accumulate_matches_scalar() {
        let src: Vec<f32> = (0..513).map(|i| (i as f32 * 0.007).cos()).collect();
        let dst_init: Vec<f32> = (0..513).map(|i| (i as f32 * 0.013).sin()).collect();

        // Scalar reference
        let expected: Vec<f32> = dst_init
            .iter()
            .zip(src.iter())
            .map(|(&d, &s)| d + s)
            .collect();

        // SIMD path
        let mut dst = dst_init.clone();
        accumulate(&mut dst, &src);

        for (i, (&e, &d)) in expected.iter().zip(dst.iter()).enumerate() {
            assert!(
                (e - d).abs() < 1e-7,
                "Mismatch at index {i}: expected {e}, got {d}"
            );
        }
    }

    #[test]
    fn simd_apply_gain_empty() {
        let mut buf: Vec<f32> = vec![];
        apply_gain(&mut buf, 2.0);
        assert!(buf.is_empty());
    }

    #[test]
    fn simd_accumulate_mismatched_len() {
        let src = vec![1.0f32; 10];
        let mut dst = vec![0.0f32; 5];
        accumulate(&mut dst, &src);
        // Only first 5 should be modified
        assert_eq!(dst, vec![1.0f32; 5]);
    }
}
