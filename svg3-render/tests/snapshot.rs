//! End-to-end snapshot tests for basic-shape rendering.
//!
//! Each case parses an svg3 document — the SVG WPT `shapes/rect-*`,
//! `shapes/circle-*`, `shapes/ellipse-*`, and `shapes/polygon-*` reference
//! tests, plus polyline fill, stroked shapes, line, path, markers, Gaussian
//! blur and image filters, mixed-shape, and canonical SVG samples — renders
//! it headlessly with
//! [`Renderer::render_to_image`], and compares the result against a committed
//! golden PNG in `tests/snapshots/`. Those
//! PNGs are the reviewable snapshots — open them in a pull request to see
//! what the renderer produces.
//!
//! After an intentional rendering change, regenerate the goldens and review
//! the updated images in the diff:
//!
//! ```sh
//! SVG3_UPDATE_SNAPSHOTS=1 cargo test -p svg3-render --test snapshot
//! ```
//!
//! Rendering needs a GPU adapter, so the suite self-skips where none is
//! available (it runs on macOS/Metal; it is skipped on a GPU-less host).

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose;
use base64::Engine as _;
use svg3_dom::parse;
use svg3_render::{Camera, Image, RenderConfig, RenderError, Renderer};

/// Side length of the default square render target, in pixels.
const CANVAS: u32 = 100;

/// Per-channel `RGBA8` tolerance when comparing against a golden, absorbing
/// sRGB-encode rounding differences between GPU drivers.
const CHANNEL_TOLERANCE: u8 = 4;

/// A named snapshot case: an svg3 document rendered into a `width`×`height`
/// target and compared against `tests/snapshots/<name>.png`.
struct Case {
    /// Golden file stem — `tests/snapshots/<name>.png`.
    name: &'static str,
    /// The svg3 document to render.
    svg: Cow<'static, str>,
    /// Render-target width, in pixels.
    width: u32,
    /// Render-target height, in pixels.
    height: u32,
    /// Optional 3D camera; `None` renders 2D content flat.
    camera: Option<Camera>,
}

impl Case {
    /// A case rendered into a square `CANVAS`×`CANVAS` target.
    fn square(name: &'static str, svg: impl Into<Cow<'static, str>>) -> Self {
        Self {
            name,
            svg: svg.into(),
            width: CANVAS,
            height: CANVAS,
            camera: None,
        }
    }

    /// A case rendered into a `width`×`height` target.
    fn sized(
        name: &'static str,
        svg: impl Into<Cow<'static, str>>,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            name,
            svg: svg.into(),
            width,
            height,
            camera: None,
        }
    }

    /// The same case viewed through `camera` instead of drawn flat.
    fn with_camera(mut self, camera: Camera) -> Self {
        self.camera = Some(camera);
        self
    }
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
                (false, false) => [242, 193, 78, 255],
                (true, false) => [37, 99, 235, 255],
                (false, true) => [17, 170, 85, 255],
                (true, true) => [193, 75, 43, 255],
            };
            rgba.extend_from_slice(&color);
        }
    }
    png_data_uri(4, 4, &rgba)
}

fn alpha_cross_png_data_uri() -> String {
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16 {
        for x in 0..16 {
            let on_cross = (6..10).contains(&x) || (6..10).contains(&y);
            let alpha = if on_cross { 255 } else { 96 };
            rgba.extend_from_slice(&[255, 255, 255, alpha]);
        }
    }
    png_data_uri(16, 16, &rgba)
}

