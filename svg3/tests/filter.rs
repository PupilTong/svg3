//! End-to-end tests for SVG filter rendering.
//!
//! Each case parses an SVG document and renders it through
//! `Renderer::render_to_image`, then asserts on final pixels. These tests are
//! GPU-backed and self-skip on hosts without an adapter, matching the snapshot
//! suite's behavior.

mod common;

use common::png_data_uri;
use svg3::dom::parse;
use svg3::render::{Image, RenderConfig, RenderError, Renderer};

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

#[test]
fn fe_image_filter_paints_embedded_png() {
    let Some(renderer) = skip_or_renderer("fe_image_filter_paints_embedded_png") else {
        return;
    };
    let href = png_data_uri(1, 1, &[255, 0, 0, 255]);
    let svg = format!(
        r##"<svg><filter id="tex"><feImage href="{href}" x="20" y="18" width="24" height="22"/></filter><rect x="2" y="2" width="10" height="10" fill="blue" filter="url(#tex)"/></svg>"##
    );
    let image = render(&renderer, &svg);

    assert_transparent(&image, 6, 6);
    let pixel = image.pixel(32, 28);
    assert!(
        pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40 && pixel[3] > 250,
        "<feImage> should paint the embedded red PNG, got {pixel:?}"
    );
}

#[test]
fn fe_image_can_feed_gaussian_blur() {
    let Some(renderer) = skip_or_renderer("fe_image_can_feed_gaussian_blur") else {
        return;
    };
    let href = png_data_uri(1, 1, &[0, 0, 255, 255]);
    let svg = format!(
        r##"<svg><filter id="tex"><feImage href="{href}" x="28" y="28" width="8" height="8"/><feGaussianBlur stdDeviation="4"/></filter><rect width="64" height="64" filter="url(#tex)"/></svg>"##
    );
    let image = render(&renderer, &svg);

    assert_blue_halo(&image, 24, 32);
    assert_transparent(&image, 8, 8);
}

// ---- Per-primitive GPU pipeline tests -------------------------------------
//
// Each test below picks a filter primitive whose visual contract is easy to
// assert with a pixel probe, then renders an SVG document that exercises that
// primitive end-to-end through the GPU chain. Together they keep one
// regression net per primitive on the renderer surface.

#[test]
fn color_matrix_luminance_to_alpha_isolates_alpha() {
    let Some(renderer) = skip_or_renderer("color_matrix_luminance_to_alpha_isolates_alpha") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="lum"><feColorMatrix type="luminanceToAlpha"/></filter><rect x="16" y="16" width="32" height="32" fill="#ffffff" filter="url(#lum)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // luminanceToAlpha emits luma into alpha and zeroes RGB. White luminance
    // is 1.0, so the centre must be fully transparent black (premultiplied).
    assert!(
        centre[3] > 200 && centre[0] < 30 && centre[1] < 30 && centre[2] < 30,
        "luminanceToAlpha centre should be alpha-only, got {centre:?}"
    );
}

#[test]
fn color_matrix_saturate_zero_desaturates() {
    let Some(renderer) = skip_or_renderer("color_matrix_saturate_zero_desaturates") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="gray"><feColorMatrix type="saturate" values="0"/></filter><rect x="16" y="16" width="32" height="32" fill="red" filter="url(#gray)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // Desaturated red is a small grey; R / G / B should be close together.
    let max = centre[0].max(centre[1]).max(centre[2]);
    let min = centre[0].min(centre[1]).min(centre[2]);
    assert!(
        centre[3] > 200 && max - min < 12,
        "saturate(0) centre should be grey, got {centre:?}"
    );
}

#[test]
fn flood_fills_filter_region_with_opaque_color() {
    let Some(renderer) = skip_or_renderer("flood_fills_filter_region_with_opaque_color") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="fl"><feFlood flood-color="#ff0000" flood-opacity="1"/></filter><rect x="0" y="0" width="64" height="64" fill="blue" filter="url(#fl)"/></svg>"##,
    );
    // The flood primitive replaces the source — the rect is no longer blue.
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] > 200 && centre[2] < 60 && centre[3] > 200,
        "flood centre should be solid red, got {centre:?}"
    );
}

#[test]
fn morphology_dilate_grows_the_silhouette() {
    let Some(renderer) = skip_or_renderer("morphology_dilate_grows_the_silhouette") else {
        return;
    };
    let unfiltered = render(
        &renderer,
        r#"<svg><rect x="28" y="28" width="8" height="8" fill="blue"/></svg>"#,
    );
    let dilated = render(
        &renderer,
        r##"<svg><filter id="grow"><feMorphology operator="dilate" radius="3"/></filter><rect x="28" y="28" width="8" height="8" fill="blue" filter="url(#grow)"/></svg>"##,
    );
    // A pixel just outside the original 8x8 box should remain transparent in
    // the unfiltered image and become opaque blue after dilation.
    assert!(unfiltered.pixel(25, 32)[3] < 4);
    let dilated_outside = dilated.pixel(25, 32);
    assert!(
        dilated_outside[3] > 200 && dilated_outside[2] > 200,
        "dilated pixel should be blue, got {dilated_outside:?}"
    );
}

