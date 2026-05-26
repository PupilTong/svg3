//! WPT-derived SVG shape cases for the subset svg3 currently supports.
//!
//! Each fixture below is reduced to implemented elements and attributes while
//! preserving the WPT behavior under test. Known failures are kept as ignored
//! tests so they are dumped in-tree and can be run explicitly without keeping
//! the default test suite red.

use svg3::dom::parse;
use svg3::render::{build_scene, document_viewport, Mesh, Viewport};

const FALLBACK_VIEWPORT: Viewport = Viewport {
    width: 480.0,
    height: 360.0,
};

fn renderable_mesh(svg: &str) -> Mesh {
    let document = parse(svg).expect("WPT-derived fixture should parse");
    let viewport = document_viewport(&document, FALLBACK_VIEWPORT);
    build_scene(&document, viewport)
}

fn assert_all_vertices_are(mesh: &Mesh, color: [f32; 4]) {
    assert!(
        mesh.vertices.iter().all(|vertex| vertex.color == color),
        "unexpected vertex colours: {:?}",
        mesh.vertices
            .iter()
            .map(|vertex| vertex.color)
            .collect::<Vec<_>>()
    );
}

fn assert_vertex_bounds(mesh: &Mesh, expected: [f32; 4]) {
    let actual = mesh.vertices.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |[min_x, min_y, max_x, max_y], vertex| {
            [
                min_x.min(vertex.position[0]),
                min_y.min(vertex.position[1]),
                max_x.max(vertex.position[0]),
                max_y.max(vertex.position[1]),
            ]
        },
    );
    for (actual_component, expected_component) in actual.into_iter().zip(expected) {
        assert!(
            (actual_component - expected_component).abs() < 1e-4,
            "expected bounds {expected:?}, got {actual:?}"
        );
    }
}

