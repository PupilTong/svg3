//! Full-renderer benchmarks for `Renderer::render_to_image`.
//!
//! These benches exercise `Renderer::render_to_image`: per-call scene
//! tessellation, GPU buffer creation, render pass, and readback. The wgpu
//! device, queue and pipeline are built once when the `Renderer` is
//! constructed and reused across iterations. They are intended for
//! GPU-capable macOS runners and self-skip when no adapter is available, so
//! CI treats them as a timing smoke check rather than tracked CodSpeed data.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use svg3::dom::{parse, Document};
use svg3::render::{Camera, Image, RenderConfig, RenderError, Renderer};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn many_shapes_document() -> Document {
    let mut svg = String::from(
        r##"<svg extension="pupiltong" width="320" height="240"><rect width="100%" height="100%" fill="#0f172a"/>"##,
    );
    for i in 0..96 {
        let x = 8 + (i * 31) % 280;
        let y = 8 + (i * 47) % 200;
        let w = 18 + (i * 7) % 32;
        let h = 14 + (i * 11) % 28;
        svg.push_str(&format!(
            r##"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" ry="6" fill="#2563eb"/>"##
        ));

        let cx = 12 + (i * 29) % 296;
        let cy = 12 + (i * 43) % 216;
        let r = 5 + (i * 5) % 18;
        svg.push_str(&format!(
            r##"<circle cx="{cx}" cy="{cy}" r="{r}" fill="#f97316"/>"##
        ));

        let x1 = 4 + (i * 17) % 300;
        let y1 = 4 + (i * 23) % 220;
        let x2 = 4 + (i * 37) % 300;
        let y2 = 4 + (i * 41) % 220;
        let stroke_width = 1 + (i % 4);
        svg.push_str(&format!(
            r##"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#e2e8f0" stroke-width="{stroke_width}"/>"##
        ));

        let px = 8 + (i * 19) % 280;
        let py = 10 + (i * 13) % 200;
        svg.push_str(&format!(
            r##"<path d="M {px} {py} c 8 -10 28 -10 36 0 l 6 20 l -48 0 z" fill="#22c55e" stroke="#111827" stroke-width="1"/>"##
        ));
    }
    svg.push_str("</svg>");
    parse(&svg).expect("benchmark fixture should parse")
}

fn config(camera: Option<Camera>) -> RenderConfig {
    RenderConfig {
        width: WIDTH,
        height: HEIGHT,
        camera,
        ..RenderConfig::default()
    }
}

fn render_or_panic(renderer: &Renderer, document: &Document, config: RenderConfig) -> Image {
    renderer
        .render_to_image(document, config)
        .unwrap_or_else(|error| panic!("render benchmark failed: {error}"))
}

fn bench_render(c: &mut Criterion) {
    let renderer = match Renderer::headless() {
        Ok(renderer) => renderer,
        Err(RenderError::NoAdapter) => {
            eprintln!("skipping full-renderer benchmarks: no GPU adapter available");
            return;
        }
        Err(error) => panic!("renderer construction failed: {error}"),
    };

    let document = many_shapes_document();
    let front = Camera::facing(WIDTH, HEIGHT);

    let mut angled = Camera::facing(WIDTH, HEIGHT);
    angled.eye.x += 120.0;

    let mut dolly = Camera::facing(WIDTH, HEIGHT);
    dolly.eye.z *= 0.55;

    let mut group = c.benchmark_group("render");
    group.bench_function("orthographic_many_shapes", |b| {
        b.iter(|| {
            black_box(render_or_panic(
                &renderer,
                black_box(&document),
                config(None),
            ))
        })
    });
    group.bench_function("camera_front_many_shapes", |b| {
        b.iter(|| {
            black_box(render_or_panic(
                &renderer,
                black_box(&document),
                config(Some(front)),
            ))
        })
    });
    group.bench_function("camera_angled_many_shapes", |b| {
        b.iter(|| {
            black_box(render_or_panic(
                &renderer,
                black_box(&document),
                config(Some(angled)),
            ))
        })
    });
    group.bench_function("camera_dolly_many_shapes", |b| {
        b.iter(|| {
            black_box(render_or_panic(
                &renderer,
                black_box(&document),
                config(Some(dolly)),
            ))
        })
    });
    group.finish();
}

criterion_group!(benches, bench_render);
criterion_main!(benches);