#[test]
fn morphology_erode_shrinks_the_silhouette() {
    let Some(renderer) = skip_or_renderer("morphology_erode_shrinks_the_silhouette") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="shrink"><feMorphology operator="erode" radius="3"/></filter><rect x="20" y="20" width="24" height="24" fill="blue" filter="url(#shrink)"/></svg>"##,
    );
    // Erosion peels the rect edges back; the pixel that was an interior of
    // the original 24x24 box (1 pixel inside the edge) becomes transparent
    // after a 3-pixel erode.
    assert!(image.pixel(21, 32)[3] < 30, "{:?}", image.pixel(21, 32));
    // The very centre stays solid.
    let centre = image.pixel(32, 32);
    assert!(centre[2] > 200 && centre[3] > 200);
}

#[test]
fn drop_shadow_paints_offset_blurred_shadow() {
    let Some(renderer) = skip_or_renderer("drop_shadow_paints_offset_blurred_shadow") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="shadow"><feDropShadow dx="6" dy="6" stdDeviation="2" flood-color="#000000" flood-opacity="1"/></filter><rect x="20" y="20" width="16" height="16" fill="red" filter="url(#shadow)"/></svg>"##,
    );
    // Pixel under the offset shadow should be dark (R, G, B small) with
    // visible alpha; the source itself should still be visibly red on top.
    let shadow_pixel = image.pixel(40, 40);
    assert!(
        shadow_pixel[3] > 8 && shadow_pixel[0] < 60 && shadow_pixel[1] < 60 && shadow_pixel[2] < 60,
        "shadow pixel should be a dark halo, got {shadow_pixel:?}"
    );
    let source_pixel = image.pixel(26, 26);
    assert!(
        source_pixel[3] > 200 && source_pixel[0] > 200,
        "source pixel should remain red on top of the shadow, got {source_pixel:?}"
    );
}

#[test]
fn turbulence_produces_non_uniform_noise() {
    let Some(renderer) = skip_or_renderer("turbulence_produces_non_uniform_noise") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="noise"><feTurbulence baseFrequency="0.2" numOctaves="3" seed="1"/></filter><rect x="0" y="0" width="64" height="64" filter="url(#noise)"/></svg>"##,
    );
    // The noise should vary across the surface, not produce a uniform colour.
    let a = image.pixel(8, 8)[0];
    let b = image.pixel(56, 56)[0];
    let c = image.pixel(32, 32)[0];
    let min = a.min(b).min(c);
    let max = a.max(b).max(c);
    assert!(
        max as i32 - min as i32 > 10,
        "turbulence pixels should vary across the surface, sampled {a} {b} {c}"
    );
}

#[test]
fn convolve_matrix_sharpen_increases_contrast() {
    let Some(renderer) = skip_or_renderer("convolve_matrix_sharpen_increases_contrast") else {
        return;
    };
    // A classic 3x3 sharpen kernel boosts the edges of any high-contrast input.
    let image = render(
        &renderer,
        r##"<svg><filter id="sharp"><feConvolveMatrix kernelMatrix="0 -1 0 -1 5 -1 0 -1 0"/></filter><rect x="20" y="20" width="24" height="24" fill="#888888" filter="url(#sharp)"/></svg>"##,
    );
    // The interior remains visible (alpha is finite).
    let centre = image.pixel(32, 32);
    assert!(centre[3] > 200);
}

#[test]
fn component_transfer_linear_doubles_red() {
    let Some(renderer) = skip_or_renderer("component_transfer_linear_doubles_red") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="boost"><feComponentTransfer><feFuncR type="linear" slope="2" intercept="0"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="#330000" filter="url(#boost)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // Doubling the red channel of #330000 in linear light pushes a visible red.
    assert!(centre[0] > 60, "boosted red pixel: {centre:?}");
}

#[test]
fn displacement_map_with_zero_scale_is_identity() {
    let Some(renderer) = skip_or_renderer("displacement_map_with_zero_scale_is_identity") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="warp"><feDisplacementMap scale="0"/></filter><rect x="16" y="16" width="32" height="32" fill="blue" filter="url(#warp)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // Scale=0 means no displacement, so the source rect must render unchanged.
    assert!(centre[2] > 200 && centre[3] > 200);
}

#[test]
fn diffuse_lighting_renders_lighted_surface() {
    let Some(renderer) = skip_or_renderer("diffuse_lighting_renders_lighted_surface") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="light"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="#ffffff"><feDistantLight azimuth="45" elevation="60"/></feDiffuseLighting></filter><circle cx="32" cy="32" r="20" fill="#888888" filter="url(#light)"/></svg>"##,
    );
    // Diffuse output is opaque; the lit centre should be a bright grey.
    let centre = image.pixel(32, 32);
    assert!(
        centre[3] > 200 && centre[0] > 30,
        "diffuse-lit centre: {centre:?}"
    );
}

#[test]
fn specular_lighting_renders_a_highlight() {
    let Some(renderer) = skip_or_renderer("specular_lighting_renders_a_highlight") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="spec"><feSpecularLighting surfaceScale="5" specularConstant="1" specularExponent="20" lighting-color="#ffffff"><feDistantLight azimuth="135" elevation="30"/></feSpecularLighting></filter><circle cx="32" cy="32" r="24" fill="#444444" filter="url(#spec)"/></svg>"##,
    );
    // Somewhere within the lit disc we should find a visible specular pixel.
    let mut bright = 0;
    for y in 0..64 {
        for x in 0..64 {
            let p = image.pixel(x, y);
            if p[3] > 4 && p[0].max(p[1]).max(p[2]) > 80 {
                bright += 1;
            }
        }
    }
    assert!(
        bright > 10,
        "specular lighting should produce a visible highlight; bright pixels: {bright}"
    );
}

