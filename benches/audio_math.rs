use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::f32::consts::{LN_10, LOG10_E};

fn bench_logs(c: &mut Criterion) {
    let mut group = c.benchmark_group("Logarithm");
    for ratio in [0.125f32, 0.25, 0.5, 0.707, 1.0].iter() {
        group.bench_with_input(BenchmarkId::new("log10", ratio), ratio, |b, i| {
            b.iter(|| black_box(*i).log10())
        });
        group.bench_with_input(BenchmarkId::new("log_base_10", ratio), ratio, |b, i| {
            b.iter(|| black_box(*i).log(10.0))
        });
        group.bench_with_input(BenchmarkId::new("ln_convert", ratio), ratio, |b, i| {
            b.iter(|| black_box(*i).ln() * LOG10_E)
        });
    }
    group.finish();
}

fn bench_pow(c: &mut Criterion) {
    let mut group = c.benchmark_group("Power");
    for db in [-18.0f32, -12.0, -6.0, -3.0, 0.0].iter() {
        let x = db * 0.05;
        group.bench_with_input(BenchmarkId::new("powf", db), &x, |b, i| {
            b.iter(|| 10.0f32.powf(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("exp_convert", db), &x, |b, i| {
            b.iter(|| f32::exp(black_box(*i) * LN_10))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_logs, bench_pow);
criterion_main!(benches);
