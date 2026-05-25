//! WPT-derived SVG filter cases for the subset svg3-render currently supports.
//!
//! Fixtures are reduced to implemented primitives and attributes while keeping
//! the WPT behavior under test. WPT files whose core behavior still depends on
//! unsupported renderer features are represented as ignored tests so the gap is
//! visible in `cargo test -- --ignored`.

use base64::engine::general_purpose;
use base64::Engine as _;
use svg3_dom::parse;
use svg3_render::{Image, RenderConfig, Renderer};

const CANVAS: u32 = 64;

fn renderer() -> Renderer {
    Renderer::headless().expect("WPT filter tests require a GPU adapter")
}

fn render(renderer: &Renderer, svg: &str) -> Image {
    render_sized(renderer, svg, CANVAS, CANVAS)
}

fn render_sized(renderer: &Renderer, svg: &str, width: u32, height: u32) -> Image {
    let document = parse(svg).expect("WPT-derived filter fixture should parse");
    renderer
        .render_to_image(
            &document,
            RenderConfig {
                width,
                height,
                ..RenderConfig::default()
            },
        )
        .expect("WPT-derived filter fixture should render")
}

fn png_data_uri(width: u32, height: u32, rgba: &[u8]) -> String {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header should encode");
        writer
            .write_image_data(rgba)
            .expect("PNG pixels should encode");
    }
    format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(bytes)
    )
}

fn solid_png_data_uri(width: u32, height: u32, color: [u8; 4]) -> String {
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..width * height {
        rgba.extend_from_slice(&color);
    }
    png_data_uri(width, height, &rgba)
}

fn quadrant_png_data_uri() -> String {
    let mut rgba = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4 {
        for x in 0..4 {
            let color = match (x >= 2, y >= 2) {
                (false, false) => [255, 0, 0, 255],
                (true, false) => [0, 255, 0, 255],
                (false, true) => [0, 0, 255, 255],
                (true, true) => [255, 255, 0, 255],
            };
            rgba.extend_from_slice(&color);
        }
    }
    png_data_uri(4, 4, &rgba)
}

fn assert_transparent(image: &Image, x: u32, y: u32) {
    let pixel = image.pixel(x, y);
    assert!(
        pixel[3] <= 4,
        "pixel ({x}, {y}) should be transparent, got {pixel:?}"
    );
}

fn assert_visible(image: &Image, x: u32, y: u32) {
    let pixel = image.pixel(x, y);
    assert!(
        pixel[3] > 16,
        "pixel ({x}, {y}) should be visible, got {pixel:?}"
    );
}

fn assert_blue_halo(image: &Image, x: u32, y: u32) {
    let pixel = image.pixel(x, y);
    assert!(
        pixel[3] > 8 && pixel[2] > 8 && pixel[0] < pixel[2] && pixel[1] < pixel[2],
        "pixel ({x}, {y}) should be a blue blur halo, got {pixel:?}"
    );
}

fn assert_red(pixel: [u8; 4]) {
    assert!(
        pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80 && pixel[3] > 180,
        "pixel should be red, got {pixel:?}"
    );
}

fn assert_green(pixel: [u8; 4]) {
    assert!(
        pixel[1] > 180 && pixel[0] < 100 && pixel[2] < 100 && pixel[3] > 180,
        "pixel should be green, got {pixel:?}"
    );
}

fn assert_blue(pixel: [u8; 4]) {
    assert!(
        pixel[2] > 180 && pixel[0] < 100 && pixel[1] < 100 && pixel[3] > 180,
        "pixel should be blue, got {pixel:?}"
    );
}

fn assert_images_differ(left: &Image, right: &Image) {
    assert_eq!(left.width, right.width);
    assert_eq!(left.height, right.height);
    let changed_pixels = left
        .pixels
        .chunks_exact(4)
        .zip(right.pixels.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed_pixels > 32,
        "images should differ meaningfully, only {changed_pixels} pixels changed"
    );
}

macro_rules! unsupported_wpt {
    ($name:ident, $path:literal, $reason:literal) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            panic!("WPT `{}` is not supported yet: {}", $path, $reason);
        }
    };
}

#[test]
fn wpt_svg_import_filters_gauss_01_b_manual_passes() {
    let renderer = renderer();
    let uniform = render(
        &renderer,
        r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#soft)"/></svg>"##,
    );
    assert_blue_halo(&uniform, 20, 32);

    let anisotropic = render(
        &renderer,
        r##"<svg><filter id="flat"><feGaussianBlur stdDeviation="6 1"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#flat)"/></svg>"##,
    );
    assert_blue_halo(&anisotropic, 20, 32);
    assert_transparent(&anisotropic, 32, 16);
}

