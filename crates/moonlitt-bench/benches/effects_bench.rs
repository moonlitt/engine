use criterion::{black_box, criterion_group, criterion_main, Criterion};
use moonlitt_core::AudioBackend;

fn bench_compressor_process_512(c: &mut Criterion) {
    let mut comp = moonlitt_effects::Compressor::new(44100);
    let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.02).sin() * 0.5).collect();
    let mut out_l = vec![0.0f32; 512];
    let mut out_r = vec![0.0f32; 512];

    c.bench_function("compressor_process_512", |b| {
        b.iter(|| {
            comp.process_effect(black_box(&input), black_box(&input), &mut out_l, &mut out_r);
            black_box(&out_l);
        })
    });
}

fn bench_db_to_linear_powf_1000(c: &mut Criterion) {
    let values: Vec<f64> = (-60..24).map(|i| i as f64).collect();

    c.bench_function("db_to_linear_powf_1000", |b| {
        b.iter(|| {
            for &v in &values {
                black_box(10.0_f64.powf(black_box(v) / 20.0));
            }
        })
    });
}

fn bench_db_to_linear_lut_1000(c: &mut Criterion) {
    let lut = moonlitt_effects::common::db_lut::DbLut::new();
    let values: Vec<f64> = (-60..24).map(|i| i as f64).collect();

    c.bench_function("db_to_linear_lut_1000", |b| {
        b.iter(|| {
            for &v in &values {
                black_box(lut.db_to_linear(black_box(v)));
            }
        })
    });
}

fn bench_oversampler_2x_512(c: &mut Criterion) {
    let mut os = moonlitt_effects::common::Oversampler::new(2, 512);
    let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.02).sin()).collect();
    let mut upsampled = vec![0.0f32; 1024];
    let mut output = vec![0.0f32; 512];

    c.bench_function("oversampler_2x_512", |b| {
        b.iter(|| {
            os.upsample(black_box(&input), &mut upsampled);
            os.downsample(black_box(&upsampled), &mut output);
            black_box(&output);
        })
    });
}

fn bench_sinc8_read_1000(c: &mut Criterion) {
    use moonlitt_effects::modulation::delay_line::FractionalDelayLine;

    let mut dl = FractionalDelayLine::new(100.0, 44100, 8);
    // Fill with some data
    for i in 0..4410 {
        dl.write((i as f32 * 0.01).sin());
    }

    c.bench_function("sinc8_read_1000", |b| {
        b.iter(|| {
            for i in 0..1000 {
                let delay = 10.0 + (i as f64 * 0.03).sin() * 5.0;
                black_box(dl.read(black_box(delay)));
            }
        })
    });
}

criterion_group!(
    benches,
    bench_compressor_process_512,
    bench_db_to_linear_powf_1000,
    bench_db_to_linear_lut_1000,
    bench_oversampler_2x_512,
    bench_sinc8_read_1000,
);
criterion_main!(benches);
