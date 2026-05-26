//! Benchmarks for `svg3::render::build_scene` — basic-shape tessellation.
//!
//! Nine representative shapes:
//! - `sharp_100`: 100 sharp `<rect>`s — per-rect quad tessellation.
//! - `rounded_100`: 100 rounded `<rect>`s — heavier corner-arc tessellation.
//! - `single_rounded`: one rounded `<rect>` — fixed per-shape cost.
//! - `circle_100`: 100 `<circle>`s — per-circle triangle-fan tessellation.
//! - `ellipse_100`: 100 `<ellipse>`s — per-ellipse triangle-fan tessellation.
//! - `polygon_100`: 100 concave-star `<polygon>`s — ear-clip triangulation.
//! - `polyline_100`: 100 filled `<polyline>`s — per-polyline point-list tessellation.
//! - `line_100`: 100 stroked `<line>`s — per-line quad tessellation.
//! - `path_100`: 100 filled and stroked `<path>`s — SVG path parsing plus Lyon tessellation.
//!
//! Documents are built once at benchmark-group setup and passed by
//! reference into the iter loop so we measure tessellation, not document
//! construction. This bench is pure CPU work — no GPU device is created —
//! so it runs under CodSpeed's Valgrind-based simulation mode.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use svg3::dom::{Document, ElementKind};
use svg3::render::{build_scene, Viewport};

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

/// A document of `n` sibling `<circle>`s under the root.
fn document_of_circles(n: usize) -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    for i in 0..n {
        let id = doc.append_child(root, ElementKind::Circle);
        let attrs = &mut doc.node_mut(id).element.attributes;
        attrs.insert("cx".to_owned(), (i * 2).to_string());
        attrs.insert("cy".to_owned(), "0".to_owned());
        attrs.insert("r".to_owned(), "10".to_owned());
        attrs.insert("fill".to_owned(), "blue".to_owned());
    }
    doc
}

/// A document of `n` sibling `<ellipse>`s under the root.
fn document_of_ellipses(n: usize) -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    for i in 0..n {
        let id = doc.append_child(root, ElementKind::Ellipse);
        let attrs = &mut doc.node_mut(id).element.attributes;
        attrs.insert("cx".to_owned(), (i * 2).to_string());
        attrs.insert("cy".to_owned(), "0".to_owned());
        attrs.insert("rx".to_owned(), "12".to_owned());
        attrs.insert("ry".to_owned(), "8".to_owned());
        attrs.insert("fill".to_owned(), "blue".to_owned());
    }
    doc
}

/// A ten-vertex star outline centred at `(cx, cy)` — a concave polygon, so
/// it exercises the ear-clipping path rather than a trivial fan.
fn star_points(cx: f32, cy: f32) -> String {
    let mut points = String::new();
    for k in 0..10 {
        let radius = if k % 2 == 0 { 16.0 } else { 7.0 };
        let angle = std::f32::consts::PI * k as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
        if k > 0 {
            points.push(' ');
        }
        points.push_str(&format!(
            "{},{}",
            cx + radius * angle.cos(),
            cy + radius * angle.sin(),
        ));
    }
    points
}

/// A document of `n` sibling concave-star `<polygon>`s under the root.
fn document_of_polygons(n: usize) -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    for i in 0..n {
        let id = doc.append_child(root, ElementKind::Polygon);
        let attrs = &mut doc.node_mut(id).element.attributes;
        attrs.insert("points".to_owned(), star_points((i * 2) as f32, 0.0));
        attrs.insert("fill".to_owned(), "blue".to_owned());
    }
    doc
}

/// A document of `n` sibling triangular `<polyline>`s under the root.
fn document_of_polylines(n: usize) -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    for i in 0..n {
        let x = i * 2;
        let id = doc.append_child(root, ElementKind::Polyline);
        let attrs = &mut doc.node_mut(id).element.attributes;
        attrs.insert(
            "points".to_owned(),
            format!("{x},0 {},20 {},0", x + 10, x + 20),
        );
        attrs.insert("fill".to_owned(), "blue".to_owned());
    }
    doc
}

/// A document of `n` sibling stroked `<line>`s under the root.
fn document_of_lines(n: usize) -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    for i in 0..n {
        let id = doc.append_child(root, ElementKind::Line);
        let attrs = &mut doc.node_mut(id).element.attributes;
        attrs.insert("x1".to_owned(), (i * 2).to_string());
        attrs.insert("y1".to_owned(), "0".to_owned());
        attrs.insert("x2".to_owned(), (i * 2 + 20).to_string());
        attrs.insert("y2".to_owned(), "20".to_owned());
        attrs.insert("stroke".to_owned(), "blue".to_owned());
        attrs.insert("stroke-width".to_owned(), "2".to_owned());
    }
    doc
}

/// A document of `n` sibling filled and stroked cubic `<path>`s under the root.
fn document_of_paths(n: usize) -> Document {
    let mut doc = Document::new();
    let root = doc.root();
    for i in 0..n {
        let x = (i * 2) as f32;
        let id = doc.append_child(root, ElementKind::Path);
        let attrs = &mut doc.node_mut(id).element.attributes;
        attrs.insert(
            "d".to_owned(),
            format!(
                "M {x} 0 C {} 6 {} 14 {} 20 L {} 20 L {} 0 Z",
                x + 5.0,
                x + 15.0,
                x + 20.0,
                x + 26.0,
                x + 6.0,
            ),
        );
        attrs.insert("fill".to_owned(), "blue".to_owned());
        attrs.insert("stroke".to_owned(), "red".to_owned());
        attrs.insert("stroke-width".to_owned(), "1.5".to_owned());
    }
    doc
}

fn bench_tessellate(c: &mut Criterion) {
    let sharp = document_of_rects(100, false);
    let rounded = document_of_rects(100, true);
    let single = document_of_rects(1, true);
    let circles = document_of_circles(100);
    let ellipses = document_of_ellipses(100);
    let polygons = document_of_polygons(100);
    let polylines = document_of_polylines(100);
    let lines = document_of_lines(100);
    let paths = document_of_paths(100);
    // The fixtures use absolute coordinates, so the viewport only needs to be
    // a valid percentage basis.
    let viewport = Viewport {
        width: 256.0,
        height: 256.0,
    };

    let mut group = c.benchmark_group("tessellate");
    group.bench_function("sharp_100", |b| {
        b.iter(|| build_scene(black_box(&sharp), viewport))
    });
    group.bench_function("rounded_100", |b| {
        b.iter(|| build_scene(black_box(&rounded), viewport))
    });
    group.bench_function("single_rounded", |b| {
        b.iter(|| build_scene(black_box(&single), viewport))
    });
    group.bench_function("circle_100", |b| {
        b.iter(|| build_scene(black_box(&circles), viewport))
    });
    group.bench_function("ellipse_100", |b| {
        b.iter(|| build_scene(black_box(&ellipses), viewport))
    });
    group.bench_function("polygon_100", |b| {
        b.iter(|| build_scene(black_box(&polygons), viewport))
    });
    group.bench_function("polyline_100", |b| {
        b.iter(|| build_scene(black_box(&polylines), viewport))
    });
    group.bench_function("line_100", |b| {
        b.iter(|| build_scene(black_box(&lines), viewport))
    });
    group.bench_function("path_100", |b| {
        b.iter(|| build_scene(black_box(&paths), viewport))
    });
    group.finish();
}

criterion_group!(benches, bench_tessellate);
criterion_main!(benches);
