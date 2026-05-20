//! Benchmarks for `svg3_dom::parse`.
//!
//! Three representative shapes:
//! - `tiny`: a one-element `<svg/>` — measures fixed parser overhead.
//! - `flat_100_cubes`: an `<svg>` with 100 `<cube/>` siblings — measures
//!   per-element work (event dispatch + attribute scan + arena push).
//! - `deep_50_groups`: 50 nested `<g>`s wrapping a `<cube/>` — measures
//!   the stack-based descent path.
//!
//! Inputs are constructed once at benchmark-group setup and passed by
//! reference into the iter loop so we measure parsing, not string
//! construction. Each call's `Document` is dropped before the next iter
//! to keep arena reuse out of the measurement.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use svg3_dom::parse;

fn flat_n_cubes(n: usize) -> String {
    let mut s = String::with_capacity(12 + n * 8);
    s.push_str("<svg>");
    for _ in 0..n {
        s.push_str("<cube/>");
    }
    s.push_str("</svg>");
    s
}

fn deep_n_groups(depth: usize) -> String {
    let mut s = String::with_capacity(12 + depth * 8);
    s.push_str("<svg>");
    for _ in 0..depth {
        s.push_str("<g>");
    }
    s.push_str("<cube/>");
    for _ in 0..depth {
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    s
}

fn bench_parse(c: &mut Criterion) {
    let tiny = "<svg/>".to_owned();
    let flat = flat_n_cubes(100);
    let deep = deep_n_groups(50);

    let mut group = c.benchmark_group("parse");
    group.bench_function("tiny", |b| b.iter(|| parse(black_box(&tiny)).unwrap()));
    group.bench_function("flat_100_cubes", |b| {
        b.iter(|| parse(black_box(&flat)).unwrap())
    });
    group.bench_function("deep_50_groups", |b| {
        b.iter(|| parse(black_box(&deep)).unwrap())
    });
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