#[test]
fn wpt_svg_import_filters_gauss_02_f_manual_passes() {
    let renderer = renderer();
    let vertical = render(
        &renderer,
        r##"<svg><filter id="v"><feGaussianBlur stdDeviation="0 6"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#v)"/></svg>"##,
    );
    assert_blue_halo(&vertical, 32, 20);
    assert_transparent(&vertical, 20, 32);

    let horizontal = render(
        &renderer,
        r##"<svg><filter id="h"><feGaussianBlur stdDeviation="6 0"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#h)"/></svg>"##,
    );
    assert_blue_halo(&horizontal, 20, 32);
    assert_transparent(&horizontal, 32, 20);
}

#[test]
fn wpt_svg_import_filters_gauss_03_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="identity"><feGaussianBlur stdDeviation="0"/></filter><rect x="20" y="20" width="28" height="28" fill="lime" filter="url(#identity)"/></svg>"##,
    );
    assert_green(image.pixel(32, 32));
    assert_transparent(&image, 16, 32);
}

#[test]
fn wpt_svg_import_filters_color_01_b_manual_passes() {
    let renderer = renderer();
    let matrix = render(
        &renderer,
        r##"<svg><filter id="m"><feColorMatrix type="matrix" values="0 0 0 0 0  0 1 0 0 0  0 0 0 0 0  0 0 0 1 0"/></filter><rect x="16" y="16" width="32" height="32" fill="white" filter="url(#m)"/></svg>"##,
    );
    assert_green(matrix.pixel(32, 32));

    let saturate = render(
        &renderer,
        r##"<svg><filter id="s"><feColorMatrix type="saturate" values="0"/></filter><rect x="16" y="16" width="32" height="32" fill="red" filter="url(#s)"/></svg>"##,
    );
    let saturate_pixel = saturate.pixel(32, 32);
    assert!(
        saturate_pixel[0].abs_diff(saturate_pixel[1]) <= 4
            && saturate_pixel[1].abs_diff(saturate_pixel[2]) <= 4
            && saturate_pixel[3] > 180,
        "saturate(0) should produce neutral gray, got {saturate_pixel:?}"
    );

    let hue = render(
        &renderer,
        r##"<svg><filter id="h"><feColorMatrix type="hueRotate" values="120"/></filter><rect x="16" y="16" width="32" height="32" fill="red" filter="url(#h)"/></svg>"##,
    );
    let hue_pixel = hue.pixel(32, 32);
    assert!(
        hue_pixel[1] > hue_pixel[0] && hue_pixel[1] > hue_pixel[2],
        "hueRotate should move red toward green, got {hue_pixel:?}"
    );

    let luma = render(
        &renderer,
        r##"<svg><filter id="l"><feColorMatrix type="luminanceToAlpha"/></filter><rect x="16" y="16" width="32" height="32" fill="white" filter="url(#l)"/></svg>"##,
    );
    let luma_pixel = luma.pixel(32, 32);
    assert!(
        luma_pixel[0] < 30 && luma_pixel[1] < 30 && luma_pixel[2] < 30 && luma_pixel[3] > 200,
        "luminanceToAlpha should move luma into alpha and clear RGB, got {luma_pixel:?}"
    );
}

#[test]
fn wpt_svg_import_filters_color_02_b_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="ct"><feComponentTransfer><feFuncR type="identity"/><feFuncR type="linear" slope="0" intercept="1"/><feFuncR type="linear" slope="0" intercept="0"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="white" filter="url(#ct)"/></svg>"##,
    );
    let centre = image.pixel(32, 32);
    assert!(
        centre[0] < 30 && centre[1] > 200 && centre[2] > 200 && centre[3] > 200,
        "last feFuncR should zero red while G/B/A remain identity, got {centre:?}"
    );
}

