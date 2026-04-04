/// Flush tiny f32 values to zero to prevent denormal CPU stalls
/// in feedback paths and recursive filters.
#[inline(always)]
pub fn flush_denormal(x: f32) -> f32 {
    if x.abs() < 1e-15 {
        0.0
    } else {
        x
    }
}

/// Flush tiny f64 values to zero to prevent denormal CPU stalls.
#[inline(always)]
pub fn flush_denormal_f64(x: f64) -> f64 {
    if x.abs() < 1e-30 {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_values_pass_through() {
        assert_eq!(flush_denormal(1.0), 1.0);
        assert_eq!(flush_denormal(-0.5), -0.5);
        assert_eq!(flush_denormal(1e-10_f32), 1e-10_f32);
    }

    #[test]
    fn tiny_values_become_zero() {
        assert_eq!(flush_denormal(1e-20_f32), 0.0);
        assert_eq!(flush_denormal(-1e-20_f32), 0.0);
        assert_eq!(flush_denormal(0.0), 0.0);
    }

    #[test]
    fn f64_normal_values_pass_through() {
        assert_eq!(flush_denormal_f64(1.0), 1.0);
        assert_eq!(flush_denormal_f64(1e-20), 1e-20);
    }

    #[test]
    fn f64_tiny_values_become_zero() {
        assert_eq!(flush_denormal_f64(1e-35), 0.0);
    }
}