#[test]
fn wpt_rect_fill_and_degenerate_cases_pass() {
    // WPT `svg/shapes/rect-01.svg`: a basic filled rect renders.
    let mesh =
        renderable_mesh(r#"<svg><rect x="10" y="10" width="50" height="50" fill="blue"/></svg>"#);
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_all_vertices_are(&mesh, [0.0, 0.0, 1.0, 1.0]);

    // WPT `svg/shapes/rect-03.svg`: rounded filled rects render.
    let mesh = renderable_mesh(
        r#"<svg><rect x="10" y="10" width="50" height="50" rx="8" ry="8" fill="blue"/></svg>"#,
    );
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_all_vertices_are(&mesh, [0.0, 0.0, 1.0, 1.0]);

    // WPT `svg/shapes/rect-05.svg`: zero width/height disables rendering.
    let mesh = renderable_mesh(
        r#"<svg width="100" height="100">
  <rect x="30" y="50" width="0" height="10" fill="red" stroke="red" stroke-width="4"/>
  <rect x="50" y="50" width="10" height="0" fill="red" stroke="red" stroke-width="4"/>
  <rect x="70" y="50" width="0" height="0" fill="red" stroke="red" stroke-width="4"/>
</svg>"#,
    );
    assert!(mesh.is_empty());
}

#[test]
fn wpt_rect_radius_auto_and_clamp_cases_pass() {
    // WPT `svg/import/shapes-rect-04-f-manual.svg`: unspecified `rx`/`ry`
    // mirrors the other radius, while omitting both leaves square corners.
    let mesh = renderable_mesh(
        r#"<svg width="480" height="360">
  <rect x="25" y="25" width="200" height="100" rx="50" fill="black"/>
  <rect x="275" y="25" width="200" height="100" ry="50" fill="black"/>
  <rect x="150" y="135" width="200" height="100" fill="black"/>
</svg>"#,
    );
    assert_eq!(mesh.vertices.len(), 12);
    assert_eq!(mesh.vertices[0].params, [100.0, 50.0, 50.0, 50.0]);
    assert_eq!(mesh.vertices[4].params, [100.0, 50.0, 50.0, 50.0]);
    assert_eq!(mesh.vertices[8].params, [0.0, 0.0, 0.0, 0.0]);

    // WPT `svg/import/shapes-rect-06-f-manual.svg`: radii larger than half
    // the side are clamped to half width/height.
    let mesh = renderable_mesh(
        r#"<svg><rect x="25" y="50" width="200" height="100" rx="150" ry="75" fill="black"/></svg>"#,
    );
    assert_eq!(mesh.vertices[0].params, [100.0, 50.0, 100.0, 50.0]);

    // WPT `svg/import/shapes-rect-07-f-manual.svg`: `rx`/`ry` copying occurs
    // before clamping, so `rx="100"` also yields `ry=50` after clamp.
    let mesh = renderable_mesh(
        r#"<svg><rect x="25" y="50" width="200" height="100" rx="100" fill="black"/></svg>"#,
    );
    assert_eq!(mesh.vertices[0].params, [100.0, 50.0, 100.0, 50.0]);
}

#[test]
fn wpt_circle_and_ellipse_fill_cases_pass() {
    // WPT `svg/import/shapes-circle-02-t-manual.svg`: `cx`/`cy` default to 0.
    let mesh = renderable_mesh(r#"<svg><circle r="50" fill="blue"/></svg>"#);
    assert_eq!(mesh.vertices.len(), 4);
    assert_all_vertices_are(&mesh, [0.0, 0.0, 1.0, 1.0]);

    // WPT `svg/shapes/circle-01.svg`: `r=0` disables rendering.
    let mesh = renderable_mesh(
        r#"<svg width="100" height="100"><circle cx="50" cy="50" r="0" fill="red" stroke="red" stroke-width="5"/></svg>"#,
    );
    assert!(mesh.is_empty());

    // WPT `svg/import/shapes-ellipse-02-t-manual.svg`: `cx`/`cy` default to
    // 0 when a fillable ellipse has explicit radii.
    let mesh = renderable_mesh(r#"<svg><ellipse rx="100" ry="50" fill="blue"/></svg>"#);
    assert_eq!(mesh.vertices.len(), 4);
    assert_all_vertices_are(&mesh, [0.0, 0.0, 1.0, 1.0]);

    // WPT `svg/shapes/ellipse-09.svg`: zero `rx` or `ry` disables rendering.
    let mesh = renderable_mesh(
        r#"<svg width="100" height="100">
  <ellipse cx="30" cy="50" rx="0" ry="10" fill="red" stroke="red" stroke-width="5"/>
  <ellipse cx="50" cy="50" rx="10" ry="0" fill="red" stroke="red" stroke-width="5"/>
  <ellipse cx="70" cy="50" rx="0" ry="0" fill="red" stroke="red" stroke-width="5"/>
</svg>"#,
    );
    assert!(mesh.is_empty());
}

#[test]
fn wpt_line_fill_has_no_effect_case_passes() {
    // WPT `svg/import/shapes-line-02-f-manual.svg`: `fill` has no effect on
    // `<line>`, so only the stroke paint contributes geometry.
    let mesh = renderable_mesh(
        r#"<svg><line x1="100" y1="100" x2="300" y2="100" stroke-width="10" stroke="blue" fill="red"/></svg>"#,
    );
    assert_eq!(mesh.vertices.len(), 4);
    assert_all_vertices_are(&mesh, [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn wpt_polygon_polyline_and_path_fill_cases_pass() {
    // WPT `svg/import/shapes-polygon-03-t-manual.svg`: an odd trailing
    // polygon coordinate is dropped after valid coordinate pairs.
    let mesh =
        renderable_mesh(r#"<svg><polygon fill="lime" points="80,60 80,160 150,110 80"/></svg>"#);
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.indices.len(), 3);
    assert_all_vertices_are(&mesh, [0.0, 1.0, 0.0, 1.0]);

    // WPT `svg/import/shapes-polyline-02-t-manual.svg`: a filled polyline is
    // rendered by closing the open point list for fill.
    let mesh = renderable_mesh(
        r#"<svg><polyline fill="lime" points="189,185 228,203 238,245 212,279 169,280 141,247 149,205"/></svg>"#,
    );
    assert!(!mesh.is_empty());
    assert_all_vertices_are(&mesh, [0.0, 1.0, 0.0, 1.0]);

    // WPT `svg/import/paths-data-16-t-manual.svg`: extra coordinate pairs
    // after `M` are implicit `L` commands.
    let mesh = renderable_mesh(r#"<svg><path d="M100,120 160,220 40,220 z" fill="blue"/></svg>"#);
    assert!(!mesh.is_empty());
    assert_all_vertices_are(&mesh, [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn wpt_rect_stroke_case_passes() {
    // WPT `svg/shapes/rect-02.svg`: a non-rounded `fill="none"` rect with a
    // visible stroke should render the stroke outline.
    let mesh = renderable_mesh(
        r#"<svg><rect x="10" y="10" width="50" height="50" fill="none" stroke="blue" stroke-width="4"/></svg>"#,
    );
    assert!(
        !mesh.is_empty(),
        "rect stroke geometry should be emitted even when fill is none"
    );
    assert_all_vertices_are(&mesh, [0.0, 0.0, 1.0, 1.0]);
    assert_vertex_bounds(&mesh, [8.0, 8.0, 62.0, 62.0]);
}

#[test]
fn wpt_circle_stroke_case_passes() {
    // WPT `svg/import/shapes-circle-01-t-manual.svg`: `fill="none"` circles
    // with visible stroke should render the stroke outline.
    let mesh = renderable_mesh(
        r#"<svg><circle cx="100" cy="100" r="50" fill="none" stroke="black"/></svg>"#,
    );
    assert!(
        !mesh.is_empty(),
        "circle stroke geometry should be emitted even when fill is none"
    );
}

#[test]
fn wpt_ellipse_stroke_case_passes() {
    // WPT `svg/import/shapes-ellipse-01-t-manual.svg`: `fill="none"`
    // ellipses with visible stroke should render the stroke outline.
    let mesh = renderable_mesh(
        r#"<svg><ellipse cx="50" cy="75" rx="30" ry="50" fill="none" stroke="black"/></svg>"#,
    );
    assert!(
        !mesh.is_empty(),
        "ellipse stroke geometry should be emitted even when fill is none"
    );
}

#[test]
fn wpt_polygon_stroke_case_passes() {
    // WPT `svg/import/shapes-polygon-01-t-manual.svg`: `fill="none"`
    // polygons with visible stroke should render the stroke outline.
    let mesh = renderable_mesh(
        r#"<svg><polygon fill="none" stroke="blue" stroke-width="8" points="59,185 98,203 108,245 82,279 39,280 11,247 19,205 59,185"/></svg>"#,
    );
    assert!(
        !mesh.is_empty(),
        "polygon stroke geometry should be emitted even when fill is none"
    );
}

#[test]
fn wpt_polyline_stroke_case_passes() {
    // WPT `svg/import/shapes-polyline-01-t-manual.svg`: `fill="none"`
    // polylines with visible stroke should render the stroke outline.
    let mesh = renderable_mesh(
        r#"<svg><polyline fill="none" stroke="blue" stroke-width="8" points="220,50 267,84 249,140 190,140 172,84 220,50"/></svg>"#,
    );
    assert!(
        !mesh.is_empty(),
        "polyline stroke geometry should be emitted even when fill is none"
    );
}

#[test]
fn wpt_polyline_odd_trailing_coordinate_passes() {
    // WPT `svg/import/shapes-polygon-03-t-manual.svg`: a trailing unpaired
    // polyline coordinate should be ignored after the valid coordinate pairs.
    let mesh = renderable_mesh(
        r#"<svg><polyline fill="lime" points="180,200 180,300 250,250 180,200 250"/></svg>"#,
    );
    assert!(
        !mesh.is_empty(),
        "polyline should render using valid pairs before the trailing coordinate"
    );
}
