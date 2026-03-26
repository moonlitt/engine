//! Window functions for sinc interpolation.
//!
//! A window tapers the sinc kernel to zero at the edges,
//! reducing Gibbs ringing artifacts.

use std::f64::consts::PI;

/// Kaiser window — the standard choice for high-quality audio resampling.
/// Beta controls the trade-off between main lobe width and side lobe level.
///
/// For audio: beta=6.0-10.0 gives excellent results.
/// sfizz uses beta≈7.0 for its high-quality mode.
pub fn kaiser(n: f64, half_len: f64, beta: f64) -> f64 {
    if n.abs() > half_len {
        return 0.0;
    }
    let ratio = n / half_len;
    let arg = beta * (1.0 - ratio * ratio).max(0.0).sqrt();
    bessel_i0(arg) / bessel_i0(beta)
}

/// Modified Bessel function of the first kind, order 0.
/// Used by the Kaiser window. Computed via series expansion.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let x_half = x * 0.5;

    for k in 1..50 {
        term *= (x_half / k as f64) * (x_half / k as f64);
        sum += term;
        if term < sum * 1e-15 {
            break;
        }
    }
    sum
}

/// Blackman-Harris 4-term window — alternative to Kaiser with very low side lobes.
pub fn blackman_harris(n: f64, half_len: f64) -> f64 {
    if n.abs() > half_len {
        return 0.0;
    }
    let x = PI * (n / half_len + 1.0); // 0 to 2π
    0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
}

/// Normalized sinc function: sin(πx) / (πx), with sinc(0) = 1.
pub fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-10 {
        1.0
    } else {
        let px = PI * x;
        px.sin() / px
    }
}
