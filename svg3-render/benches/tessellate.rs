//! Benchmarks for `svg3_render::build_scene` — `<rect>` tessellation.
//!
//! Three representative shapes:
//! - `sharp_100`: 100 sharp `<rect>`s — per-rect quad tessellation.
//! - `rounded_100`: 100 rounded `<rect>`s — heavier corner-arc tessellation.
//! - `single_rounded`: one rounded `<rect>` — fixed per-shape cost.
//!
//! Documents are built once at benchmark-group setup and passed by
//! reference into the iter loop so we measure tessellation, not document
//! construction. This bench is pure CPU work — no GPU device is created —
//! so it runs under CodSpeed's Valgrind-based simulation mode.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use svg3_dom::{Document, ElementKind};
use svg3_render::build_scene;

/// A document of `n` sibling `<rect>`s under the root, optionally rounded.
fn document_of_rects(n: usize, rounded: bool) -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    for i in 0..n {
        let id = doc.append_child(root, ElementKind::Rect);
        let attrs = &mut doc.node_mut(id).element.attributes;
        attrs.insert("x".to_owned(), (i * 2).to_string());
        attrs.insert("y".to_owned(), "0".to_owned());
        attrs.insert("width".to_owned(), "20".to_owned());
        attrs.insert("height".to_owned(), "20".to_owned());
        attrs.insert("fill".to_owned(), "blue".to_owned());
        if rounded {
            attrs.insert("rx".to_owned(), "5".to_owned());
            attrs.insert("ry".to_owned(), "5".to_owned());
        }
    }
    doc
}

fn bench_tessellate(c: &mut Criterion) {
    let sharp = document_of_rects(100, false);
    let rounded = document_of_rects(100, true);
    let single = document_of_rects(1, true);

    let mut group = c.benchmark_group("tessellate");
    group.bench_function("sharp_100", |b| b.iter(|| build_scene(black_box(&sharp))));
    group.bench_function("rounded_100", |b| {
        b.iter(|| build_scene(black_box(&rounded)))
    });
    group.bench_function("single_rounded", |b| {
        b.iter(|| build_scene(black_box(&single)))
    });
    group.finish();
}

criterion_group!(benches, bench_tessellate);
criterion_main!(benches);
