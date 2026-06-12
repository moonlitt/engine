use crate::{Quality, SincInterpolator};

#[test]
fn test_linear_exact_sample() {
    let interp = SincInterpolator::new(Quality::Linear);
    let samples = vec![0.0, 0.5, 1.0, 0.5, 0.0];
    assert_eq!(interp.interpolate(&samples, 2, 0.0), 1.0);
}

#[test]
fn test_linear_midpoint() {
    let interp = SincInterpolator::new(Quality::Linear);
    let samples = vec![0.0, 1.0];
    let val = interp.interpolate(&samples, 0, 0.5);
    assert!((val - 0.5).abs() < 0.001);
}

#[test]
fn test_sinc8_exact_sample() {
    let interp = SincInterpolator::new(Quality::Sinc8);
    // A constant signal should interpolate to the same constant
    let samples = vec![1.0f32; 64];
    let val = interp.interpolate(&samples, 32, 0.0);
    assert!((val - 1.0).abs() < 0.01, "got {val}");
}

#[test]
fn test_sinc72_exact_sample() {
    let interp = SincInterpolator::new(Quality::Sinc72);
    let samples = vec![1.0f32; 128];
    let val = interp.interpolate(&samples, 64, 0.0);
    assert!((val - 1.0).abs() < 0.01, "got {val}");
}

#[test]
fn test_sinc72_midpoint_sine() {
    // Interpolate a sine wave at midpoint — should match sin(midpoint)
    let interp = SincInterpolator::new(Quality::Sinc72);
    let n = 256;
    let freq = 1.0 / 32.0; // low frequency relative to sample rate
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32).sin())
        .collect();

    // Interpolate at position 64.5
    let idx = 64;
    let frac = 0.5;
    let val = interp.interpolate(&samples, idx, frac);
    let expected = (2.0 * std::f32::consts::PI * freq * (idx as f32 + frac)).sin();
    let error = (val - expected).abs();
    assert!(
        error < 0.001,
        "Sinc72 sine interpolation error: {error} (got {val}, expected {expected})"
    );
}

#[test]
fn test_quality_hierarchy() {
    // Higher quality should give more accurate interpolation of a sine wave
    let n = 256;
    let freq = 0.1; // fairly high frequency to stress test
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32).sin())
        .collect();

    let idx = 100;
    let frac = 0.37;
    let expected = (2.0 * std::f32::consts::PI * freq * (idx as f32 + frac)).sin();

    let qualities = [
        Quality::Linear,
        Quality::Sinc8,
        Quality::Sinc16,
        Quality::Sinc36,
        Quality::Sinc48,
        Quality::Sinc72,
    ];

    let mut prev_error = f32::MAX;
    for q in qualities {
        let interp = SincInterpolator::new(q);
        let val = interp.interpolate_safe(&samples, idx, frac);
        let error = (val - expected).abs();
        eprintln!("{:?}: error = {error:.8}", q);
        // Each quality level should be at least as good as the previous
        // (not strictly monotonic due to edge effects, but generally true)
        assert!(error <= prev_error + 0.01, "{q:?} worse than previous");
        prev_error = error;
    }
}

#[test]
fn test_interpolate_safe_boundary() {
    let interp = SincInterpolator::new(Quality::Sinc72);
    let samples = vec![1.0f32; 10];
    // Should not panic even at boundaries
    let _ = interp.interpolate_safe(&samples, 0, 0.0);
    let _ = interp.interpolate_safe(&samples, 9, 0.5);
}

#[test]
fn test_sinc_table_size() {
    let interp = SincInterpolator::new(Quality::Sinc72);
    assert_eq!(interp.num_points(), 72);
}