#[test]
fn wpt_svg_import_filters_comptran_01_b_manual_passes() {
    let renderer = renderer();
    let table = render(
        &renderer,
        r##"<svg><filter id="t"><feComponentTransfer><feFuncR type="table" tableValues="0 0"/><feFuncG type="identity"/><feFuncB type="identity"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="white" filter="url(#t)"/></svg>"##,
    );
    let table_pixel = table.pixel(32, 32);
    assert!(
        table_pixel[0] < 30 && table_pixel[1] > 200 && table_pixel[2] > 200,
        "table transfer should clear red only, got {table_pixel:?}"
    );

    let discrete = render(
        &renderer,
        r##"<svg><filter id="d"><feComponentTransfer><feFuncG type="discrete" tableValues="0 1"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="#c0c0c0" filter="url(#d)"/></svg>"##,
    );
    let discrete_pixel = discrete.pixel(32, 32);
    assert!(
        discrete_pixel[1] > discrete_pixel[0] && discrete_pixel[1] > discrete_pixel[2],
        "discrete transfer should push mid-green to the high bucket, got {discrete_pixel:?}"
    );

    let linear = render(
        &renderer,
        r##"<svg><filter id="l"><feComponentTransfer><feFuncB type="linear" slope="0" intercept="1"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="black" filter="url(#l)"/></svg>"##,
    );
    assert_blue(linear.pixel(32, 32));

    let gamma = render(
        &renderer,
        r##"<svg><filter id="g"><feComponentTransfer><feFuncR type="gamma" amplitude="1" exponent="2" offset="0"/></feComponentTransfer></filter><rect x="16" y="16" width="32" height="32" fill="#808080" filter="url(#g)"/></svg>"##,
    );
    let gamma_pixel = gamma.pixel(32, 32);
    assert!(
        gamma_pixel[0] < 90 && gamma_pixel[1] > 90 && gamma_pixel[2] > 90,
        "gamma transfer should darken the red channel, got {gamma_pixel:?}"
    );
}

#[test]
fn wpt_svg_import_filters_conv_01_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="sharp"><feConvolveMatrix kernelMatrix="0 -1 0 -1 5 -1 0 -1 0"/></filter><rect x="16" y="16" width="32" height="32" fill="#c14b2b" filter="url(#sharp)"/></svg>"##,
    );
    assert_red(image.pixel(32, 32));
}

#[test]
fn wpt_svg_import_filters_conv_02_f_manual_passes() {
    let renderer = renderer();
    let default_order = render(
        &renderer,
        r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="0 0 0 0 1 0 0 0 0" preserveAlpha="true"/></filter><rect x="16" y="16" width="32" height="32" fill="red" filter="url(#c)"/></svg>"##,
    );
    assert_red(default_order.pixel(32, 32));

    let explicit_order = render(
        &renderer,
        r##"<svg><filter id="c"><feConvolveMatrix order="3 3" kernelMatrix="0 0 0 0 1 0 0 0 0" preserveAlpha="true"/></filter><rect x="16" y="16" width="32" height="32" fill="red" filter="url(#c)"/></svg>"##,
    );
    assert_red(explicit_order.pixel(32, 32));
}

#[test]
fn wpt_svg_import_filters_conv_03_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feFlood flood-color="blue"/><feConvolveMatrix in="SourceGraphic" kernelMatrix="0 0 0 0 1 0 0 0 0"/></filter><rect x="16" y="16" width="32" height="32" fill="red" filter="url(#c)"/></svg>"##,
    );
    assert_red(image.pixel(32, 32));
}

#[test]
fn wpt_svg_import_filters_conv_04_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="0 0 0 0 1 0 0 0 0" bias="0.5"/></filter><rect x="16" y="16" width="32" height="32" fill="black" filter="url(#c)"/></svg>"##,
    );
    let biased = image.pixel(32, 32);
    assert!(
        biased[0].abs_diff(biased[1]) <= 4
            && biased[1].abs_diff(biased[2]) <= 4
            && biased[0] > 120
            && biased[3] > 180,
        "positive convolution bias should lift black toward gray/white, got {biased:?}"
    );
}

#[test]
fn wpt_svg_import_filters_diffuse_01_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="light"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="white"><feDistantLight azimuth="45" elevation="60"/></feDiffuseLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#light)"/></svg>"##,
    );
    assert_visible(&image, 32, 32);
}

#[test]
fn wpt_svg_import_filters_displace_01_f_manual_passes() {
    let renderer = renderer();
    let map = quadrant_png_data_uri();
    let svg = format!(
        r##"<svg><filter id="warp"><feImage href="{map}" x="0" y="0" width="64" height="64" result="map"/><feDisplacementMap in="SourceGraphic" in2="map" scale="8" xChannelSelector="R" yChannelSelector="G"/></filter><rect x="20" y="20" width="24" height="24" fill="blue" filter="url(#warp)"/></svg>"##
    );
    let warped = render(&renderer, &svg);
    let identity = render(
        &renderer,
        r#"<svg><rect x="20" y="20" width="24" height="24" fill="blue"/></svg>"#,
    );
    assert_images_differ(&warped, &identity);
}

