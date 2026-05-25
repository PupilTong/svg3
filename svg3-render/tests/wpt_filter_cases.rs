//! WPT-derived SVG filter cases for the subset svg3-render currently supports.
//!
//! Fixtures are reduced to implemented primitives and attributes while keeping
//! the WPT behavior under test.

use base64::engine::general_purpose;
use base64::Engine as _;
use svg3_dom::parse;
use svg3_render::{Image, RenderConfig, Renderer};

const CANVAS: u32 = 64;

fn renderer() -> Renderer {
    Renderer::headless().expect("WPT filter tests require a GPU adapter")
}

fn render(renderer: &Renderer, svg: &str) -> Image {
    let document = parse(svg).expect("WPT-derived filter fixture should parse");
    renderer
        .render_to_image(
            &document,
            RenderConfig {
                width: CANVAS,
                height: CANVAS,
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
fn wpt_filter_gaussian_blur_zero_axis_cases_pass() {
    // WPT `svg/import/filters-gauss-02-f-manual.svg`: a zero stdDeviation
    // component disables blur only on that axis.
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
fn wpt_filter_component_transfer_defaults_and_duplicate_func_cases_pass() {
    // WPT `svg/import/filters-color-02-b-manual.svg`: unspecified component
    // functions default to identity, and duplicated per-channel functions use
    // the last occurrence.
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
fn wpt_filter_fe_image_href_case_passes() {
    // WPT `svg/linking/reftests/href-feImage-element.html`: `<feImage>`
    // accepts `href` as well as the older `xlink:href`.
    let renderer = renderer();
    let href = png_data_uri(1, 1, &[255, 0, 0, 255]);
    let svg = format!(
        r##"<svg><filter id="tex"><feImage href="{href}" x="20" y="18" width="24" height="22"/></filter><rect width="64" height="64" filter="url(#tex)"/></svg>"##
    );
    let image = render(&renderer, &svg);

    assert_transparent(&image, 8, 8);
    let pixel = image.pixel(32, 28);
    assert!(
        pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40 && pixel[3] > 250,
        "feImage href should paint the embedded red PNG, got {pixel:?}"
    );
}

#[test]
fn wpt_filter_primitive_subregion_flood_case_passes() {
    // WPT `svg/import/filters-felem-02-f-manual.svg`: `x`/`y`/`width`/
    // `height` on a filter primitive should restrict that primitive's result
    // to a subregion.
    let renderer = renderer();
    let image = render(
        &renderer,
        r##"<svg><filter id="subregion"><feFlood flood-color="lime" x="16" y="16" width="32" height="32"/></filter><rect width="64" height="64" filter="url(#subregion)"/></svg>"##,
    );

    assert_transparent(&image, 8, 8);
    let centre = image.pixel(32, 32);
    assert!(
        centre[1] > 200 && centre[0] < 60 && centre[2] < 60 && centre[3] > 200,
        "flood subregion centre should be green, got {centre:?}"
    );
}
