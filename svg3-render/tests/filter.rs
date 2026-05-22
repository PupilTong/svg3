//! End-to-end tests for SVG filter rendering.
//!
//! Each case parses an SVG document and renders it through
//! `Renderer::render_to_image`, then asserts on final pixels. These tests are
//! GPU-backed and self-skip on hosts without an adapter, matching the snapshot
//! suite's behavior.

use svg3_dom::parse;
use svg3_render::{Image, RenderConfig, RenderError, Renderer};

const CANVAS: u32 = 64;

fn skip_or_renderer(test: &str) -> Option<Renderer> {
    match Renderer::headless() {
        Ok(renderer) => Some(renderer),
        Err(RenderError::NoAdapter) => {
            eprintln!("skipping {test}: no GPU adapter");
            None
        }
        Err(error) => panic!("renderer construction failed: {error}"),
    }
}

fn render(renderer: &Renderer, svg: &str) -> Image {
    let document = parse(svg).expect("filter E2E fixture should parse");
    renderer
        .render_to_image(
            &document,
            RenderConfig {
                width: CANVAS,
                height: CANVAS,
                ..RenderConfig::default()
            },
        )
        .expect("filter E2E render failed")
}

fn assert_transparent(image: &Image, x: u32, y: u32) {
    let pixel = image.pixel(x, y);
    assert!(
        pixel[3] <= 4,
        "pixel ({x}, {y}) should be transparent, got {pixel:?}"
    );
}

fn assert_blue_halo(image: &Image, x: u32, y: u32) {
    let pixel = image.pixel(x, y);
    assert!(
        pixel[3] > 8 && pixel[2] > 8 && pixel[0] < pixel[2] && pixel[1] < pixel[2],
        "pixel ({x}, {y}) should be a blue blur halo, got {pixel:?}"
    );
}

#[test]
fn gaussian_blur_creates_halo_outside_source_geometry() {
    let Some(renderer) = skip_or_renderer("gaussian_blur_creates_halo_outside_source_geometry")
    else {
        return;
    };
    let sharp = render(
        &renderer,
        r#"<svg><rect x="24" y="24" width="16" height="16" fill="blue"/></svg>"#,
    );
    let blurred = render(
        &renderer,
        r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#soft)"/></svg>"##,
    );

    assert_transparent(&sharp, 20, 32);
    assert_blue_halo(&blurred, 20, 32);
    assert_transparent(&blurred, 4, 4);

    let centre = blurred.pixel(32, 32);
    assert!(
        centre[3] > 80 && centre[2] > 80,
        "blurred source centre should stay visibly blue, got {centre:?}"
    );
}

#[test]
fn std_deviation_two_value_form_blurs_each_axis_independently() {
    let Some(renderer) =
        skip_or_renderer("std_deviation_two_value_form_blurs_each_axis_independently")
    else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="x-only"><feGaussianBlur stdDeviation="6 0"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#x-only)"/></svg>"##,
    );

    assert_blue_halo(&image, 20, 32);
    assert_transparent(&image, 32, 20);
}

#[test]
fn quoted_fragment_filter_url_resolves() {
    let Some(renderer) = skip_or_renderer("quoted_fragment_filter_url_resolves") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter='url("#soft")'/></svg>"##,
    );

    assert_blue_halo(&image, 20, 32);
}

#[test]
fn zero_std_deviation_filter_keeps_edges_sharp() {
    let Some(renderer) = skip_or_renderer("zero_std_deviation_filter_keeps_edges_sharp") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="none"><feGaussianBlur stdDeviation="0"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#none)"/></svg>"##,
    );

    assert_transparent(&image, 20, 32);
    let centre = image.pixel(32, 32);
    assert!(
        centre[2] > 200 && centre[0] < 60 && centre[1] < 60 && centre[3] > 250,
        "zero-blur source centre should render as the original blue rect, got {centre:?}"
    );
}

#[test]
fn filtered_group_is_rendered_to_one_offscreen_surface() {
    let Some(renderer) = skip_or_renderer("filtered_group_is_rendered_to_one_offscreen_surface")
    else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="3"/></filter><g filter="url(#soft)"><rect x="22" y="22" width="10" height="20" fill="red"/><rect x="32" y="22" width="10" height="20" fill="blue"/></g></svg>"##,
    );

    let left_halo = image.pixel(18, 32);
    assert!(
        left_halo[3] > 4 && left_halo[0] > left_halo[2],
        "group blur should carry the red side outward, got {left_halo:?}"
    );
    let right_halo = image.pixel(46, 32);
    assert!(
        right_halo[3] > 4 && right_halo[2] > right_halo[0],
        "group blur should carry the blue side outward, got {right_halo:?}"
    );
}

#[test]
fn filtered_content_composites_in_svg_painter_order() {
    let Some(renderer) = skip_or_renderer("filtered_content_composites_in_svg_painter_order")
    else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="20" y="20" width="20" height="20" fill="red" filter="url(#soft)"/><rect x="36" y="20" width="20" height="20" fill="#11aa55"/></svg>"##,
    );

    let later_rect = image.pixel(44, 30);
    assert!(
        later_rect[1] > later_rect[0] && later_rect[1] > later_rect[2],
        "later green rect should paint over the earlier red filter result, got {later_rect:?}"
    );
}

#[test]
fn filter_definition_subtree_does_not_paint_directly() {
    let Some(renderer) = skip_or_renderer("filter_definition_subtree_does_not_paint_directly")
    else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="soft"><rect width="64" height="64" fill="red"/><feGaussianBlur stdDeviation="4"/></filter></svg>"##,
    );

    assert_transparent(&image, 32, 32);
}