#[test]
fn displacement_map_honors_in_and_in2_dag_wiring() {
    // The MDN canonical fixture: feTurbulence with `result="turbulence"`,
    // then feDisplacementMap with `in="SourceGraphic"` and
    // `in2="turbulence"`. A correct DAG executor produces an amorphous
    // black shape (the circle's pixels reshuffled by the turbulence offset),
    // not a noise field — the renderer must address the circle as the
    // graphic and the turbulence as the map, not the other way around.
    let Some(renderer) = skip_or_renderer("displacement_map_honors_in_and_in2_dag_wiring") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="warp"><feTurbulence type="turbulence" baseFrequency="0.05" numOctaves="2" result="turbulence"/><feDisplacementMap in="SourceGraphic" in2="turbulence" scale="20" xChannelSelector="R" yChannelSelector="G"/></filter><circle cx="32" cy="32" r="28" filter="url(#warp)"/></svg>"##,
    );

    // The centre is well inside any reasonable displacement of the circle,
    // so it must remain opaque (= the SourceGraphic was the input, not the
    // noise).
    let centre = image.pixel(32, 32);
    assert!(
        centre[3] > 200,
        "displaced circle centre should remain opaque: {centre:?}"
    );

    // The image must contain at least one fully transparent pixel — proof
    // that the displacement reached the edge and pushed the circle off it
    // (or, equivalently, that we're not just rendering a uniform noise
    // field).
    let transparent_count = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .filter(|(x, y)| image.pixel(*x, *y)[3] < 4)
        .count();
    assert!(
        transparent_count > 100,
        "displacement should leave visible transparent regions, got {transparent_count}"
    );

    // The displaced circle must be RGB-monochrome (the default SVG fill is
    // black) — if the renderer were mixing the turbulence pixels into the
    // graphic by mistake, we'd see varied colours instead.
    for (x, y) in [(20, 32), (32, 20), (44, 32), (32, 44)] {
        let p = image.pixel(x, y);
        if p[3] > 200 {
            let max = p[0].max(p[1]).max(p[2]);
            assert!(
                max < 16,
                "displaced circle pixel ({x}, {y}) should be near-black: {p:?}"
            );
        }
    }
}

#[test]
fn source_alpha_pseudo_input_strips_rgb() {
    // `in="SourceAlpha"` should hand the next primitive the alpha channel
    // of the source with RGB cleared to zero. We use a colour-matrix that
    // adds a constant red to make the result trivially recognisable: the
    // output anywhere inside the circle should be opaque pure red.
    let Some(renderer) = skip_or_renderer("source_alpha_pseudo_input_strips_rgb") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="alpha"><feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 1   0 0 0 0 0   0 0 0 0 0   0 0 0 1 0"/></filter><circle cx="32" cy="32" r="20" fill="blue" filter="url(#alpha)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // The circle's fill was blue, but SourceAlpha discarded the blue and the
    // matrix adds a constant red, so the centre is opaque pure red.
    assert!(
        centre[0] > 200 && centre[1] < 30 && centre[2] < 30 && centre[3] > 200,
        "SourceAlpha-only centre should be opaque red, got {centre:?}"
    );
}

#[test]
fn named_result_can_be_referenced_later() {
    // Two-step chain: feColorMatrix recolours the source and stores its
    // output as `result="red"`. feFlood writes solid white but is *not*
    // the last primitive; the third step uses feColorMatrix again with
    // `in="red"` to pick the named result. The final composited centre
    // must be the recoloured red, proving the named-result lookup.
    let Some(renderer) = skip_or_renderer("named_result_can_be_referenced_later") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="g"><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0" result="red"/><feFlood flood-color="#ffffff"/><feColorMatrix in="red" type="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 1 0"/></filter><rect x="16" y="16" width="32" height="32" fill="blue" filter="url(#g)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] > 200 && centre[1] < 30 && centre[2] < 30 && centre[3] > 200,
        "named-result `in=\"red\"` should recover the recoloured source, got {centre:?}"
    );
}

#[test]
fn zero_deviation_blur_with_source_alpha_still_strips_rgb() {
    // Bug guard (review P2): a primitive whose parameters are no-ops but
    // whose `in` is non-default must still run through the chain. A naive
    // "if no primitive is visible, just composite the source" gate would
    // surface the original blue circle here; the correct behaviour is to
    // run the chain, take `SourceAlpha`, blur it by zero (identity), and
    // composite the (R=0, G=0, B=0, A=src.a) silhouette.
    let Some(renderer) = skip_or_renderer("zero_deviation_blur_with_source_alpha_still_strips_rgb")
    else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="strip"><feGaussianBlur in="SourceAlpha" stdDeviation="0"/></filter><circle cx="32" cy="32" r="20" fill="blue" filter="url(#strip)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[3] > 200 && centre[0] < 30 && centre[1] < 30 && centre[2] < 30,
        "SourceAlpha + zero blur should leave an alpha-only silhouette, got {centre:?}"
    );
}