#[test]
fn wpt_svg_import_filters_displace_02_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="warp"><feDisplacementMap in="SourceGraphic" in2="SourceAlpha" scale="0"/></filter><rect x="16" y="16" width="32" height="32" fill="blue" filter="url(#warp)"/></svg>"##,
    );
    assert_blue(image.pixel(32, 32));
}

#[test]
fn wpt_svg_import_filters_image_01_b_manual_passes() {
    let renderer = renderer();
    let href = solid_png_data_uri(1, 1, [255, 0, 0, 255]);
    let svg = format!(
        r##"<svg><filter id="tex"><feImage href="{href}" x="20" y="18" width="24" height="22"/></filter><rect width="64" height="64" filter="url(#tex)"/></svg>"##
    );
    let image = render(&renderer, &svg);
    assert_transparent(&image, 8, 8);
    assert_red(image.pixel(32, 28));
}

#[test]
fn wpt_svg_import_filters_image_03_f_manual_passes() {
    let renderer = renderer();
    let href = solid_png_data_uri(1, 1, [0, 255, 0, 255]);
    let svg = format!(
        r##"<svg width="200" height="100"><filter id="tex"><feImage href="{href}" x="20%" y="25%" width="30%" height="40%"/></filter><rect width="200" height="100" filter="url(#tex)"/></svg>"##
    );
    let image = render_sized(&renderer, &svg, 200, 100);
    assert_transparent(&image, 20, 20);
    assert_green(image.pixel(80, 50));
}

#[test]
fn wpt_svg_import_filters_image_04_f_manual_passes() {
    let renderer = renderer();
    let href = solid_png_data_uri(10, 6, [0, 0, 255, 255]);
    let svg = format!(
        r##"<svg><filter id="tex"><feImage href="{href}" x="30" y="22" width="20" height="18"/></filter><rect width="64" height="64" filter="url(#tex)"/></svg>"##
    );
    let image = render(&renderer, &svg);
    assert_blue(image.pixel(40, 30));
    assert_transparent(&image, 56, 30);
}

#[test]
fn wpt_svg_import_filters_light_01_f_manual_passes() {
    let renderer = renderer();
    let distant = render(
        &renderer,
        r##"<svg><filter id="l"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><feDistantLight azimuth="45" elevation="60"/></feDiffuseLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#l)"/></svg>"##,
    );
    let point = render(
        &renderer,
        r##"<svg><filter id="l"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><fePointLight x="32" y="32" z="40"/></feDiffuseLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#l)"/></svg>"##,
    );
    let spot = render(
        &renderer,
        r##"<svg><filter id="l"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><feSpotLight x="32" y="32" z="40" pointsAtX="32" pointsAtY="32" pointsAtZ="0" limitingConeAngle="35"/></feDiffuseLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#l)"/></svg>"##,
    );
    assert_visible(&distant, 32, 32);
    assert_visible(&point, 32, 32);
    assert_visible(&spot, 32, 32);
}

#[test]
fn wpt_svg_import_filters_light_02_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="spec"><feSpecularLighting surfaceScale="5" specularConstant="1" specularExponent="8"><feDistantLight azimuth="135" elevation="45"/></feSpecularLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#spec)"/></svg>"##,
    );
    assert_visible(&image, 32, 32);
}

#[test]
fn wpt_svg_import_filters_light_03_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="spec"><feSpecularLighting surfaceScale="5" specularConstant="1" specularExponent="8"><fePointLight x="32" y="32" z="40"/></feSpecularLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#spec)"/></svg>"##,
    );
    assert_visible(&image, 32, 32);
}

#[test]
fn wpt_svg_import_filters_light_04_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="spot"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><feSpotLight x="32" y="32" z="40" pointsAtX="32" pointsAtY="32" pointsAtZ="0" limitingConeAngle="35"/></feDiffuseLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#spot)"/></svg>"##,
    );
    assert_visible(&image, 32, 32);
}

#[test]
fn wpt_svg_import_filters_morph_01_f_manual_passes() {
    let renderer = renderer();
    let dilate = render(
        &renderer,
        r##"<svg><filter id="grow"><feMorphology operator="dilate" radius="4"/></filter><rect x="28" y="28" width="8" height="8" fill="lime" filter="url(#grow)"/></svg>"##,
    );
    assert_green(dilate.pixel(24, 32));

    let erode = render(
        &renderer,
        r##"<svg><filter id="shrink"><feMorphology operator="erode" radius="4"/></filter><rect x="20" y="20" width="24" height="24" fill="lime" filter="url(#shrink)"/></svg>"##,
    );
    assert_transparent(&erode, 22, 32);
    assert_green(erode.pixel(32, 32));
}

