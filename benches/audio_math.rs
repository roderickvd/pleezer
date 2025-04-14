use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::f32::{
    self,
    consts::{LN_10, LOG2_10, LOG10_2, LOG10_E},
};

#[inline]
fn log10(x: f32) -> f32 {
    x.log10() * 20.0
}

#[inline]
fn log_base_10(x: f32) -> f32 {
    x.log(10.0) * 20.0
}

#[inline]
fn ln_convert(x: f32) -> f32 {
    x.ln() * LOG10_E * 20.0
}

#[inline]
fn log2_convert(x: f32) -> f32 {
    x.log2() * LOG10_2 * 20.0
}

#[inline]
fn fast_log2(x: f32) -> f32 {
    fast_math::log2(x) * LOG10_2 * 20.0
}

#[inline]
fn raw_log2(x: f32) -> f32 {
    fast_math::log2_raw(x) * LOG10_2 * 20.0
}

#[inline]
fn approx_fast_log2(x: f32) -> f32 {
    fastapprox::fast::log2(x) * LOG10_2 * 20.0
}

#[inline]
fn approx_faster_log2(x: f32) -> f32 {
    fastapprox::faster::log2(x) * LOG10_2 * 20.0
}

#[inline]
fn approx_fast_ln_convert(x: f32) -> f32 {
    fastapprox::fast::ln(x) * LOG10_E * 20.0
}

#[inline]
fn approx_faster_ln_convert(x: f32) -> f32 {
    fastapprox::faster::ln(x) * LOG10_E * 20.0
}

fn bench_logs(c: &mut Criterion) {
    let mut group = c.benchmark_group("Logarithm");
    for ratio in [f32::MIN_POSITIVE, 0.5, 1.0].iter() {
        group.bench_with_input(BenchmarkId::new("log10", ratio), ratio, |b, i| {
            b.iter(|| log10(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("log_base_10", ratio), ratio, |b, i| {
            b.iter(|| log_base_10(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("ln_convert", ratio), ratio, |b, i| {
            b.iter(|| ln_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("log2_convert", ratio), ratio, |b, i| {
            b.iter(|| log2_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("fast_log2", ratio), ratio, |b, i| {
            b.iter(|| fast_log2(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("raw_log2", ratio), ratio, |b, i| {
            b.iter(|| raw_log2(black_box(*i)))
        });
        group.bench_with_input(
            BenchmarkId::new("approx_fast_ln_convert", ratio),
            ratio,
            |b, i| b.iter(|| approx_fast_ln_convert(black_box(*i))),
        );
        group.bench_with_input(
            BenchmarkId::new("approx_faster_ln_convert", ratio),
            ratio,
            |b, i| b.iter(|| approx_faster_ln_convert(black_box(*i))),
        );
        group.bench_with_input(
            BenchmarkId::new("approx_fast_log2", ratio),
            ratio,
            |b, i| b.iter(|| approx_fast_log2(black_box(*i))),
        );
        group.bench_with_input(
            BenchmarkId::new("approx_faster_log2", ratio),
            ratio,
            |b, i| b.iter(|| approx_faster_log2(black_box(*i))),
        );
    }
    group.finish();
}

#[inline]
fn powf(x: f32) -> f32 {
    10.0f32.powf(x) * 0.05
}

#[inline]
fn approx_fast_pow(x: f32) -> f32 {
    fastapprox::fast::pow(10.0, x) * 0.05
}

#[inline]
fn approx_faster_pow(x: f32) -> f32 {
    fastapprox::faster::pow(10.0, x) * 0.05
}

#[inline]
fn approx_fast_pow2(x: f32) -> f32 {
    fastapprox::fast::pow2(x * LOG2_10) * 0.05
}

#[inline]
fn approx_faster_pow2(x: f32) -> f32 {
    fastapprox::faster::pow2(x * LOG2_10) * 0.05
}

#[inline]
fn exp_convert(x: f32) -> f32 {
    f32::exp(x * LN_10) * 0.05
}

#[inline]
fn exp2_convert(x: f32) -> f32 {
    f32::exp2(x * LOG2_10) * 0.05
}

#[inline]
fn fast_exp_convert(x: f32) -> f32 {
    fast_math::exp(x * LN_10) * 0.05
}

#[inline]
fn raw_exp_convert(x: f32) -> f32 {
    fast_math::exp_raw(x * LN_10) * 0.05
}

#[inline]
fn fast_exp2_convert(x: f32) -> f32 {
    fast_math::exp2(x * LOG2_10) * 0.05
}

#[inline]
fn raw_exp2_convert(x: f32) -> f32 {
    fast_math::exp2_raw(x * LOG2_10) * 0.05
}

#[inline]
fn approx_fast_exp(x: f32) -> f32 {
    fastapprox::fast::exp(x * LN_10) * 0.05
}

#[inline]
fn approx_faster_exp(x: f32) -> f32 {
    fastapprox::faster::exp(x * LN_10) * 0.05
}

fn bench_pow(c: &mut Criterion) {
    let mut group = c.benchmark_group("Power");
    for db in [f32::MIN, -6.0, 0.0].iter() {
        group.bench_with_input(BenchmarkId::new("powf", db), db, |b, i| {
            b.iter(|| powf(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("exp_convert", db), db, |b, i| {
            b.iter(|| exp_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("exp2_convert", db), db, |b, i| {
            b.iter(|| exp2_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("fast_exp_convert", db), db, |b, i| {
            b.iter(|| fast_exp_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("raw_exp_convert", db), db, |b, i| {
            b.iter(|| raw_exp_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("fast_exp2_convert", db), db, |b, i| {
            b.iter(|| fast_exp2_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("raw_exp2_convert", db), db, |b, i| {
            b.iter(|| raw_exp2_convert(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("approx_fast_exp", db), db, |b, i| {
            b.iter(|| approx_fast_exp(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("approx_faster_exp", db), db, |b, i| {
            b.iter(|| approx_faster_exp(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("approx_fast_pow", db), db, |b, i| {
            b.iter(|| approx_fast_pow(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("approx_faster_pow", db), db, |b, i| {
            b.iter(|| approx_faster_pow(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("approx_fast_pow2", db), db, |b, i| {
            b.iter(|| approx_fast_pow2(black_box(*i)))
        });
        group.bench_with_input(BenchmarkId::new("approx_faster_pow2", db), db, |b, i| {
            b.iter(|| approx_faster_pow2(black_box(*i)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_logs, bench_pow);
criterion_main!(benches);