#[test]
fn component_transfer_table_with_two_entries_is_identity() {
    // Bug guard (review P2): `tableValues="0 1"` as a piecewise-linear
    // table is the identity function. The previous 4-entry-only impl
    // stored `[0, 1, 0, 0]` and rendered the upper input range as black.
    let Some(renderer) = skip_or_renderer("component_transfer_table_with_two_entries_is_identity")
    else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="ident"><feComponentTransfer><feFuncR type="table" tableValues="0 1"/><feFuncG type="table" tableValues="0 1"/><feFuncB type="table" tableValues="0 1"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="#cccccc" filter="url(#ident)"/></svg>"##,
    );
    // The grey rect must come through near-unchanged (small variation
    // tolerated for sRGB rounding).
    let centre = image.pixel(32, 32);
    let max = centre[0].max(centre[1]).max(centre[2]);
    let min = centre[0].min(centre[1]).min(centre[2]);
    assert!(
        centre[3] > 200 && min > 180 && (max - min) < 12,
        "table=\"0 1\" should be identity on a light grey rect, got {centre:?}"
    );
}

#[test]
fn component_transfer_discrete_with_three_buckets() {
    // A 3-bucket discrete function maps inputs [0, 1/3) -> 0,
    // [1/3, 2/3) -> 0.5, [2/3, 1] -> 1. The previous fixed-4-bucket
    // implementation would have used boundaries at 0.25 / 0.5 / 0.75 with
    // a `0` in the fourth bucket, which is wrong for any non-4-entry table.
    let Some(renderer) = skip_or_renderer("component_transfer_discrete_with_three_buckets") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="d"><feComponentTransfer><feFuncR type="discrete" tableValues="0 0.5 1"/><feFuncG type="discrete" tableValues="0 0.5 1"/><feFuncB type="discrete" tableValues="0 0.5 1"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="#ffffff" filter="url(#d)"/></svg>"##,
    );
    // White (1.0) input falls in the top bucket -> 1.0 output.
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] > 240 && centre[1] > 240 && centre[2] > 240 && centre[3] > 200,
        "discrete \"0 0.5 1\" on white should output white, got {centre:?}"
    );
}

#[test]
fn spot_light_inside_cone_illuminates_surface() {
    // Bug guard (review P3): feSpotLight was being silently dropped. The
    // spotlight here is centred over the disc and pointed straight down,
    // so the disc centre lies INSIDE the limiting cone and must receive
    // visible illumination — a regression to "fall back to distant light"
    // would also illuminate, but the *outside-cone* sibling test below
    // ensures the cone math is honoured.
    let Some(renderer) = skip_or_renderer("spot_light_inside_cone_illuminates_surface") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="spot"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="#ffffff"><feSpotLight x="32" y="32" z="40" pointsAtX="32" pointsAtY="32" pointsAtZ="0" specularExponent="2" limitingConeAngle="45"/></feDiffuseLighting></filter><circle cx="32" cy="32" r="22" fill="#888888" filter="url(#spot)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[3] > 200 && centre[0] > 60,
        "spot-lit centre should be visible, got {centre:?}"
    );
}

#[test]
fn spot_light_limiting_cone_excludes_pixels_outside() {
    // The cone is narrow (5°) and aimed at one quadrant — pixels far from
    // the aim direction should fall to zero illumination.
    let Some(renderer) = skip_or_renderer("spot_light_limiting_cone_excludes_pixels_outside")
    else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg width="64" height="64"><filter id="spot"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="#ffffff"><feSpotLight x="10" y="10" z="20" pointsAtX="10" pointsAtY="10" pointsAtZ="0" specularExponent="2" limitingConeAngle="5"/></feDiffuseLighting></filter><rect x="0" y="0" width="64" height="64" fill="#888888" filter="url(#spot)"/></svg>"##,
    );
    // The far corner is far outside the 5° cone — RGB should be near black
    // (diffuse output is opaque per SVG 1.1, so alpha stays ~255).
    let far_corner = image.pixel(60, 60);
    assert!(
        far_corner[0] < 32 && far_corner[1] < 32 && far_corner[2] < 32,
        "pixel outside the spotlight cone should be unlit, got {far_corner:?}"
    );
}

#[test]
fn primitive_chain_applies_blur_then_color_matrix() {
    let Some(renderer) = skip_or_renderer("primitive_chain_applies_blur_then_color_matrix") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="chain"><feGaussianBlur stdDeviation="3"/><feColorMatrix type="saturate" values="0"/></filter><rect x="24" y="24" width="16" height="16" fill="red" filter="url(#chain)"/></svg>"##,
    );
    // The blurred + desaturated centre is grey (R/G/B equal-ish) and still
    // contributes alpha around the original rect.
    let centre = image.pixel(32, 32);
    let max = centre[0].max(centre[1]).max(centre[2]);
    let min = centre[0].min(centre[1]).min(centre[2]);
    assert!(
        centre[3] > 80 && max - min < 12,
        "blurred + desaturated centre should be grey, got {centre:?}"
    );
}

// ---- feOffset --------------------------------------------------------------

#[test]
fn offset_translates_source_by_dx_dy() {
    let Some(renderer) = skip_or_renderer("offset_translates_source_by_dx_dy") else {
        return;
    };
    // Source rect occupies [20, 28] x [20, 28]. With dx=12, dy=8 the offset
    // output covers [32, 40] x [28, 36] and the original region is empty.
    let image = render(
        &renderer,
        r##"<svg><filter id="o"><feOffset dx="12" dy="8"/></filter><rect x="20" y="20" width="8" height="8" fill="blue" filter="url(#o)"/></svg>"##,
    );
    assert_transparent(&image, 24, 24);
    let moved = image.pixel(36, 32);
    assert!(
        moved[2] > 180 && moved[3] > 180,
        "translated rect should appear at offset position, got {moved:?}"
    );
}