#[test]
fn wpt_svg_import_filters_specular_01_f_manual_passes() {
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="spec"><feSpecularLighting surfaceScale="10" specularConstant="2" specularExponent="4" lighting-color="red"><feDistantLight azimuth="135" elevation="45"/></feSpecularLighting></filter><circle cx="32" cy="32" r="20" fill="white" filter="url(#spec)"/></svg>"##,
    );
    let pixel = image.pixel(32, 32);
    assert!(
        pixel[0] > pixel[1] && pixel[0] > pixel[2] && pixel[3] > 16,
        "red specular lighting should tint output red, got {pixel:?}"
    );
}

#[test]
fn wpt_svg_import_filters_turb_01_f_manual_passes() {
    let renderer = renderer();
    let turbulence = render(
        &renderer,
        r##"<svg><filter id="n"><feTurbulence type="turbulence" baseFrequency="0.08" numOctaves="2" seed="1"/></filter><rect x="8" y="8" width="48" height="48" filter="url(#n)"/></svg>"##,
    );
    let fractal = render(
        &renderer,
        r##"<svg><filter id="n"><feTurbulence type="fractalNoise" baseFrequency="0.08" numOctaves="2" seed="1"/></filter><rect x="8" y="8" width="48" height="48" filter="url(#n)"/></svg>"##,
    );
    assert_visible(&turbulence, 32, 32);
    assert_visible(&fractal, 32, 32);
    assert_images_differ(&turbulence, &fractal);
}

#[test]
fn wpt_svg_import_filters_turb_02_f_manual_passes() {
    let renderer = renderer();
    let seed_one = render(
        &renderer,
        r##"<svg><filter id="n"><feTurbulence seed="1" baseFrequency="0.08" type="turbulence"/></filter><rect x="8" y="8" width="48" height="48" filter="url(#n)"/></svg>"##,
    );
    let seed_two = render(
        &renderer,
        r##"<svg><filter id="n"><feTurbulence seed="2" baseFrequency="0.08" type="turbulence"/></filter><rect x="8" y="8" width="48" height="48" filter="url(#n)"/></svg>"##,
    );
    assert_images_differ(&seed_one, &seed_two);
}

#[test]
fn wpt_svg_linking_reftests_href_fe_image_element_passes() {
    let renderer = renderer();
    let href = solid_png_data_uri(1, 1, [255, 0, 0, 255]);
    let xlink = solid_png_data_uri(1, 1, [0, 255, 0, 255]);
    let svg = format!(
        r##"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><filter id="tex"><feImage href="{href}" xlink:href="{xlink}" x="20" y="18" width="24" height="22"/></filter><rect width="64" height="64" filter="url(#tex)"/></svg>"##
    );
    let image = render(&renderer, &svg);
    assert_red(image.pixel(32, 28));
}