/// All snapshot cases: the 2D shape references, then the 3D camera views.
fn cases() -> Vec<Case> {
    // A deliberately asymmetric scene — a yellow disc near the top-left and
    // a red panel near the bottom-right over a navy ground — so a camera
    // view's orientation (upright? mirrored?) is unambiguous.
    const CAMERA_SCENE: &str = r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><circle cx="30" cy="28" r="18" fill="#f2c14e"/><rect x="52" y="56" width="36" height="32" fill="#c14b2b"/></svg>"##;

    // Straight-on reference view, framing the document head-on.
    let front = Camera::facing(100, 100);
    // Pan: slide the eye and its target together along +X — the scene
    // translates across the frame without foreshortening.
    let mut pan = Camera::facing(100, 100);
    pan.eye.x += 24.0;
    pan.target.x += 24.0;
    // Angled: move the eye to one side, still aimed at the document centre —
    // an oblique view with visible perspective foreshortening.
    let mut angled = Camera::facing(100, 100);
    angled.eye.x += 60.0;
    // Dolly: push the eye toward the scene along -Z — a perspective zoom-in.
    let mut dolly = Camera::facing(100, 100);
    dolly.eye.z *= 0.6;

    let quadrants = quadrant_png_data_uri();
    let alpha_cross = alpha_cross_png_data_uri();
    let red = solid_png_data_uri(1, 1, [193, 75, 43, 255]);
    let green = solid_png_data_uri(1, 1, [17, 170, 85, 255]);
    let blue = solid_png_data_uri(1, 1, [37, 99, 235, 255]);
    let yellow = solid_png_data_uri(20, 14, [242, 193, 78, 255]);
    let orange = solid_png_data_uri(1, 1, [255, 165, 0, 255]);

    vec![
        // WPT `shapes/rect-01`: a basic filled rectangle.
        Case::square(
            "rect-fill",
            r#"<svg><rect x="10" y="10" width="80" height="80" fill="blue"/></svg>"#,
        ),
        // A rectangle with no `fill` — SVG 1.1's initial value is opaque black.
        Case::square(
            "rect-default-fill",
            r#"<svg><rect x="20" y="20" width="60" height="50"/></svg>"#,
        ),
        // WPT `shapes/rect-03`: rounded corners via `rx`/`ry`.
        Case::square(
            "rect-rounded",
            r#"<svg><rect x="10" y="10" width="80" height="80" rx="16" ry="16" fill="blue"/></svg>"#,
        ),
        // WPT `import/shapes-rect-06`: `rx`/`ry` over half the side are clamped —
        // here to a fully-rounded "stadium".
        Case::square(
            "rect-rounded-clamped",
            r#"<svg><rect x="15" y="30" width="70" height="40" rx="80" ry="80" fill="blue"/></svg>"#,
        ),
        // Basic-shape stroke: fill paints first, then a dashed rounded-rect
        // stroke is tessellated through the same mesh path.
        Case::square(
            "rect-rounded-dashed-stroke",
            r##"<svg><rect x="14" y="16" width="72" height="68" rx="14" ry="22" fill="#f2c14e" stroke="#13294b" stroke-width="8" stroke-dasharray="12 7" stroke-linejoin="round"/></svg>"##,
        ),
        // WPT `shapes/rect-05`: a zero-width rectangle is not rendered.
        Case::square(
            "rect-zero-size",
            r#"<svg><rect x="30" y="30" width="0" height="40" fill="blue"/></svg>"#,
        ),
        // Painter's order: a later `<rect>` paints over an earlier one.
        Case::square(
            "rect-overlap",
            r#"<svg><rect x="10" y="10" width="55" height="55" fill="blue"/><rect x="40" y="40" width="50" height="50" fill="red"/></svg>"#,
        ),
        // A hexadecimal `fill` colour.
        Case::square(
            "rect-hex-fill",
            r##"<svg><rect x="18" y="18" width="64" height="64" fill="#11aa55"/></svg>"##,
        ),
        // WPT `shapes/circle-*`: a basic filled circle.
        Case::square(
            "circle-fill",
            r#"<svg><circle cx="50" cy="50" r="40" fill="blue"/></svg>"#,
        ),
        // A circle with no `fill` — SVG 1.1's initial value is opaque black.
        Case::square(
            "circle-default-fill",
            r#"<svg><circle cx="50" cy="50" r="35"/></svg>"#,
        ),
        // [SVG11] §9.3: a zero-radius circle is not rendered.
        Case::square(
            "circle-zero-radius",
            r#"<svg><circle cx="50" cy="50" r="0" fill="blue"/></svg>"#,
        ),
        // Painter's order: a later `<circle>` paints over an earlier one.
        Case::square(
            "circle-overlap",
            r#"<svg><circle cx="38" cy="38" r="32" fill="blue"/><circle cx="62" cy="62" r="32" fill="red"/></svg>"#,
        ),
        // A hexadecimal `fill` colour.
        Case::square(
            "circle-hex-fill",
            r##"<svg><circle cx="50" cy="50" r="38" fill="#11aa55"/></svg>"##,
        ),
        // Circular stroke dashes are calibrated by `pathLength`, so the dash
        // pattern follows the authored path-length scale rather than raw user
        // units.
        Case::square(
            "circle-dashed-pathlength-stroke",
            r##"<svg><circle cx="50" cy="50" r="34" fill="none" stroke="#2563eb" stroke-width="9" stroke-dasharray="10 7" stroke-dashoffset="4" pathLength="100"/></svg>"##,
        ),
        // SVG WPT `shapes/ellipse-*`: a basic filled ellipse, wider than it
        // is tall so it is visibly not a circle.
        Case::square(
            "ellipse-fill",
            r#"<svg><ellipse cx="50" cy="50" rx="45" ry="28" fill="blue"/></svg>"#,
        ),
        // An ellipse with no `fill` — SVG 1.1's initial value is opaque black.
        Case::square(
            "ellipse-default-fill",
            r#"<svg><ellipse cx="50" cy="50" rx="42" ry="30"/></svg>"#,
        ),
        // [SVG11] §9.4: an ellipse with a zero radius is not rendered.
        Case::square(
            "ellipse-zero-radius",
            r#"<svg><ellipse cx="50" cy="50" rx="0" ry="30" fill="blue"/></svg>"#,
        ),
        // Painter's order: a later `<ellipse>` paints over an earlier one.
        Case::square(
            "ellipse-overlap",
            r#"<svg><ellipse cx="40" cy="44" rx="38" ry="24" fill="blue"/><ellipse cx="62" cy="58" rx="30" ry="40" fill="red"/></svg>"#,
        ),
        // Stroke opacity and global opacity multiply the stroke paint before
        // compositing over the filled ground.
        Case::square(
            "ellipse-stroke-opacity",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><ellipse cx="50" cy="50" rx="38" ry="24" fill="none" stroke="#f2c14e" stroke-width="12" stroke-opacity="0.7" opacity="0.8"/></svg>"##,
        ),
        // WPT `shapes/polygon-*`: a basic filled (convex) triangle.
        Case::square(
            "polygon-triangle",
            r#"<svg><polygon points="50,10 90,85 10,85" fill="blue"/></svg>"#,
        ),
        // A concave star: its inner vertices are reflex, so a centre-pivoted
        // fan would spill outside the outline — this exercises ear clipping.
        Case::square(
            "polygon-star",
            r##"<svg><polygon points="50,5 61,35 93,36 67,56 76,86 50,68 24,86 33,56 7,36 39,35" fill="#11aa55"/></svg>"##,
        ),
        // Polygon strokes exercise closed-path joins independently from fill
        // triangulation.
        Case::square(
            "polygon-round-join-stroke",
            r##"<svg><polygon points="50,8 88,84 12,84" fill="#f2c14e" stroke="#c14b2b" stroke-width="12" stroke-linejoin="round"/></svg>"##,
        ),
        // SVG WPT `shapes/polyline-*`: an open point list is closed for fill
        // rendering, producing a filled triangle.
        Case::square(
            "polyline-triangle-fill",
            r#"<svg><polyline points="16,82 50,18 84,82" fill="blue"/></svg>"#,
        ),
        // A reversed point order exercises the tessellator's winding branch
        // and should still produce a filled triangle.
        Case::square(
            "polyline-clockwise-fill",
            r#"<svg><polyline points="16,82 84,82 50,18" fill="blue"/></svg>"#,
        ),
        // A repeated first point is an explicitly closed list; normalisation
        // drops the duplicate endpoint before triangulation.
        Case::square(
            "polyline-explicit-close",
            r##"<svg><polyline points="20,20 80,26 72,80 28,72 20,20" fill="#11aa55"/></svg>"##,
        ),
        // Concave simple polygon: the L shape exercises ear-clipping through
        // the public parse -> render path, not just unit-level triangulation.
        Case::square(
            "polyline-concave-fill",
            r#"<svg><polyline points="14,14 82,14 82,36 42,36 42,82 14,82" fill="blue"/></svg>"#,
        ),
        // Painter's order with a polyline: a later rect paints over the
        // triangle in the combined mesh.
        Case::square(
            "polyline-overlap-order",
            r#"<svg><polyline points="10,84 50,10 90,84" fill="blue"/><rect x="35" y="45" width="30" height="30" fill="red"/></svg>"#,
        ),
        // Open polylines can now be stroked without requiring fill geometry,
        // and dashes keep round caps and bevel joins through tessellation.
        Case::square(
            "polyline-dashed-roundcap-stroke",
            r##"<svg><polyline points="10,78 32,24 56,76 78,24 90,78" fill="none" stroke="#11aa55" stroke-width="8" stroke-linecap="round" stroke-linejoin="bevel" stroke-dasharray="14 9"/></svg>"##,
        ),
        // Invalid SVG 1.1 point coordinates are skipped, so the red polyline
        // with a percentage coordinate does not cover the gray background.
        Case::square(
            "polyline-invalid-points-skipped",
            r##"<svg><rect width="100%" height="100%" fill="#666666"/><polyline points="18,82 50%,18 82,82" fill="red"/></svg>"##,
        ),
        // WPT `shapes/line-*`: a stroked horizontal line. SVG's initial
        // stroke is `none`, so this fixture declares stroke paint explicitly.
        Case::square(
            "line-stroke",
            r##"<svg><line x1="10" y1="50" x2="90" y2="50" stroke="#11aa55" stroke-width="8"/></svg>"##,
        ),
        // A non-axis-aligned line, catching perpendicular-offset math on both
        // axes rather than only the horizontal-line path above.
        Case::square(
            "line-diagonal",
            r##"<svg><line x1="15.25" y1="82.5" x2="84.75" y2="18.25" stroke="#2563eb" stroke-width="9.5"/></svg>"##,
        ),
        // Percentage coordinates and percentage stroke width, resolved through
        // the document viewport before tessellation.
        Case::sized(
            "line-percentages",
            r##"<svg width="160" height="90"><line x1="10%" y1="80%" x2="90%" y2="20%" stroke="#c14b2b" stroke-width="8%"/></svg>"##,
            160,
            90,
        ),
        // SVG WPT `paths/path-*`: closed path fill, including cubic and
        // quadratic curves flattened through the path tessellator.
        Case::square(
            "path-fill-curves",
            r##"<svg><path d="M 18 76 C 18 20 82 20 82 76 Q 50 92 18 76 Z" fill="#2563eb"/></svg>"##,
        ),
        // Stroked open path with round caps and joins.
        Case::square(
            "path-stroke-curve",
            r##"<svg><path d="M 14 76 C 24 18 76 18 86 76" fill="none" stroke="#11aa55" stroke-width="10" stroke-linecap="round"/></svg>"##,
        ),
        // `fill-rule="evenodd"` cuts a hole through same-winding subpaths.
        Case::square(
            "path-evenodd-hole",
            r##"<svg><path fill="#13294b" fill-rule="evenodd" d="M 10 10 H 90 V 90 H 10 Z M 30 30 H 70 V 70 H 30 Z"/></svg>"##,
        ),
        // Referenced markers are collected from `<defs>` and instanced at the
        // start, middle, and end placements after the stroked polyline.
        Case::square(
            "polyline-markers",
            r##"<svg><defs><marker id="dot" markerUnits="userSpaceOnUse" markerWidth="8" markerHeight="8" refX="4" refY="4"><circle cx="4" cy="4" r="4" fill="#f2c14e"/></marker><marker id="arrow" markerUnits="userSpaceOnUse" markerWidth="12" markerHeight="10" refX="10" refY="5" orient="auto"><path d="M0 0 L12 5 L0 10 Z" fill="#c14b2b"/></marker></defs><polyline points="14,72 38,28 64,72 86,28" fill="none" stroke="#2563eb" stroke-width="5" marker-start="url(#dot)" marker-mid="url(#dot)" marker-end="url(#arrow)"/></svg>"##,
        ),
        // A closed shape's end marker belongs at the initial vertex, not the
        // last authored point before the implicit close segment.
        Case::square(
            "polygon-closed-marker-end",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><defs><marker id="dot" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="10" refX="5" refY="5" orient="0"><circle cx="5" cy="5" r="5" fill="#f2c14e"/></marker></defs><polygon points="24,24 78,26 76,78" fill="none" stroke="#2563eb" stroke-width="5" marker-end="url(#dot)"/></svg>"##,
        ),
        // Mixed basic shapes in painter's order: the line is composited over
        // filled rect/circle/ellipse geometry in one end-to-end scene.
        Case::square(
            "basic-shapes-overlap",
            r##"<svg><rect x="8" y="8" width="84" height="84" fill="#13294b"/><circle cx="36" cy="36" r="22" fill="#f2c14e"/><ellipse cx="62" cy="62" rx="28" ry="18" fill="#c14b2b"/><line x1="18" y1="80" x2="82" y2="20" stroke="#ffffff" stroke-width="6"/></svg>"##,
        ),
        // SVG filters: referenced `<filter>` definitions with
        // `<feGaussianBlur>` render their target subtree into an offscreen GPU
        // texture, blur it, and composite it back in painter order.
        Case::square(
            "filter-blur-rect",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="30" y="30" width="40" height="40" fill="#f2c14e" filter="url(#soft)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-circle",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="5"/></filter><circle cx="50" cy="50" r="24" fill="#11aa55" filter="url(#soft)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-ellipse",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="3"/></filter><ellipse cx="50" cy="50" rx="34" ry="18" fill="#c14b2b" filter="url(#soft)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-path",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="3"/></filter><path d="M 22 74 C 22 22 78 22 78 74 Q 50 90 22 74 Z" fill="#2563eb" filter="url(#soft)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-line",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="3"/></filter><line x1="18" y1="78" x2="82" y2="22" stroke="#ffffff" stroke-width="8" filter="url(#soft)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-horizontal",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="wide"><feGaussianBlur stdDeviation="8 0"/></filter><rect x="38" y="30" width="24" height="40" fill="#f2c14e" filter="url(#wide)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-vertical",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tall"><feGaussianBlur stdDeviation="0 8"/></filter><rect x="30" y="38" width="40" height="24" fill="#f2c14e" filter="url(#tall)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-two-axis",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="wide-soft"><feGaussianBlur stdDeviation="7 2"/></filter><circle cx="50" cy="50" r="19" fill="#c14b2b" filter="url(#wide-soft)"/></svg>"##,
        ),
        Case::square(
            "filter-blur-group",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><g filter="url(#soft)"><rect x="24" y="24" width="26" height="46" fill="#c14b2b"/><circle cx="62" cy="42" r="18" fill="#2563eb"/></g></svg>"##,
        ),
        Case::square(
            "filter-blur-painter-order",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="5"/></filter><circle cx="44" cy="50" r="26" fill="#c14b2b" filter="url(#soft)"/><rect x="48" y="26" width="30" height="48" fill="#11aa55"/></svg>"##,
        ),
        Case::square(
            "filter-blur-quoted-url",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="28" y="28" width="44" height="44" fill="#2563eb" filter='url("#soft")'/></svg>"##,
        ),
        Case::square(
            "filter-blur-zero",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="none"><feGaussianBlur stdDeviation="0"/></filter><rect x="26" y="26" width="48" height="48" fill="#f2c14e" filter="url(#none)"/></svg>"##,
        ),
        // `feColorMatrix type="saturate" values="0"` desaturates the source —
        // the orange rect becomes a mid grey while everything else keeps its
        // colour.
        Case::square(
            "filter-color-matrix-saturate",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="gray"><feColorMatrix type="saturate" values="0"/></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#gray)"/></svg>"##,
        ),
        // `feColorMatrix type="luminanceToAlpha"` collapses RGB to luma in
        // alpha, leaving a translucent silhouette.
        Case::square(
            "filter-color-matrix-luminance",
            r##"<svg><rect width="100%" height="100%" fill="#888888"/><filter id="lum"><feColorMatrix type="luminanceToAlpha"/></filter><rect x="20" y="20" width="60" height="60" fill="#ffffff" filter="url(#lum)"/></svg>"##,
        ),
        // `feColorMatrix type="hueRotate"` rotates the chrominance plane —
        // here `180deg` swaps reds toward cyans.
        Case::square(
            "filter-color-matrix-hue-rotate",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="hue"><feColorMatrix type="hueRotate" values="180"/></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#hue)"/></svg>"##,
        ),
        // `feTurbulence` fills the filter region with deterministic
        // value-noise; the golden captures the seeded pattern.
        Case::square(
            "filter-turbulence",
            r##"<svg><filter id="noise"><feTurbulence baseFrequency="0.08" numOctaves="3" seed="1"/></filter><rect x="10" y="10" width="80" height="80" filter="url(#noise)"/></svg>"##,
        ),
        // `feTurbulence type="fractalNoise"` produces smoother
        // [0, 1]-mapped noise (no abs reflection).
        Case::square(
            "filter-turbulence-fractal",
            r##"<svg><filter id="noise"><feTurbulence baseFrequency="0.12" numOctaves="4" seed="3" type="fractalNoise"/></filter><rect x="10" y="10" width="80" height="80" filter="url(#noise)"/></svg>"##,
        ),
        // `feFlood` replaces the filter source with a solid colour, modulated
        // by `flood-opacity` — the source rect's blue is gone.
        Case::square(
            "filter-flood",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="fl"><feFlood flood-color="#c14b2b" flood-opacity="0.7"/></filter><rect x="20" y="20" width="60" height="60" fill="blue" filter="url(#fl)"/></svg>"##,
        ),
        // `feMorphology operator="dilate"` grows the silhouette by `radius`
        // pixels in each direction.
        Case::square(
            "filter-morphology-dilate",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="grow"><feMorphology operator="dilate" radius="4"/></filter><circle cx="50" cy="50" r="14" fill="#f2c14e" filter="url(#grow)"/></svg>"##,
        ),
        // `feMorphology operator="erode"` shrinks the silhouette; a 4px erode
        // pulls the square's edges 4 pixels inward.
        Case::square(
            "filter-morphology-erode",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="shrink"><feMorphology operator="erode" radius="4"/></filter><rect x="30" y="30" width="40" height="40" fill="#f2c14e" filter="url(#shrink)"/></svg>"##,
        ),
        // `feDropShadow` offsets and blurs the source alpha behind the
        // original geometry.
        Case::square(
            "filter-drop-shadow",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="sh"><feDropShadow dx="6" dy="6" stdDeviation="3" flood-color="#000000" flood-opacity="1"/></filter><rect x="24" y="24" width="40" height="40" fill="#c14b2b" filter="url(#sh)"/></svg>"##,
        ),
        // `feDisplacementMap scale="0"` is an identity passthrough — the
        // golden should look exactly like the underlying rect.
        Case::square(
            "filter-displacement-map-identity",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="warp"><feDisplacementMap scale="0"/></filter><rect x="20" y="20" width="60" height="60" fill="#f2c14e" filter="url(#warp)"/></svg>"##,
        ),
        // The MDN canonical `feDisplacementMap` example: a turbulence noise
        // texture (named via `result="turbulence"`) displaces the circle from
        // SourceGraphic, producing a black ink-blot outline. This pins the
        // DAG-input semantics — `in="SourceGraphic"` + `in2="turbulence"` —
        // not just per-primitive shader output.
        Case::sized(
            "filter-displacement-map-mdn",
            r##"<svg width="220" height="220" viewBox="0 0 220 220" xmlns="http://www.w3.org/2000/svg">
  <filter id="displacementFilter">
    <feTurbulence type="turbulence" baseFrequency="0.05" numOctaves="2" result="turbulence"/>
    <feDisplacementMap in2="turbulence" in="SourceGraphic" scale="50" xChannelSelector="R" yChannelSelector="G"/>
  </filter>
  <circle cx="100" cy="100" r="100" filter="url(#displacementFilter)"/>
</svg>"##,
            220,
            220,
        ),
        // A 3×3 sharpen kernel boosts the rect edges; the kernel sum is 1
        // so colours stay near their input intensity.
        Case::square(
            "filter-convolve-sharpen",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="sharp"><feConvolveMatrix kernelMatrix="0 -1 0 -1 5 -1 0 -1 0"/></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#sharp)"/></svg>"##,
        ),
        // `feComponentTransfer` linear functions doubled the red channel and
        // halved the blue channel of an olive-grey source.
        Case::square(
            "filter-component-transfer",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tr"><feComponentTransfer><feFuncR type="linear" slope="2" intercept="0"/><feFuncB type="linear" slope="0.5" intercept="0"/></feComponentTransfer></filter><rect x="20" y="20" width="60" height="60" fill="#888844" filter="url(#tr)"/></svg>"##,
        ),
        // `feDiffuseLighting` treats source alpha as a height field; the
        // distant light shades the disc into a lit cap.
        Case::square(
            "filter-diffuse-lighting",
            r##"<svg><filter id="light"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="#ffffff"><feDistantLight azimuth="45" elevation="60"/></feDiffuseLighting></filter><circle cx="50" cy="50" r="30" fill="#888888" filter="url(#light)"/></svg>"##,
        ),
        // `feSpecularLighting` adds a Phong highlight on the same surface,
        // tightening into a small bright spot at the lit pole.
        Case::square(
            "filter-specular-lighting",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="spec"><feSpecularLighting surfaceScale="5" specularConstant="1" specularExponent="20" lighting-color="#ffffff"><feDistantLight azimuth="135" elevation="30"/></feSpecularLighting></filter><circle cx="50" cy="50" r="32" fill="#444444" filter="url(#spec)"/></svg>"##,
        ),
        // A multi-primitive chain: blur first, then desaturate. The result is
        // a soft grey halo around the original red rect.
        Case::square(
            "filter-chain-blur-saturate",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="chain"><feGaussianBlur stdDeviation="3"/><feColorMatrix type="saturate" values="0"/></filter><rect x="32" y="32" width="36" height="36" fill="red" filter="url(#chain)"/></svg>"##,
        ),
        // --- DAG wiring snapshots --------------------------------------
        //
        // `in="SourceAlpha"` strips RGB and hands only the source alpha to
        // the next primitive. Here a colour-matrix recolours the silhouette
        // from blue to pure red — the blue is *gone*, proving SourceAlpha
        // (not SourceGraphic) was the input.
        Case::square(
            "filter-source-alpha",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="silhouette"><feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 1   0 0 0 0 0   0 0 0 0 0   0 0 0 1 0"/></filter><circle cx="50" cy="50" r="28" fill="blue" filter="url(#silhouette)"/></svg>"##,
        ),
        // `feGaussianBlur in="SourceAlpha"` blurs only the silhouette, which
        // is the canonical "build a soft shadow" recipe; the source RGB
        // never reaches the blur kernel, so the halo is purely the alpha
        // gradient — no colour bleed from the original blue circle.
        Case::square(
            "filter-blur-source-alpha",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="alphablur"><feGaussianBlur in="SourceAlpha" stdDeviation="5"/></filter><circle cx="50" cy="50" r="22" fill="blue" filter="url(#alphablur)"/></svg>"##,
        ),
        // `result="foo"` followed by a later `in="foo"`. The middle primitive
        // is `feFlood` (which writes a flat colour and discards any input);
        // the third primitive references the FIRST primitive's named output,
        // so the flood is shadowed and the recoloured red rect emerges.
        // A naive linear chain would surface the flood instead.
        Case::square(
            "filter-named-result-dag",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="dag"><feColorMatrix type="matrix" values="0 0 0 0 1   0 0 0 0 0   0 0 0 0 0   0 0 0 1 0" result="red"/><feFlood flood-color="#ffffff"/><feColorMatrix in="red" type="matrix" values="1 0 0 0 0   0 1 0 0 0   0 0 1 0 0   0 0 0 1 0"/></filter><rect x="30" y="30" width="40" height="40" fill="blue" filter="url(#dag)"/></svg>"##,
        ),
        // `in="SourceGraphic"` on the LAST primitive ignores everything the
        // chain computed and just composites the original source — proving
        // `SourceGraphic` keeps the original RGB+alpha addressable from any
        // position in the chain, not only at the start.
        Case::square(
            "filter-source-graphic-late",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="late"><feGaussianBlur stdDeviation="6"/><feColorMatrix in="SourceGraphic" type="matrix" values="0 0 0 0 0   0 1 0 0 0   0 0 0 0 0   0 0 0 1 0"/></filter><circle cx="50" cy="50" r="28" fill="#c14b2b" filter="url(#late)"/></svg>"##,
        ),
        // Generators chained: `feTurbulence` produces noise (no input), then
        // `feColorMatrix` (default `in` = previous primitive's output) tints
        // it green. Pins the "Default input on a non-first primitive" path.
        Case::square(
            "filter-turbulence-recolour",
            r##"<svg><filter id="green-noise"><feTurbulence baseFrequency="0.08" numOctaves="2" seed="2"/><feColorMatrix type="matrix" values="0 0 0 0 0   1 0 0 0 0   0 0 0 0 0   0 0 0 0 1"/></filter><rect x="10" y="10" width="80" height="80" filter="url(#green-noise)"/></svg>"##,
        ),
        // Bug guard (review P2): a parameter-wise no-op primitive with a
        // non-default `in` must still run through the chain. A naive
        // visibility gate would treat `stdDeviation="0"` as identity and
        // surface the original *blue* circle; the correct output is a
        // black SourceAlpha silhouette.
        Case::square(
            "filter-source-alpha-zero-blur",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="strip"><feGaussianBlur in="SourceAlpha" stdDeviation="0"/></filter><circle cx="50" cy="50" r="28" fill="blue" filter="url(#strip)"/></svg>"##,
        ),
        // Bug guard (review P2): a 2-entry `tableValues="0 1"` is identity.
        // The previous fixed-4-entry implementation incorrectly stored
        // `[0, 1, 0, 0]` and would have produced very wrong values for the
        // upper half of the input range.
        Case::square(
            "filter-component-transfer-table-identity",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="t"><feComponentTransfer><feFuncR type="table" tableValues="0 1"/><feFuncG type="table" tableValues="0 1"/><feFuncB type="table" tableValues="0 1"/></feComponentTransfer></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#t)"/></svg>"##,
        ),
        // Bug guard (review P3): `feSpotLight` used to be silently dropped
        // and fell back to a default distant light. With proper cone math
        // the disc shows a bright on-axis hotspot and dark off-axis edges.
        Case::square(
            "filter-spot-lighting",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="spot"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="#ffffff"><feSpotLight x="50" y="50" z="40" pointsAtX="50" pointsAtY="50" pointsAtZ="0" specularExponent="4" limitingConeAngle="35"/></feDiffuseLighting></filter><circle cx="50" cy="50" r="28" fill="#888888" filter="url(#spot)"/></svg>"##,
        ),
        // SVG image filters: referenced `<feImage>` data URLs become
        // lazily-uploaded GPU textures, then composite in painter order just
        // like geometry-sourced filters.
        Case::square(
            "filter-image-explicit-size",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{quadrants}" x="18" y="16" width="64" height="48"/></filter><rect x="2" y="2" width="12" height="12" fill="white" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::square(
            "filter-image-intrinsic-size",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{yellow}" x="40" y="43"/></filter><rect width="1" height="1" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::sized(
            "filter-image-percent-geometry",
            format!(
                r##"<svg width="200" height="100"><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{green}" x="20%" y="25%" width="30%" height="40%"/></filter><rect width="1" height="1" filter="url(#tex)"/></svg>"##
            ),
            200,
            100,
        ),
        Case::square(
            "filter-image-xlink-href",
            format!(
                r##"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage xlink:href="{orange}" x="30" y="28" width="40" height="44"/></filter><rect width="1" height="1" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::square(
            "filter-image-alpha",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{alpha_cross}" x="26" y="22" width="48" height="56"/></filter><rect width="1" height="1" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::square(
            "filter-image-blur",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{blue}" x="36" y="34" width="28" height="24"/><feGaussianBlur stdDeviation="5"/></filter><rect width="1" height="1" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::square(
            "filter-image-painter-order",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{red}" x="22" y="26" width="56" height="44"/></filter><rect width="1" height="1" filter="url(#tex)"/><circle cx="64" cy="50" r="22" fill="#11aa55"/></svg>"##
            ),
        ),
        Case::square(
            "filter-image-zero-size-skipped",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{red}" x="20" y="20" width="0" height="40"/></filter><rect width="100%" height="100%" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::square(
            "filter-image-unsupported-skipped",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="data:text/plain;base64,Zm9v" x="20" y="20" width="60" height="60"/></filter><rect width="100%" height="100%" filter="url(#tex)"/></svg>"##,
        ),
        Case::square(
            "filter-image-invalid-data-skipped",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="plain"><feImage href="data:image/png,abc" x="12" y="24" width="18" height="52"/></filter><filter id="bad64"><feImage href="data:image/png;base64,@@@" x="41" y="24" width="18" height="52"/></filter><filter id="badpng"><feImage href="data:image/png;base64,bm90IGEgcG5n" x="70" y="24" width="18" height="52"/></filter><rect width="100%" height="100%" fill="red" filter="url(#plain)"/><rect width="100%" height="100%" fill="red" filter="url(#bad64)"/><rect width="100%" height="100%" fill="red" filter="url(#badpng)"/></svg>"##,
        ),
        Case::square(
            "filter-image-missing-href-fallback",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="empty"><feImage x="20" y="20" width="60" height="60"/></filter><rect x="24" y="28" width="52" height="44" fill="#c14b2b" filter="url(#empty)"/></svg>"##,
        ),
        // The canonical SVG sample, rendered at its declared 300×200 size. The
        // `<rect width="100%">` exercises percentage lengths; the `<text>` is
        // parsed but not yet rendered (text rendering is a separate milestone),
        // so the golden shows a red ground with a centred green disc.
        Case::sized(
            "svg-rect-circle-text",
            r#"<svg version="1.1" width="300" height="200" xmlns="http://www.w3.org/2000/svg">
  <rect width="100%" height="100%" fill="red" />
  <circle cx="150" cy="100" r="80" fill="green" />
  <text x="150" y="125" font-size="60" text-anchor="middle" fill="white">SVG</text>
</svg>"#,
            300,
            200,
        ),
        // The movable 3D camera: one asymmetric scene viewed from four
        // positions. `camera-front` is the upright, head-on reference; the
        // others pan, tilt and dolly the eye through 3D space.
        Case::square("camera-front", CAMERA_SCENE).with_camera(front),
        Case::square("camera-pan", CAMERA_SCENE).with_camera(pan),
        Case::square("camera-angled", CAMERA_SCENE).with_camera(angled),
        Case::square("camera-dolly", CAMERA_SCENE).with_camera(dolly),
    ]
}

#[test]
fn shape_snapshots_match_references() {
    let renderer = match Renderer::headless() {
        Ok(renderer) => renderer,
        Err(RenderError::NoAdapter) => {
            eprintln!("skipping snapshot tests: no GPU adapter available");
            return;
        }
        Err(error) => panic!("renderer construction failed: {error}"),
    };
    let update = std::env::var_os("SVG3_UPDATE_SNAPSHOTS").is_some();

    let mut updated: Vec<&str> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for case in cases() {
        let document = parse(&case.svg).expect("snapshot fixture should parse");
        let config = RenderConfig {
            width: case.width,
            height: case.height,
            camera: case.camera,
            ..RenderConfig::default()
        };
        let image = renderer
            .render_to_image(&document, config)
            .unwrap_or_else(|error| panic!("render failed for `{}`: {error}", case.name));

        let golden = snapshot_path(case.name);
        if update {
            write_png(&golden, &image);
            updated.push(case.name);
        } else if !golden.exists() {
            // A missing golden must fail — otherwise a deleted PNG or a new
            // case without a committed golden would pass CI silently.
            failures.push(format!(
                "{}: no golden at `{}` — create it with SVG3_UPDATE_SNAPSHOTS=1",
                case.name,
                golden.display(),
            ));
        } else if let Err(message) = compare(&golden, &image) {
            failures.push(format!("{}: {message}", case.name));
        }
    }

    if !updated.is_empty() {
        eprintln!(
            "wrote {} snapshot golden(s): {}",
            updated.len(),
            updated.join(", "),
        );
    }
    assert!(
        failures.is_empty(),
        "{} snapshot case(s) failed:\n  {}\n\nFor a pixel mismatch, inspect \
         the `*.actual.png` written beside the golden. If the change is \
         intentional (or a golden is missing), regenerate the goldens with \
         `SVG3_UPDATE_SNAPSHOTS=1`.",
        failures.len(),
        failures.join("\n  "),
    );
}

/// Absolute path of the golden PNG for `name`.
fn snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.png"))
}

/// Compare `image` against the golden at `golden`. On a mismatch the actual
/// render is written beside the golden as `<name>.actual.png` for review.
fn compare(golden: &Path, image: &Image) -> Result<(), String> {
    let (width, height, expected) = read_png(golden);
    if (width, height) != (image.width, image.height) {
        write_png(&golden.with_extension("actual.png"), image);
        return Err(format!(
            "golden is {width}x{height} but the render is {}x{}",
            image.width, image.height,
        ));
    }

    let differing = expected
        .chunks_exact(4)
        .zip(image.pixels.chunks_exact(4))
        .filter(|(want, got)| {
            want.iter()
                .zip(got.iter())
                .any(|(w, g)| w.abs_diff(*g) > CHANNEL_TOLERANCE)
        })
        .count();
    // A small budget absorbs the ~1px anti-aliasing rim that SDF shapes
    // (circles, ellipses, rounded rects, lines) carry along their whole
    // perimeter, which a different GPU may rasterise a hair differently; a
    // real regression shifts far more than this.
    let budget = (image.width * image.height) as usize / 100;
    if differing > budget {
        write_png(&golden.with_extension("actual.png"), image);
        return Err(format!(
            "{differing} pixel(s) differ by more than {CHANNEL_TOLERANCE} (budget {budget})"
        ));
    }
    Ok(())
}

/// Encode `image` as an `RGBA8` PNG at `path`.
fn write_png(path: &Path, image: &Image) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create snapshots directory");
    }
    let file = std::fs::File::create(path).expect("create snapshot file");
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write PNG header");
    writer
        .write_image_data(&image.pixels)
        .expect("write PNG data");
}

/// Decode the `RGBA8` PNG at `path` into `(width, height, pixels)`.
fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let file = std::io::BufReader::new(std::fs::File::open(path).expect("open golden PNG"));
    let mut reader = png::Decoder::new(file).read_info().expect("read PNG info");
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    // Goldens are always written as 8-bit RGBA, so the frame is `w * h * 4`.
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    reader.next_frame(&mut pixels).expect("decode PNG frame");
    (width, height, pixels)
}