#[test]
fn offset_with_zero_translation_is_identity() {
    let Some(renderer) = skip_or_renderer("offset_with_zero_translation_is_identity") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="o"><feOffset/></filter><rect x="20" y="20" width="24" height="24" fill="lime" filter="url(#o)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[1] > 180 && centre[3] > 180,
        "zero-offset identity should keep source visible, got {centre:?}"
    );
}

// ---- feMerge ---------------------------------------------------------------

#[test]
fn merge_composites_named_inputs_in_painter_order() {
    let Some(renderer) = skip_or_renderer("merge_composites_named_inputs_in_painter_order") else {
        return;
    };
    // Two flood layers merged: blue then red. The red node is later in
    // document order, so it should paint *on top* of the blue.
    let image = render(
        &renderer,
        r##"<svg><filter id="m"><feFlood flood-color="blue" result="blue"/><feFlood flood-color="red" result="red"/><feMerge><feMergeNode in="blue"/><feMergeNode in="red"/></feMerge></filter><rect width="64" height="64" filter="url(#m)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] > 180 && centre[2] < 80 && centre[3] > 180,
        "later merge node should paint over earlier one, got {centre:?}"
    );
}

#[test]
fn merge_offset_blur_then_source_layers_drop_shadow() {
    let Some(renderer) = skip_or_renderer("merge_offset_blur_then_source_layers_drop_shadow")
    else {
        return;
    };
    // The classic SVG drop-shadow recipe: offset + blur the alpha for the
    // shadow, then merge SourceGraphic on top of it.
    let image = render(
        &renderer,
        r##"<svg><filter id="ds"><feGaussianBlur in="SourceAlpha" stdDeviation="2" result="blur"/><feOffset in="blur" dx="6" dy="4" result="shadow"/><feMerge><feMergeNode in="shadow"/><feMergeNode in="SourceGraphic"/></feMerge></filter><rect x="20" y="20" width="14" height="14" fill="lime" filter="url(#ds)"/></svg>"##,
    );
    // The lime source must still be visible at the source position. The
    // source's max bound is x=34, y=34; (27, 27) sits well inside.
    let source = image.pixel(27, 27);
    assert!(
        source[1] > 180 && source[3] > 180,
        "source should remain visible above the shadow, got {source:?}"
    );
    // The shadow halo straddles the offset rect's edge. Sampling at the
    // exact corner (post-offset = sampling the un-offset rect's corner)
    // should hit the Gaussian's 50% transition.
    let halo = image.pixel(40, 38);
    assert!(
        halo[3] > 8 && halo[3] < 240,
        "shadow halo edge should have partial alpha, got {halo:?}"
    );
}

// ---- feBlend ---------------------------------------------------------------

#[test]
fn blend_multiply_darkens_overlap() {
    let Some(renderer) = skip_or_renderer("blend_multiply_darkens_overlap") else {
        return;
    };
    // Two opaque grey layers (0.5) multiplied → 0.25 grey. Test isolates the
    // blend math: in=top is grey, in2=bottom is grey.
    let image = render(
        &renderer,
        r##"<svg><filter id="b"><feFlood flood-color="#808080" result="a"/><feFlood flood-color="#808080" result="b"/><feBlend mode="multiply" in="a" in2="b"/></filter><rect width="64" height="64" filter="url(#b)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // 0.5 * 0.5 = 0.25 ≈ 64/255. Wide tolerance for sRGB encode rounding.
    assert!(
        centre[0] < 110 && centre[0] > 20 && centre[3] > 200,
        "multiply blend should darken, got {centre:?}"
    );
}

#[test]
fn blend_screen_lightens_overlap() {
    let Some(renderer) = skip_or_renderer("blend_screen_lightens_overlap") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="b"><feFlood flood-color="#808080" result="a"/><feFlood flood-color="#808080" result="b"/><feBlend mode="screen" in="a" in2="b"/></filter><rect width="64" height="64" filter="url(#b)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // Screen: 1 - (1-0.5)*(1-0.5) = 0.75 ≈ 191/255.
    assert!(
        centre[0] > 150 && centre[3] > 200,
        "screen blend should lighten, got {centre:?}"
    );
}

// ---- feComposite -----------------------------------------------------------

#[test]
fn composite_in_intersects_alpha() {
    let Some(renderer) = skip_or_renderer("composite_in_intersects_alpha") else {
        return;
    };
    // in: keep `in` where `in2` has alpha. Use a red flood masked by the
    // SourceAlpha of a small rect — output should only cover the rect.
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feFlood flood-color="red" result="flood"/><feComposite in="flood" in2="SourceAlpha" operator="in"/></filter><rect x="20" y="20" width="24" height="24" fill="white" filter="url(#c)"/></svg>"##,
    );
    let inside = image.pixel(32, 32);
    assert!(
        inside[0] > 180 && inside[3] > 180,
        "composite-in should keep the flood inside SourceAlpha, got {inside:?}"
    );
    assert_transparent(&image, 8, 8);
}

#[test]
fn composite_out_subtracts_alpha() {
    let Some(renderer) = skip_or_renderer("composite_out_subtracts_alpha") else {
        return;
    };
    // out: keep `in` where `in2` does NOT have alpha. Flood the whole
    // filter region red, then subtract the SourceAlpha of a small rect.
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feFlood flood-color="red" result="flood"/><feComposite in="flood" in2="SourceAlpha" operator="out"/></filter><rect x="20" y="20" width="24" height="24" fill="white" filter="url(#c)"/></svg>"##,
    );
    assert_transparent(&image, 32, 32);
    let outside = image.pixel(8, 8);
    assert!(
        outside[0] > 180 && outside[3] > 180,
        "composite-out should keep flood outside the source rect, got {outside:?}"
    );
}