unsupported_wpt!(
    wpt_svg_render_order_clip_path_filter_order,
    "svg/render/order/clip-path-filter-order.svg",
    "requires clip-path support before filter order can be validated"
);
unsupported_wpt!(
    wpt_svg_render_reftests_filter_effects_on_pattern,
    "svg/render/reftests/filter-effects-on-pattern.html",
    "requires SVG pattern paint servers"
);
unsupported_wpt!(
    wpt_svg_shapes_reftests_polygon_with_filtered_marker,
    "svg/shapes/reftests/polygon-with-filtered-marker.html",
    "requires filters on marker contents"
);
unsupported_wpt!(
    wpt_svg_svg_in_svg_circular_filter_reference_crash,
    "svg/svg-in-svg/svg-in-svg-circular-filter-reference-crash.html",
    "requires nested SVG-as-image and circular filter reference handling"
);
unsupported_wpt!(
    wpt_svg_extensibility_foreign_object_filter_repaint,
    "svg/extensibility/foreignObject/filter-repaint.html",
    "requires foreignObject rendering"
);
unsupported_wpt!(
    wpt_svg_extensibility_foreign_object_circular_filter_reference_crash,
    "svg/extensibility/foreignObject/foreign-object-circular-filter-reference-crash.html",
    "requires foreignObject rendering and circular filter reference handling"
);
unsupported_wpt!(
    wpt_svg_import_filters_background_01_f_manual,
    "svg/import/filters-background-01-f-manual.svg",
    "requires BackgroundImage/BackgroundAlpha, feOffset, and feMerge"
);
unsupported_wpt!(
    wpt_svg_import_filters_blend_01_b_manual,
    "svg/import/filters-blend-01-b-manual.svg",
    "requires feBlend"
);
unsupported_wpt!(
    wpt_svg_import_filters_composite_02_b_manual,
    "svg/import/filters-composite-02-b-manual.svg",
    "requires feComposite"
);
unsupported_wpt!(
    wpt_svg_import_filters_composite_03_f_manual,
    "svg/import/filters-composite-03-f-manual.svg",
    "requires feComposite"
);
unsupported_wpt!(
    wpt_svg_import_filters_composite_04_f_manual,
    "svg/import/filters-composite-04-f-manual.svg",
    "requires feComposite arithmetic"
);
unsupported_wpt!(
    wpt_svg_import_filters_composite_05_f_manual,
    "svg/import/filters-composite-05-f-manual.svg",
    "requires feComposite arithmetic"
);
unsupported_wpt!(
    wpt_svg_import_filters_conv_05_f_manual,
    "svg/import/filters-conv-05-f-manual.svg",
    "requires feConvolveMatrix edgeMode"
);
unsupported_wpt!(
    wpt_svg_import_filters_example_01_b_manual,
    "svg/import/filters-example-01-b-manual.svg",
    "requires feOffset and feComposite"
);
unsupported_wpt!(
    wpt_svg_import_filters_felem_01_b_manual,
    "svg/import/filters-felem-01-b-manual.svg",
    "requires empty and unresolved filters to produce transparent black"
);
unsupported_wpt!(
    wpt_svg_import_filters_felem_02_f_manual,
    "svg/import/filters-felem-02-f-manual.svg",
    "requires primitiveUnits and primitive subregion clipping for feFlood/feGaussianBlur/feOffset"
);
unsupported_wpt!(
    wpt_svg_import_filters_image_02_b_manual,
    "svg/import/filters-image-02-b-manual.svg",
    "requires SMIL animation of feImage href"
);
unsupported_wpt!(
    wpt_svg_import_filters_image_05_f_manual,
    "svg/import/filters-image-05-f-manual.svg",
    "requires preserveAspectRatio on feImage"
);
unsupported_wpt!(
    wpt_svg_import_filters_light_05_f_manual,
    "svg/import/filters-light-05-f-manual.svg",
    "requires currentColor inheritance and feMerge"
);
unsupported_wpt!(
    wpt_svg_import_filters_offset_01_b_manual,
    "svg/import/filters-offset-01-b-manual.svg",
    "requires feOffset and feMerge"
);
unsupported_wpt!(
    wpt_svg_import_filters_offset_02_b_manual,
    "svg/import/filters-offset-02-b-manual.svg",
    "requires feOffset"
);
unsupported_wpt!(
    wpt_svg_import_filters_overview_01_b_manual,
    "svg/import/filters-overview-01-b-manual.svg",
    "requires BackgroundImage/BackgroundAlpha, FillPaint, StrokePaint, and feMerge"
);
unsupported_wpt!(
    wpt_svg_import_filters_overview_02_b_manual,
    "svg/import/filters-overview-02-b-manual.svg",
    "requires gradients, BackgroundImage/BackgroundAlpha, FillPaint, StrokePaint, and feMerge"
);
unsupported_wpt!(
    wpt_svg_import_filters_overview_03_b_manual,
    "svg/import/filters-overview-03-b-manual.svg",
    "requires gradients, BackgroundImage/BackgroundAlpha, FillPaint, StrokePaint, and feMerge"
);
unsupported_wpt!(
    wpt_svg_import_filters_tile_01_b_manual,
    "svg/import/filters-tile-01-b-manual.svg",
    "requires feTile and feOffset"
);
unsupported_wpt!(
    wpt_svg_import_masking_filter_01_f_manual,
    "svg/import/masking-filter-01-f-manual.svg",
    "requires SVG mask support"
);
unsupported_wpt!(
    wpt_svg_linking_reftests_href_filter_element,
    "svg/linking/reftests/href-filter-element.html",
    "requires href inheritance on filter elements plus feOffset and feMerge"
);
unsupported_wpt!(
    wpt_svg_styling_filter_render_frame_cases,
    "svg/styling/svg-filter-render-*.html",
    "requires browser frame/plugin security rendering behavior"
);
