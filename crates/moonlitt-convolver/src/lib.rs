//! # moonlitt-convolver
//!
//! FFT partitioned convolution reverb using overlap-add.
//!
//! References:
//! - Linear convolution: y[n] = Sigma_k x[k] * h[n-k]
//! - Overlap-add: Gardner, "Efficient Convolution without Input-Output Delay" (1995)
//! - Parseval's theorem: energy in time domain = energy in frequency domain
//! - FFT convolution: circular convolution with zero-padding = linear convolution
//!
//! Zero tolerance: identity IR = bit-exact, bypass = bit-exact.

mod convolver;
mod partition;

pub use convolver::Convolver;