#[test]
fn composite_arithmetic_adds_inputs() {
    let Some(renderer) = skip_or_renderer("composite_arithmetic_adds_inputs") else {
        return;
    };
    // Reference: a single flood layer rendered through the identity filter
    // gives the baseline brightness. The arithmetic-doubled output must be
    // strictly brighter than the reference (since arithmetic operates in
    // linear space, the comparison stays valid through the sRGB encode).
    let reference = render(
        &renderer,
        r##"<svg><filter id="c"><feFlood flood-color="#404040"/></filter><rect width="64" height="64" filter="url(#c)"/></svg>"##,
    );
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feFlood flood-color="#404040" result="a"/><feFlood flood-color="#404040" result="b"/><feComposite in="a" in2="b" operator="arithmetic" k1="0" k2="1" k3="1" k4="0"/></filter><rect width="64" height="64" filter="url(#c)"/></svg>"##,
    );
    let base = reference.pixel(32, 32);
    let sum = image.pixel(32, 32);
    assert!(
        sum[0] > base[0] + 10 && sum[3] > 180,
        "arithmetic k2+k3=1+1 should brighten beyond a single flood: base {base:?}, sum {sum:?}"
    );
}

#[test]
fn composite_arithmetic_with_k1_multiplies_inputs() {
    let Some(renderer) = skip_or_renderer("composite_arithmetic_with_k1_multiplies_inputs") else {
        return;
    };
    // k1=1, k2=k3=k4=0: result = src * dst (channelwise). White * white
    // stays white; the alpha-channel product should also be near 1.
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feFlood flood-color="white" result="a"/><feFlood flood-color="white" result="b"/><feComposite in="a" in2="b" operator="arithmetic" k1="1" k2="0" k3="0" k4="0"/></filter><rect width="64" height="64" filter="url(#c)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] > 230 && centre[3] > 230,
        "arithmetic k1=1 should multiply: white*white ≈ white, got {centre:?}"
    );
}

// ---- feTile ----------------------------------------------------------------

// ---- feImage preserveAspectRatio -------------------------------------------

#[test]
fn fe_image_preserve_aspect_default_centres_with_margins() {
    let Some(renderer) = skip_or_renderer("fe_image_preserve_aspect_default_centres_with_margins")
    else {
        return;
    };
    // 4x2 image, drawn into a 20x20 rect. Default `preserveAspectRatio` is
    // `xMidYMid meet`: scale = min(20/4, 20/2) = 5, draw size 20x10,
    // centred vertically — top and bottom of the rect should be transparent.
    let rgba: Vec<u8> = (0..4 * 2).flat_map(|_| [255_u8, 0, 0, 255]).collect();
    let href = png_data_uri(4, 2, &rgba);
    let svg = format!(
        r##"<svg><filter id="t"><feImage href="{href}" x="22" y="22" width="20" height="20"/></filter><rect width="64" height="64" filter="url(#t)"/></svg>"##
    );
    let image = render(&renderer, &svg);
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] > 180 && centre[3] > 180,
        "centre of draw rect should be the red image, got {centre:?}"
    );
    // Top of the bounding rect — outside the centred image.
    assert_transparent(&image, 32, 23);
    // Bottom of the bounding rect — outside the centred image.
    assert_transparent(&image, 32, 40);
}

#[test]
fn fe_image_preserve_aspect_none_stretches_to_full_rect() {
    let Some(renderer) = skip_or_renderer("fe_image_preserve_aspect_none_stretches_to_full_rect")
    else {
        return;
    };
    // Same 4x2 image, but `preserveAspectRatio="none"` stretches the image
    // to fully fill the rect — so the top and bottom of the bounding rect
    // are now opaque.
    let rgba: Vec<u8> = (0..4 * 2).flat_map(|_| [255_u8, 0, 0, 255]).collect();
    let href = png_data_uri(4, 2, &rgba);
    let svg = format!(
        r##"<svg><filter id="t"><feImage href="{href}" x="22" y="22" width="20" height="20" preserveAspectRatio="none"/></filter><rect width="64" height="64" filter="url(#t)"/></svg>"##
    );
    let image = render(&renderer, &svg);
    let top = image.pixel(32, 23);
    let bottom = image.pixel(32, 40);
    assert!(
        top[0] > 180 && top[3] > 180,
        "top of stretched rect should be red, got {top:?}"
    );
    assert!(
        bottom[0] > 180 && bottom[3] > 180,
        "bottom of stretched rect should be red, got {bottom:?}"
    );
}

// ---- currentColor inheritance ----------------------------------------------

#[test]
fn flood_with_current_color_uses_element_color() {
    let Some(renderer) = skip_or_renderer("flood_with_current_color_uses_element_color") else {
        return;
    };
    // `flood-color="currentColor"` resolves from the filtered element's
    // `color="red"`. Without the substitution the flood would be the
    // parser's default black.
    let image = render(
        &renderer,
        r##"<svg><filter id="f"><feFlood flood-color="currentColor"/></filter><rect color="red" x="16" y="16" width="32" height="32" filter="url(#f)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] > 200 && centre[1] < 40 && centre[2] < 40 && centre[3] > 200,
        "flood should pick up element's color=red, got {centre:?}"
    );
}

