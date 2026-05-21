//! Full-renderer benchmarks for `Renderer::render_to_image`.
//!
//! These benches exercise `Renderer::render_to_image` as it exists today:
//! per-call GPU adapter/device acquisition, pipeline creation, scene
//! tessellation, vertex upload, render pass, and readback. They are intended
//! for GPU-capable macOS runners and self-skip when no adapter is available,
//! so CI treats them as a timing smoke check rather than tracked CodSpeed data.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use svg3_dom::{parse, Document};
use svg3_render::{Camera, Image, RenderConfig, RenderError, Renderer};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn many_shapes_document() -> Document {
    let mut svg = String::from(
        r##"<svg width="320" height="240"><rect width="100%" height="100%" fill="#0f172a"/>"##,
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
    match renderer.render_to_image(document, config) {
        Ok(image) => image,
        Err(RenderError::NoAdapter) => panic!("GPU adapter disappeared during render benchmark"),
        Err(error) => panic!("render benchmark failed: {error}"),
    }
}

fn gpu_available(renderer: &Renderer) -> bool {
    let document = parse(r#"<svg><rect width="1" height="1"/></svg>"#)
        .expect("smoke benchmark fixture should parse");
    match renderer.render_to_image(
        &document,
        RenderConfig {
            width: 1,
            height: 1,
            ..RenderConfig::default()
        },
    ) {
        Ok(image) => {
            black_box(image);
            true
        }
        Err(RenderError::NoAdapter) => false,
        Err(error) => panic!("render benchmark GPU smoke test failed: {error}"),
    }
}

fn bench_render(c: &mut Criterion) {
    let renderer = Renderer::new();
    if !gpu_available(&renderer) {
        eprintln!("skipping full-renderer benchmarks: no GPU adapter available");
        return;
    }

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