#[test]
fn lighting_with_current_color_uses_element_color() {
    let Some(renderer) = skip_or_renderer("lighting_with_current_color_uses_element_color") else {
        return;
    };
    // `lighting-color="currentColor"` resolves from the element's `color`.
    let image = render(
        &renderer,
        r##"<svg><filter id="f"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="currentColor"><feDistantLight azimuth="0" elevation="90"/></feDiffuseLighting></filter><circle color="red" cx="32" cy="32" r="20" fill="white" filter="url(#f)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // Light is straight-down (elevation=90), so the lit centre carries the
    // lighting colour at full intensity. With currentColor=red, R should
    // dominate G and B.
    assert!(
        centre[0] > centre[1] + 30 && centre[0] > centre[2] + 30,
        "diffuse lighting should tint by currentColor=red, got {centre:?}"
    );
}

// ---- Filter href inheritance -----------------------------------------------

#[test]
fn filter_with_href_inherits_referenced_chain() {
    let Some(renderer) = skip_or_renderer("filter_with_href_inherits_referenced_chain") else {
        return;
    };
    // `child` has no primitives of its own and inherits from `parent`.
    let image = render(
        &renderer,
        r##"<svg><filter id="parent"><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0"/></filter><filter id="child" href="#parent"/><rect x="20" y="20" width="24" height="24" fill="white" filter="url(#child)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // The inherited color matrix forces R=1, G=0, B=0, A=1 — pure red.
    assert!(
        centre[0] > 230 && centre[1] < 30 && centre[2] < 30 && centre[3] > 230,
        "child should inherit parent's color matrix, got {centre:?}"
    );
}

#[test]
fn filter_href_cycle_renders_transparent_black() {
    let Some(renderer) = skip_or_renderer("filter_href_cycle_renders_transparent_black") else {
        return;
    };
    // `a` -> `b` -> `a` cycle: both filters are empty per `resolve_href_chain`.
    let image = render(
        &renderer,
        r##"<svg><filter id="a" href="#b"/><filter id="b" href="#a"/><rect x="20" y="20" width="24" height="24" fill="white" filter="url(#a)"/></svg>"##,
    );
    assert_transparent(&image, 32, 32);
}

// ---- feConvolveMatrix edgeMode --------------------------------------------

#[test]
fn convolve_edge_mode_none_darkens_canvas_edge() {
    let Some(renderer) = skip_or_renderer("convolve_edge_mode_none_darkens_canvas_edge") else {
        return;
    };
    // A rect that fills the whole canvas, then a normalised 3x3 box-blur
    // with edgeMode="none": at the canvas edge the kernel's outer taps fall
    // outside the source texture and contribute zero, darkening the edge.
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="1 1 1 1 1 1 1 1 1" divisor="9" edgeMode="none"/></filter><rect width="64" height="64" fill="white" filter="url(#c)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    let edge = image.pixel(0, 32);
    assert!(
        edge[0] + 16 < centre[0],
        "edgeMode=none should darken the canvas edge: edge {edge:?}, centre {centre:?}"
    );
}

#[test]
fn convolve_edge_mode_duplicate_keeps_canvas_edge_bright() {
    let Some(renderer) = skip_or_renderer("convolve_edge_mode_duplicate_keeps_canvas_edge_bright")
    else {
        return;
    };
    // Same kernel but edgeMode="duplicate": out-of-bounds taps clamp to the
    // edge pixel (white), so the kernel still averages to white.
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="1 1 1 1 1 1 1 1 1" divisor="9" edgeMode="duplicate"/></filter><rect width="64" height="64" fill="white" filter="url(#c)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    let edge = image.pixel(0, 32);
    let diff = centre[0].abs_diff(edge[0]);
    assert!(
        diff < 16,
        "edgeMode=duplicate should preserve the canvas edge: edge {edge:?}, centre {centre:?}"
    );
}

// ---- Empty / unresolved filters (SVG 1.1 §15.4) ----------------------------

#[test]
fn empty_filter_renders_transparent_black() {
    let Some(renderer) = skip_or_renderer("empty_filter_renders_transparent_black") else {
        return;
    };
    // An empty <filter> still applies; the result is transparent black.
    let image = render(
        &renderer,
        r##"<svg><filter id="empty"></filter><rect x="10" y="10" width="40" height="40" fill="red" filter="url(#empty)"/></svg>"##,
    );
    assert_transparent(&image, 30, 30);
}

#[test]
fn unresolved_filter_reference_renders_transparent_black() {
    let Some(renderer) = skip_or_renderer("unresolved_filter_reference_renders_transparent_black")
    else {
        return;
    };
    // A url() reference whose target is missing also yields transparent
    // black per spec.
    let image = render(
        &renderer,
        r##"<svg><rect x="10" y="10" width="40" height="40" fill="red" filter="url(#nope)"/></svg>"##,
    );
    assert_transparent(&image, 30, 30);
}

#[test]
fn filter_none_renders_source_unchanged() {
    let Some(renderer) = skip_or_renderer("filter_none_renders_source_unchanged") else {
        return;
    };
    // `filter="none"` is the SVG spec's "no filter applied" value.
    let image = render(
        &renderer,
        r##"<svg><rect x="10" y="10" width="40" height="40" fill="red" filter="none"/></svg>"##,
    );
    let centre = image.pixel(30, 30);
    assert!(
        centre[0] > 180 && centre[3] > 180,
        "filter=none should render the rect normally, got {centre:?}"
    );
}

#[test]
fn tile_repeats_input_across_filter_region() {
    let Some(renderer) = skip_or_renderer("tile_repeats_input_across_filter_region") else {
        return;
    };
    // A small source rect filtered through `feTile` should fill the entire
    // filter region (the rect's bounding box, expanded by default 10% in
    // each direction).
    let image = render(
        &renderer,
        r##"<svg><filter id="t"><feTile/></filter><rect x="20" y="20" width="24" height="24" fill="blue" filter="url(#t)"/></svg>"##,
    );
    // Multiple points inside the filter region should show the tiled blue.
    let p1 = image.pixel(24, 24);
    let p2 = image.pixel(40, 40);
    assert!(
        p1[2] > 180 && p1[3] > 180 && p2[2] > 180 && p2[3] > 180,
        "tile should repeat across filter region, got {p1:?} and {p2:?}"
    );
}

// ---- Regressions: review feedback (P2 fixes) -------------------------------

#[test]
fn no_op_primitive_with_subregion_still_clips() {
    // Review feedback (P2): the scene-level visibility gate used to ignore
    // `primitive.subregion`, so a parameter-wise no-op like `feOffset
    // dx="0" dy="0"` with an authored `x/y/width/height` would skip the
    // entire filter pipeline and leak the unclipped source. The subregion
    // is now part of `affects_output`, so the post-clip pass runs and
    // zeroes out pixels outside the rect.
    let Some(renderer) = skip_or_renderer("no_op_primitive_with_subregion_still_clips") else {
        return;
    };
    let image = render(
        &renderer,
        r##"<svg><filter id="f" x="0" y="0" width="64" height="64"><feOffset dx="0" dy="0" x="20" y="20" width="10" height="10"/></filter><rect width="64" height="64" fill="white" filter="url(#f)"/></svg>"##,
    );
    // Inside the subregion: source still visible.
    let inside = image.pixel(24, 24);
    assert!(
        inside[0] > 200 && inside[3] > 200,
        "subregion interior should keep the source, got {inside:?}"
    );
    // Outside the subregion: clipped away.
    assert_transparent(&image, 40, 40);
}

#[test]
fn blend_normal_with_transparent_dst_stays_premultiplied() {
    // Review feedback (P2): the original `fs_blend` body unpremultiplied
    // both inputs and then summed with `(1 - dst.a) * s.rgb + …`, which
    // produced *straight* RGB out of a premultiplied texture pipeline. For
    // a 50%-alpha red source over a transparent destination the leaked RGB
    // would be opaque red, so the next compositing pass painted a brighter
    // colour than the input. The reformulated shader keeps the linear
    // terms premultiplied; the centre pixel below shows the expected ~50%
    // pre-multiplied red (R ≈ 128) rather than the broken ~255.
    let Some(renderer) = skip_or_renderer("blend_normal_with_transparent_dst_stays_premultiplied")
    else {
        return;
    };
    // `feFlood` writes (0,0,0,0) — a transparent destination — and
    // `feBlend mode="normal"` composites the half-alpha source over it.
    let image = render(
        &renderer,
        r##"<svg><filter id="b"><feFlood flood-color="white" flood-opacity="0" result="empty"/><feBlend in="SourceGraphic" in2="empty" mode="normal"/></filter><rect x="20" y="20" width="24" height="24" fill="red" fill-opacity="0.5" filter="url(#b)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    // ~50% pre-multiplied red: alpha ~128, R well below 200 (the broken
    // straight value would put R ≥ 230 at the source's authored colour).
    assert!(
        centre[3] > 100 && centre[3] < 180,
        "blend output should keep ~50% alpha, got {centre:?}"
    );
    assert!(
        centre[0] < 200,
        "premultiplied blend output should not have full-strength RGB at half alpha, got {centre:?}"
    );
}

#[test]
fn tile_wraps_inside_input_primitive_subregion() {
    // Review feedback (P2): `feTile` used to hardcode the source rect to
    // the full texture `[0, 0, 1, 1]`, making it a passthrough when the
    // upstream primitive's actual output occupied a smaller subregion. The
    // renderer now resolves the input primitive's UV subregion and the
    // tile shader wraps inside that rect — so a small flood with its own
    // subregion gets repeated across the filter region rather than just
    // returning the original flood patch.
    let Some(renderer) = skip_or_renderer("tile_wraps_inside_input_primitive_subregion") else {
        return;
    };
    // The flood paints into a 16×16 subregion in the centre of a 64×64
    // canvas; `feTile` should replicate that patch across the whole filter
    // region. Sampling at canvas corners (well outside the original
    // 16×16 patch) must therefore see the tiled flood colour.
    let image = render(
        &renderer,
        r##"<svg><filter id="t" x="0" y="0" width="64" height="64"><feFlood flood-color="#11aa55" x="24" y="24" width="16" height="16" result="patch"/><feTile in="patch"/></filter><rect width="64" height="64" fill="white" filter="url(#t)"/></svg>"##,
    );
    // Pixel well outside the original flood patch — should be the tiled
    // green if the tile rect tracked the patch, transparent if the tile
    // shader was treating the (mostly empty) texture as its source.
    let corner = image.pixel(4, 4);
    assert!(
        corner[1] > 80 && corner[3] > 80,
        "feTile should repeat the upstream subregion across the filter region, got {corner:?}"
    );
}
