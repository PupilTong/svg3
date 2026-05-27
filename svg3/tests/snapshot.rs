//! End-to-end snapshot tests for shape rendering.
//!
//! Each case parses an svg3 document — the SVG WPT `shapes/rect-*`,
//! `shapes/circle-*`, `shapes/ellipse-*`, and `shapes/polygon-*` reference
//! tests, plus polyline fill, stroked shapes, line, path, markers, SVG paint
//! servers, the svg3 3D primitives, Gaussian blur and image filters,
//! mixed-shape, and canonical SVG samples — renders
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
//! SVG3_UPDATE_SNAPSHOTS=1 cargo test -p svg3 --test snapshot
//! ```
//!
//! Rendering needs a GPU adapter, so the suite self-skips where none is
//! available (it runs on macOS/Metal; it is skipped on a GPU-less host).

use std::borrow::Cow;
use std::path::{Path, PathBuf};

mod common;

use common::{png_data_uri, quadrant_png_data_uri, solid_png_data_uri};
use svg3::dom::parse;
use svg3::render::{Camera, Image, RenderConfig, RenderError, Renderer};

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

/// The three-sphere lighting document — three green discs each lit by a
/// different `<fe…Light>` source through a `feDiffuseLighting` + arithmetic
/// `feComposite` chain. Shared between the orthographic and perspective
/// snapshot cases so the only difference between their goldens is the
/// projection (and the depth handling that surfaces under perspective).
const THREE_SPHERES_LIGHTING_SVG: &str = r##"<svg width="440" height="140" xmlns="http://www.w3.org/2000/svg">
  <filter id="lightMe1">
    <feDiffuseLighting in="SourceGraphic" result="light" lighting-color="white">
      <fePointLight x="150" y="60" z="20" />
    </feDiffuseLighting>
    <feComposite in="SourceGraphic" in2="light" operator="arithmetic" k1="1" k2="0" k3="0" k4="0" />
  </filter>
  <circle cx="170" cy="80" r="50" fill="green" filter="url(#lightMe1)" />
  <filter id="lightMe2">
    <feDiffuseLighting in="SourceGraphic" result="light" lighting-color="white">
      <feDistantLight azimuth="240" elevation="20" />
    </feDiffuseLighting>
    <feComposite in="SourceGraphic" in2="light" operator="arithmetic" k1="1" k2="0" k3="0" k4="0" />
  </filter>
  <circle cx="280" cy="80" r="50" fill="green" filter="url(#lightMe2)" />
  <filter id="lightMe3">
    <feDiffuseLighting in="SourceGraphic" result="light" lighting-color="white">
      <feSpotLight x="360" y="5" z="30" limitingConeAngle="20" pointsAtX="390" pointsAtY="80" pointsAtZ="0" />
    </feDiffuseLighting>
    <feComposite in="SourceGraphic" in2="light" operator="arithmetic" k1="1" k2="0" k3="0" k4="0" />
  </filter>
  <circle cx="390" cy="80" r="50" fill="green" filter="url(#lightMe3)" />
</svg>"##;

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

    let quadrants = quadrant_png_data_uri([
        [242, 193, 78, 255],
        [37, 99, 235, 255],
        [17, 170, 85, 255],
        [193, 75, 43, 255],
    ]);
    let alpha_cross = alpha_cross_png_data_uri();
    let red = solid_png_data_uri(1, 1, [193, 75, 43, 255]);
    let green = solid_png_data_uri(1, 1, [17, 170, 85, 255]);
    let blue = solid_png_data_uri(1, 1, [37, 99, 235, 255]);
    let yellow = solid_png_data_uri(20, 14, [242, 193, 78, 255]);
    let orange = solid_png_data_uri(1, 1, [255, 165, 0, 255]);
    // 4:1 image used by the `preserveAspectRatio` cases. Half red and half
    // blue along its long axis so the alignment / scaling behaviour is
    // unambiguous in the rendered golden.
    let wide_image = {
        let mut rgba = Vec::with_capacity(16 * 4 * 4);
        for _ in 0..4 {
            for x in 0..16 {
                let color = if x < 8 {
                    [193, 75, 43, 255]
                } else {
                    [37, 99, 235, 255]
                };
                rgba.extend_from_slice(&color);
            }
        }
        png_data_uri(16, 4, &rgba)
    };

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
        // SVG paint servers: `<linearGradient>` with user-space coordinates
        // and three `<stop>` entries, sampled per fragment in the shape
        // shader.
        Case::square(
            "paint-linear-gradient-fill-user-space",
            r##"<svg><defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="16" y1="20" x2="84" y2="20"><stop offset="0%" stop-color="#c14b2b"/><stop offset="50%" stop-color="#f2c14e"/><stop offset="100%" stop-color="#2563eb"/></linearGradient></defs><rect width="100%" height="100%" fill="#13294b"/><rect x="16" y="20" width="68" height="60" fill="url(#g)"/></svg>"##,
        ),
        // Default `objectBoundingBox` gradient units on stroke paint. The
        // gradient resolves against the authored rect bounds, not the
        // expanded stroke mesh.
        Case::square(
            "paint-linear-gradient-stroke-object-bbox",
            r##"<svg><defs><linearGradient id="g"><stop offset="0" stop-color="#c14b2b"/><stop offset="1" stop-color="#2563eb"/></linearGradient></defs><rect width="100%" height="100%" fill="#13294b"/><rect x="24" y="24" width="52" height="52" rx="8" fill="none" stroke="url(#g)" stroke-width="14"/></svg>"##,
        ),
        // `<radialGradient>` plus `<stop>` presentation attributes and style
        // declarations, including stop-opacity blended over the background.
        Case::square(
            "paint-radial-gradient-stop-opacity",
            r##"<svg><defs><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="50" cy="50" r="38"><stop style="offset: 0%; stop-color: #f2c14e; stop-opacity: 1"/><stop offset="55%" stop-color="#c14b2b" stop-opacity="0.75"/><stop style="offset: 100%; stop-color: #2563eb; stop-opacity: 35%"/></radialGradient></defs><rect width="100%" height="100%" fill="#13294b"/><circle cx="50" cy="50" r="38" fill="url(#g)"/></svg>"##,
        ),
        // `<pattern patternUnits="userSpaceOnUse">` with rectangular child
        // paints. The golden exposes tile repetition and child painter order.
        Case::square(
            "paint-pattern-fill-user-space",
            r##"<svg><defs><pattern id="p" patternUnits="userSpaceOnUse" width="16" height="16"><rect width="16" height="16" fill="#2563eb"/><rect width="8" height="8" fill="#f2c14e"/><rect x="8" y="8" width="8" height="8" fill="#c14b2b" fill-opacity="0.75"/></pattern></defs><rect width="100%" height="100%" fill="#13294b"/><rect x="14" y="14" width="72" height="72" fill="url(#p)"/></svg>"##,
        ),
        // Default `objectBoundingBox` pattern units: the tile size tracks the
        // painted element bounds. Semi-transparent child paint proves pattern
        // items are composited source-over in document order.
        Case::square(
            "paint-pattern-object-bbox-composite",
            r##"<svg><defs><pattern id="p" width="25%" height="25%"><rect width="15" height="15" fill="#11aa55"/><rect x="4" y="4" width="11" height="11" fill="#13294b" fill-opacity="0.55"/></pattern></defs><rect width="100%" height="100%" fill="#13294b"/><rect x="20" y="20" width="60" height="60" fill="url(#p)"/></svg>"##,
        ),
        // Paint servers feed SourceGraphic before filter execution. The
        // desaturating filter should see the tiled pattern, not the fallback
        // solid fill or an unpainted silhouette.
        Case::square(
            "paint-pattern-filter-source-graphic",
            r##"<svg><defs><pattern id="p" patternUnits="userSpaceOnUse" width="10" height="10"><rect width="5" height="10" fill="#c14b2b"/><rect x="5" width="5" height="10" fill="#2563eb"/></pattern><filter id="gray"><feColorMatrix type="saturate" values="0"/></filter></defs><rect width="100%" height="100%" fill="#13294b"/><rect x="18" y="18" width="64" height="64" fill="url(#p)" filter="url(#gray)"/></svg>"##,
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
        // WPT-derived SVG filter snapshots. These mirror the passing reduced
        // fixtures in `tests/wpt_filter_cases.rs` so WPT migrations also have
        // reviewable visual output.
        Case::square(
            "wpt-filter-gauss-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><filter id="flat"><feGaussianBlur stdDeviation="6 1"/></filter><rect x="16" y="24" width="18" height="18" fill="#2563eb" filter="url(#soft)"/><rect x="56" y="24" width="18" height="18" fill="#f2c14e" filter="url(#flat)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-gauss-02",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="v"><feGaussianBlur stdDeviation="0 6"/></filter><filter id="h"><feGaussianBlur stdDeviation="6 0"/></filter><rect x="22" y="24" width="16" height="16" fill="#2563eb" filter="url(#v)"/><rect x="62" y="24" width="16" height="16" fill="#c14b2b" filter="url(#h)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-gauss-03",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="identity"><feGaussianBlur stdDeviation="0"/></filter><rect x="26" y="26" width="48" height="48" fill="#11aa55" filter="url(#identity)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-color-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="m"><feColorMatrix type="matrix" values="0 0 0 0 0  0 1 0 0 0  0 0 0 0 0  0 0 0 1 0"/></filter><filter id="s"><feColorMatrix type="saturate" values="0"/></filter><filter id="h"><feColorMatrix type="hueRotate" values="120"/></filter><filter id="l"><feColorMatrix type="luminanceToAlpha"/></filter><rect x="10" y="18" width="18" height="64" fill="white" filter="url(#m)"/><rect x="30" y="18" width="18" height="64" fill="red" filter="url(#s)"/><rect x="52" y="18" width="18" height="64" fill="red" filter="url(#h)"/><rect x="74" y="18" width="18" height="64" fill="white" filter="url(#l)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-color-02",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="ct"><feComponentTransfer><feFuncR type="identity"/><feFuncR type="linear" slope="0" intercept="1"/><feFuncR type="linear" slope="0" intercept="0"/></feComponentTransfer></filter><rect x="20" y="20" width="60" height="60" fill="white" filter="url(#ct)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-comptran-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="t"><feComponentTransfer><feFuncR type="table" tableValues="0 0"/><feFuncG type="identity"/><feFuncB type="identity"/></feComponentTransfer></filter><filter id="d"><feComponentTransfer><feFuncG type="discrete" tableValues="0 1"/></feComponentTransfer></filter><filter id="l"><feComponentTransfer><feFuncB type="linear" slope="0" intercept="1"/></feComponentTransfer></filter><filter id="g"><feComponentTransfer><feFuncR type="gamma" amplitude="1" exponent="2" offset="0"/></feComponentTransfer></filter><rect x="8" y="22" width="16" height="56" fill="white" filter="url(#t)"/><rect x="31" y="22" width="16" height="56" fill="#c0c0c0" filter="url(#d)"/><rect x="54" y="22" width="16" height="56" fill="black" filter="url(#l)"/><rect x="77" y="22" width="16" height="56" fill="#808080" filter="url(#g)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-conv-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="sharp"><feConvolveMatrix kernelMatrix="0 -1 0 -1 5 -1 0 -1 0"/></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#sharp)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-conv-02",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="c"><feConvolveMatrix order="3 3" kernelMatrix="0 0 0 0 1 0 0 0 0" preserveAlpha="true"/></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#c)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-conv-03",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="c"><feFlood flood-color="#2563eb"/><feConvolveMatrix in="SourceGraphic" kernelMatrix="0 0 0 0 1 0 0 0 0"/></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#c)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-conv-04",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="c"><feConvolveMatrix kernelMatrix="0 0 0 0 1 0 0 0 0" bias="0.5"/></filter><rect x="20" y="20" width="60" height="60" fill="black" filter="url(#c)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-diffuse-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="light"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="white"><feDistantLight azimuth="45" elevation="60"/></feDiffuseLighting></filter><circle cx="50" cy="50" r="30" fill="white" filter="url(#light)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-displace-01",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="warp"><feImage href="{quadrants}" x="0" y="0" width="100" height="100" result="map"/><feDisplacementMap in="SourceGraphic" in2="map" scale="8" xChannelSelector="R" yChannelSelector="G"/></filter><rect x="32" y="32" width="36" height="36" fill="#2563eb" filter="url(#warp)"/></svg>"##
            ),
        ),
        Case::square(
            "wpt-filter-displace-02",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="warp"><feDisplacementMap in="SourceGraphic" in2="SourceAlpha" scale="0"/></filter><rect x="20" y="20" width="60" height="60" fill="#2563eb" filter="url(#warp)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-image-01",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{red}" x="24" y="22" width="52" height="48"/></filter><rect width="100" height="100" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::sized(
            "wpt-filter-image-03",
            format!(
                r##"<svg width="200" height="100"><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{green}" x="20%" y="25%" width="30%" height="40%"/></filter><rect width="200" height="100" filter="url(#tex)"/></svg>"##
            ),
            200,
            100,
        ),
        Case::square(
            "wpt-filter-image-04",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{blue}" x="30" y="22" width="40" height="36"/></filter><rect width="100" height="100" filter="url(#tex)"/></svg>"##
            ),
        ),
        Case::square(
            "wpt-filter-light-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="d"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><feDistantLight azimuth="45" elevation="60"/></feDiffuseLighting></filter><filter id="p"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><fePointLight x="50" y="50" z="40"/></feDiffuseLighting></filter><filter id="s"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><feSpotLight x="50" y="50" z="40" pointsAtX="50" pointsAtY="50" pointsAtZ="0" limitingConeAngle="35"/></feDiffuseLighting></filter><circle cx="25" cy="50" r="18" fill="white" filter="url(#d)"/><circle cx="50" cy="50" r="18" fill="white" filter="url(#p)"/><circle cx="75" cy="50" r="18" fill="white" filter="url(#s)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-light-02",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="spec"><feSpecularLighting surfaceScale="5" specularConstant="1" specularExponent="8"><feDistantLight azimuth="135" elevation="45"/></feSpecularLighting></filter><circle cx="50" cy="50" r="30" fill="white" filter="url(#spec)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-light-03",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="spec"><feSpecularLighting surfaceScale="5" specularConstant="1" specularExponent="8"><fePointLight x="50" y="50" z="40"/></feSpecularLighting></filter><circle cx="50" cy="50" r="30" fill="white" filter="url(#spec)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-light-04",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="spot"><feDiffuseLighting surfaceScale="5" diffuseConstant="1"><feSpotLight x="50" y="50" z="40" pointsAtX="50" pointsAtY="50" pointsAtZ="0" limitingConeAngle="35"/></feDiffuseLighting></filter><circle cx="50" cy="50" r="30" fill="white" filter="url(#spot)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-morph-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="grow"><feMorphology operator="dilate" radius="4"/></filter><filter id="shrink"><feMorphology operator="erode" radius="4"/></filter><rect x="22" y="36" width="18" height="18" fill="#11aa55" filter="url(#grow)"/><rect x="58" y="28" width="28" height="28" fill="#f2c14e" filter="url(#shrink)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-specular-01",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="spec"><feSpecularLighting surfaceScale="10" specularConstant="2" specularExponent="4" lighting-color="red"><feDistantLight azimuth="135" elevation="45"/></feSpecularLighting></filter><circle cx="50" cy="50" r="30" fill="white" filter="url(#spec)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-turb-01",
            r##"<svg><filter id="n"><feTurbulence type="fractalNoise" baseFrequency="0.08" numOctaves="2" seed="1"/></filter><rect x="10" y="10" width="80" height="80" filter="url(#n)"/></svg>"##,
        ),
        Case::square(
            "wpt-filter-turb-02",
            r##"<svg><filter id="n"><feTurbulence seed="2" baseFrequency="0.08" type="turbulence"/></filter><rect x="10" y="10" width="80" height="80" filter="url(#n)"/></svg>"##,
        ),
        Case::square(
            "wpt-linking-href-feimage",
            format!(
                r##"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{red}" xlink:href="{green}" x="24" y="22" width="52" height="48"/></filter><rect width="100" height="100" filter="url(#tex)"/></svg>"##
            ),
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
        // The MDN canonical "three lights side-by-side" example: a row of
        // three green discs lit respectively by a `<fePointLight>`, a
        // `<feDistantLight>`, and a `<feSpotLight>`, each composited with
        // `feComposite operator="arithmetic" k1=1 k2=0 k3=0 k4=0` —
        // i.e. multiply the green source by the lighting result. Pins the
        // canonical "modulate the source by a light" recipe across all
        // three light source kinds in a single document, and exercises a
        // wide non-square user-space (440×140).
        Case::sized(
            "filter-three-spheres-lighting",
            THREE_SPHERES_LIGHTING_SVG,
            440,
            140,
        ),
        // Same three-sphere doc, but viewed through the 3D perspective
        // camera that `app-macos` uses. Pins the fix for a
        // multi-filter perspective bug: the filter composite's halo NDC
        // depth was hardcoded to `0.5` (the orthographic z=0 plane), so
        // under perspective every filter past the first one wrote depth
        // `> 0.5` for its actual geometry and lost the `LessEqual`
        // depth test against the previous filter's halo, dropping every
        // subsequent sphere. The shader now reads the renderer-supplied
        // NDC depth of `z = 0` under the active projection.
        Case::sized(
            "filter-three-spheres-lighting-perspective",
            THREE_SPHERES_LIGHTING_SVG,
            440,
            140,
        )
        .with_camera(Camera::facing(440, 140)),
        // Same three-sphere doc, viewed through a *yawed* perspective
        // camera. Pins the second half of the perspective-filter fix:
        // light positions and spot-cone math used to live in filter
        // texture pixel coordinates (with a single `extent / viewport`
        // pre-scale that assumed orthographic). Under a yawed camera
        // the geometry projects non-uniformly, so the cone test shifted
        // wildly off the lit region and left a hard black wedge across
        // sphere 3. The lighting shader now recovers each texel's
        // user-space `(X, Y)` via an inverse homography of the
        // projection's `z = 0` plane, so light vectors evaluate in
        // SVG user-space units regardless of camera pose.
        Case::sized(
            "filter-three-spheres-lighting-yawed",
            THREE_SPHERES_LIGHTING_SVG,
            440,
            140,
        )
        .with_camera({
            let mut c = Camera::facing(440, 140);
            // Match the kind of yaw an `app-macos` user produces with a
            // right-arrow orbit: shift the eye to the +X side, keep the
            // look-at at the document centre, eye at the same distance.
            c.eye.x += 120.0;
            c
        }),
        // The same three-sphere doc viewed through a *pitched* perspective
        // camera. Pins the third aspect of the perspective-filter fix: the
        // halo NDC depth used to be a single value sampled at the document
        // centre, which is correct for any axis-aligned projection but
        // wrong as soon as the z = 0 plane crosses a range of NDC depths
        // across the screen — under pitch every fragment past the centre
        // failed the `LessEqual` depth test and was clipped, leaving the
        // bottom half of spheres past the first one missing. The composite
        // shader now computes the z = 0 plane NDC depth per fragment from
        // the inverse projection packed into the composite uniform.
        Case::sized(
            "filter-three-spheres-lighting-pitched",
            THREE_SPHERES_LIGHTING_SVG,
            440,
            140,
        )
        .with_camera({
            let mut c = Camera::facing(440, 140);
            // Match a `+15°` pitch from the `app-macos` orbit camera:
            // lift the eye towards `-Y` and pull it back along `+Z` so
            // the look direction tilts down at the document.
            c.eye.y -= 100.0;
            c
        }),
        // The MDN canonical `feSpecularLighting` example, verbatim: a 220×220
        // viewBox circle lit by a `<fePointLight>` with `lighting-color` =
        // `#bbbbbb` and `specularExponent="20"`, then composited onto the
        // source via `feComposite operator="arithmetic"` (`k2=1, k3=1`). The
        // golden pins the canonical recipe for "add a specular highlight on
        // top of the source" — the same pattern shown on the MDN page for
        // `feSpecularLighting` — and exercises a non-square user-space (the
        // `width="200"` plus `viewBox="0 0 220 220"`).
        Case::sized(
            "filter-specular-lighting-mdn",
            r##"<svg
  height="200"
  width="200"
  viewBox="0 0 220 220"
  xmlns="http://www.w3.org/2000/svg">
  <filter id="filter">
    <feSpecularLighting
      result="specOut"
      specularExponent="20"
      lighting-color="#bbbbbb">
      <fePointLight x="50" y="75" z="200" />
    </feSpecularLighting>
    <feComposite
      in="SourceGraphic"
      in2="specOut"
      operator="arithmetic"
      k1="0"
      k2="1"
      k3="1"
      k4="0" />
  </filter>
  <circle cx="110" cy="110" r="100" filter="url(#filter)" />
</svg>"##,
            200,
            200,
        ),
        // `feDiffuseLighting` with `feDistantLight`, varying `surfaceScale`
        // (positive vs negative). WPT `filters-diffuse-01` exercises this
        // axis; per SVG 1.1 §15.21 a negative surfaceScale inverts the
        // height-field, flipping the lit/dark sides.
        Case::square(
            "filter-diffuse-lighting-negative-surface-scale",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="up"><feDiffuseLighting surfaceScale="10" diffuseConstant="1" lighting-color="white"><feDistantLight azimuth="45" elevation="45"/></feDiffuseLighting></filter><filter id="down"><feDiffuseLighting surfaceScale="-10" diffuseConstant="1" lighting-color="white"><feDistantLight azimuth="45" elevation="45"/></feDiffuseLighting></filter><circle cx="30" cy="50" r="18" fill="#666666" filter="url(#up)"/><circle cx="70" cy="50" r="18" fill="#666666" filter="url(#down)"/></svg>"##,
        ),
        // WPT `filters-light-02`-derived: `azimuth="0"` (light coming from
        // the +x direction in SVG y-down user space) lights the right side
        // of the disc; `azimuth="180"` lights the left. Pins the spec
        // convention that azimuth is interpreted clockwise from the +x axis.
        Case::square(
            "filter-distant-light-azimuth-pair",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="r"><feSpecularLighting surfaceScale="5" specularConstant="20" specularExponent="10" lighting-color="white"><feDistantLight azimuth="0" elevation="30"/></feSpecularLighting></filter><filter id="l"><feSpecularLighting surfaceScale="5" specularConstant="20" specularExponent="10" lighting-color="white"><feDistantLight azimuth="180" elevation="30"/></feSpecularLighting></filter><circle cx="30" cy="50" r="18" fill="black" filter="url(#r)"/><circle cx="70" cy="50" r="18" fill="black" filter="url(#l)"/></svg>"##,
        ),
        // WPT `filters-light-05`-derived: `elevation="90"` is straight
        // overhead (full diffuse strength at the lit pole), `elevation="0"`
        // is grazing (much darker). Pins the spec convention that elevation
        // is the angle above the surface plane in degrees.
        Case::square(
            "filter-distant-light-elevation-pair",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="overhead"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="white"><feDistantLight azimuth="0" elevation="90"/></feDiffuseLighting></filter><filter id="grazing"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="white"><feDistantLight azimuth="0" elevation="0"/></feDiffuseLighting></filter><circle cx="30" cy="50" r="18" fill="white" filter="url(#overhead)"/><circle cx="70" cy="50" r="18" fill="white" filter="url(#grazing)"/></svg>"##,
        ),
        // WPT `filters-specular-01`-derived: side-by-side `specularExponent`
        // values 1, 4, and 20. The exponent controls highlight tightness —
        // 1 spreads diffusely; 20 collapses to a small bright spot.
        Case::square(
            "filter-specular-exponent-sweep",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="e1"><feSpecularLighting surfaceScale="10" specularConstant="1" specularExponent="1" lighting-color="white"><feDistantLight azimuth="45" elevation="45"/></feSpecularLighting></filter><filter id="e4"><feSpecularLighting surfaceScale="10" specularConstant="1" specularExponent="4" lighting-color="white"><feDistantLight azimuth="45" elevation="45"/></feSpecularLighting></filter><filter id="e20"><feSpecularLighting surfaceScale="10" specularConstant="1" specularExponent="20" lighting-color="white"><feDistantLight azimuth="45" elevation="45"/></feSpecularLighting></filter><circle cx="20" cy="50" r="14" fill="#222222" filter="url(#e1)"/><circle cx="50" cy="50" r="14" fill="#222222" filter="url(#e4)"/><circle cx="80" cy="50" r="14" fill="#222222" filter="url(#e20)"/></svg>"##,
        ),
        // WPT `filters-light-04`-derived: `feSpotLight` with
        // `limitingConeAngle="0"` produces no illumination (zero cone), and
        // a wide cone produces near-full illumination. Pins that the cone
        // limit is honored even with extreme angles.
        Case::square(
            "filter-spot-limiting-cone-extremes",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="narrow"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="white"><feSpotLight x="50" y="50" z="40" pointsAtX="50" pointsAtY="50" pointsAtZ="0" specularExponent="0" limitingConeAngle="0.001"/></feDiffuseLighting></filter><filter id="wide"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="white"><feSpotLight x="50" y="50" z="40" pointsAtX="50" pointsAtY="50" pointsAtZ="0" specularExponent="0" limitingConeAngle="89"/></feDiffuseLighting></filter><circle cx="30" cy="50" r="18" fill="white" filter="url(#narrow)"/><circle cx="70" cy="50" r="18" fill="white" filter="url(#wide)"/></svg>"##,
        ),
        // `feSpotLight specularExponent="0"`: the spec's "no falloff inside
        // the cone" case. The cone-factor should be a flat 1.0 wherever the
        // cone is hit, giving a sharp lit disc rather than a soft spot.
        Case::square(
            "filter-spot-no-cone-falloff",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="flat"><feDiffuseLighting surfaceScale="5" diffuseConstant="1" lighting-color="white"><feSpotLight x="50" y="50" z="50" pointsAtX="50" pointsAtY="50" pointsAtZ="0" specularExponent="0" limitingConeAngle="30"/></feDiffuseLighting></filter><circle cx="50" cy="50" r="32" fill="white" filter="url(#flat)"/></svg>"##,
        ),
        // Combined diffuse + specular via `feMerge` (the WPT `light-05`
        // recipe). Stacks a diffuse base under a specular highlight to
        // produce a "shaded sphere" look from one distant light.
        Case::square(
            "filter-lighting-diffuse-plus-specular-merge",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="combined"><feDiffuseLighting surfaceScale="10" diffuseConstant="1" lighting-color="#88aacc" result="diff"><feDistantLight azimuth="135" elevation="45"/></feDiffuseLighting><feSpecularLighting in="SourceGraphic" surfaceScale="10" specularConstant="1" specularExponent="20" lighting-color="white" result="spec"><feDistantLight azimuth="135" elevation="45"/></feSpecularLighting><feMerge><feMergeNode in="diff"/><feMergeNode in="spec"/></feMerge></filter><circle cx="50" cy="50" r="30" fill="white" filter="url(#combined)"/></svg>"##,
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
        // feOffset: translate the source by (dx, dy). The blue square sits
        // at its authored position; the offset shifts the filter output
        // toward the bottom-right, leaving the original region transparent.
        Case::square(
            "filter-offset-basic",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="o"><feOffset dx="20" dy="14"/></filter><rect x="14" y="14" width="30" height="30" fill="#2563eb" filter="url(#o)"/></svg>"##,
        ),
        // The classic SVG drop-shadow recipe: feGaussianBlur(SourceAlpha) +
        // feOffset + feMerge. The source rect stays visible above its own
        // soft, offset shadow.
        Case::square(
            "filter-offset-merge-drop-shadow",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="ds"><feGaussianBlur in="SourceAlpha" stdDeviation="3" result="blur"/><feOffset in="blur" dx="6" dy="6" result="shadow"/><feMerge><feMergeNode in="shadow"/><feMergeNode in="SourceGraphic"/></feMerge></filter><rect x="28" y="22" width="32" height="32" fill="#f2c14e" filter="url(#ds)"/></svg>"##,
        ),
        // feMerge with three named layers stacked in painter order:
        // bottom flood, middle offset alpha, top SourceGraphic.
        Case::square(
            "filter-merge-three-layers",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="m"><feFlood flood-color="#11aa55" result="bg"/><feOffset in="SourceAlpha" dx="-6" dy="-6" result="halo"/><feMerge><feMergeNode in="bg"/><feMergeNode in="halo"/><feMergeNode in="SourceGraphic"/></feMerge></filter><rect x="24" y="24" width="44" height="44" fill="#c14b2b" filter="url(#m)"/></svg>"##,
        ),
        // feBlend mode="multiply": top flood (orange) multiplied with the
        // source's saturated red. Multiply darkens the overlap.
        Case::square(
            "filter-blend-multiply",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="b"><feFlood flood-color="#f2c14e" result="top"/><feBlend in="top" in2="SourceGraphic" mode="multiply"/></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#b)"/></svg>"##,
        ),
        // feBlend mode="screen" lightens the overlap of two mid-grey
        // sources.
        Case::square(
            "filter-blend-screen",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="b"><feFlood flood-color="#808080" result="top"/><feBlend in="top" in2="SourceGraphic" mode="screen"/></filter><rect x="20" y="20" width="60" height="60" fill="#808080" filter="url(#b)"/></svg>"##,
        ),
        // feComposite operator="in": keep the flood only where SourceAlpha
        // has coverage — the rect's footprint, with the flood paint.
        Case::square(
            "filter-composite-in",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="c"><feFlood flood-color="#2563eb" result="flood"/><feComposite in="flood" in2="SourceAlpha" operator="in"/></filter><rect x="22" y="22" width="56" height="56" fill="white" filter="url(#c)"/></svg>"##,
        ),
        // feComposite operator="out": keep the flood outside SourceAlpha's
        // footprint. The rect's interior is "subtracted" out of the flood.
        Case::square(
            "filter-composite-out",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="c"><feFlood flood-color="#c14b2b" result="flood"/><feComposite in="flood" in2="SourceAlpha" operator="out"/></filter><rect x="30" y="30" width="40" height="40" fill="white" filter="url(#c)"/></svg>"##,
        ),
        // feComposite arithmetic with k1=0, k2=1, k3=1, k4=0 adds the two
        // grey inputs together (operates in linear-light, so the encoded
        // pixel is brighter than either input alone).
        Case::square(
            "filter-composite-arithmetic-add",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="c"><feFlood flood-color="#404040" result="a"/><feFlood flood-color="#404040" result="b"/><feComposite in="a" in2="b" operator="arithmetic" k1="0" k2="1" k3="1" k4="0"/></filter><rect width="100%" height="100%" filter="url(#c)"/></svg>"##,
        ),
        // feTile spreads a small SourceGraphic across the entire filter
        // region — the small blue rect becomes a uniform blue field
        // expanded out to the filter region (= source bbox inflated 10%).
        Case::square(
            "filter-tile-spreads-source",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="t"><feTile/></filter><rect x="40" y="40" width="20" height="20" fill="#2563eb" filter="url(#t)"/></svg>"##,
        ),
        // SVG 1.1 §15.4: an empty `<filter>` renders the element as
        // transparent black. Without the spec-correct fallback this rect
        // would paint red into the navy background.
        Case::square(
            "filter-empty-transparent-black",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="empty"></filter><rect x="20" y="20" width="60" height="60" fill="#c14b2b" filter="url(#empty)"/></svg>"##,
        ),
        // feConvolveMatrix edgeMode="none": a normalised 3×3 box-blur of a
        // canvas-filling white rect darkens at the canvas edges because
        // out-of-bounds taps contribute zero.
        Case::square(
            "filter-convolve-edgemode-none",
            r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="1 1 1 1 1 1 1 1 1" divisor="9" edgeMode="none"/></filter><rect width="100" height="100" fill="white" filter="url(#c)"/></svg>"##,
        ),
        // feConvolveMatrix edgeMode="wrap": out-of-bounds taps wrap to the
        // opposite side, so a half-and-half source averages along the seam.
        Case::square(
            "filter-convolve-edgemode-wrap",
            r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="1 1 1 1 1 1 1 1 1" divisor="9" edgeMode="wrap"/></filter><rect x="0" y="0" width="50" height="100" fill="#c14b2b" filter="url(#c)"/><rect x="50" y="0" width="50" height="100" fill="#2563eb" filter="url(#c)"/></svg>"##,
        ),
        // feImage preserveAspectRatio default (`xMidYMid meet`): a
        // wide image source centred inside a square draw rect with
        // transparent top/bottom margins.
        Case::square(
            "filter-image-aspect-default-meet",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{wide}" x="20" y="20" width="60" height="60"/></filter><rect width="100" height="100" filter="url(#tex)"/></svg>"##,
                wide = wide_image,
            ),
        ),
        // Same source + draw rect, but `preserveAspectRatio="none"` —
        // image stretches to fully fill the rect.
        Case::square(
            "filter-image-aspect-none-stretch",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{wide}" x="20" y="20" width="60" height="60" preserveAspectRatio="none"/></filter><rect width="100" height="100" filter="url(#tex)"/></svg>"##,
                wide = wide_image,
            ),
        ),
        // `preserveAspectRatio="xMinYMax slice"`: the image covers the
        // draw rect, anchored bottom-left. The unused content overflows
        // into the (texture-clipped) right/top.
        Case::square(
            "filter-image-aspect-slice-bottom-left",
            format!(
                r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="tex"><feImage href="{wide}" x="20" y="20" width="60" height="60" preserveAspectRatio="xMinYMax slice"/></filter><rect width="100" height="100" filter="url(#tex)"/></svg>"##,
                wide = wide_image,
            ),
        ),
        // Filter `href` inheritance: `child` carries no primitives of its
        // own, so it inherits `parent`'s colour-matrix and paints the rect
        // red.
        Case::square(
            "filter-href-inheritance",
            r##"<svg><filter id="parent"><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0"/></filter><filter id="child" href="#parent"/><rect width="100%" height="100%" fill="#13294b"/><rect x="20" y="20" width="60" height="60" fill="white" filter="url(#child)"/></svg>"##,
        ),
        // Circular filter `href`: `a → b → a`. Cycle detection returns
        // transparent black; the navy background remains visible.
        Case::square(
            "filter-href-cycle-transparent",
            r##"<svg><filter id="a" href="#b"/><filter id="b" href="#a"/><rect width="100%" height="100%" fill="#13294b"/><rect x="20" y="20" width="60" height="60" fill="white" filter="url(#a)"/></svg>"##,
        ),
        // `currentColor` on `flood-color`: the flood inherits `color="red"`
        // from the filtered element, then `composite-in` against
        // SourceAlpha keeps it where the rect is.
        Case::square(
            "filter-current-color-flood",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="f"><feFlood flood-color="currentColor" result="flood"/><feComposite in="flood" in2="SourceAlpha" operator="in"/></filter><rect color="#c14b2b" x="22" y="22" width="56" height="56" fill="white" filter="url(#f)"/></svg>"##,
        ),
        // Per-primitive subregion (SVG 1.1 §15.5): the `<feFlood>` carries
        // its own `x/y/width/height`, so the flood is clipped to that rect
        // and the rest of the filter region stays transparent.
        Case::square(
            "filter-primitive-subregion",
            r##"<svg><rect width="100%" height="100%" fill="#13294b"/><filter id="f" x="0" y="0" width="100" height="100"><feFlood flood-color="#11aa55" x="30" y="30" width="40" height="40"/></filter><rect width="100" height="100" fill="white" filter="url(#f)"/></svg>"##,
        ),
        // SVG 2 render order: `clip-path` applies BEFORE the filter. The
        // rect is clipped to the inner clip rect, then the color-matrix
        // filter turns the surviving region red — outside the clip stays
        // navy because the source was already discarded.
        Case::square(
            "filter-clip-path-before-filter",
            r##"<svg><defs><clipPath id="c"><rect x="30" y="30" width="40" height="40"/></clipPath><filter id="f"><feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0"/></filter></defs><rect width="100%" height="100%" fill="#13294b"/><rect x="10" y="10" width="80" height="80" fill="white" clip-path="url(#c)" filter="url(#f)"/></svg>"##,
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
        // svg3's 3D `<cube>` primitive (SPEC §5.2). Through the default
        // orthographic projection the cube collapses to its axis-aligned
        // bounding rectangle: same coverage as a same-sized `<rect>`.
        Case::square(
            "cube-orthographic",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><cube cx="50" cy="50" cz="0" size="50" fill="#f2c14e"/></svg>"##,
        ),
        // A non-cubic cuboid: explicit `width`/`height`/`depth` produce a
        // rectangular box. Through the orthographic default it projects to
        // its width-by-height rectangle.
        Case::square(
            "cube-non-cubic",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><cube cx="50" cy="50" cz="0" width="70" height="30" depth="20" fill="#c14b2b"/></svg>"##,
        ),
        // Through the camera the cube's depth dimension is visible. The
        // background `<rect>` sits at world `z = 0` and (per SPEC §7.3)
        // occludes any cube surface behind it; the cube here is centred on
        // `z = 0` so only the front half (`z > 0`) of its hexagonal
        // silhouette survives the depth test.
        Case::square(
            "cube-camera-angled",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><cube cx="50" cy="50" cz="0" size="40" fill="#f2c14e"/></svg>"##,
        )
        .with_camera({
            // The same `angled` framing as the 2D camera snapshots, so the
            // cube's silhouette is comparable to the 2D scene at the same eye.
            let mut c = Camera::facing(100, 100);
            c.eye.x += 60.0;
            c
        }),
        // SPEC §7.3: spatial Z occlusion between 2D and 3D content. The 2D
        // `<rect>` (in the plane `z = 0`) hides the cube where the cube is
        // behind it (cz = -20, size = 30 → cube z in [-35, -5], entirely
        // behind the rect plane). Without depth-aware rendering the cube
        // would paint over the rect per document order; with it, the rect
        // wins at every overlap pixel.
        Case::square(
            "cube-occluded-by-2d-rect",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><rect x="30" y="30" width="40" height="40" fill="#c14b2b"/><cube cx="50" cy="50" cz="-20" size="30" fill="#f2c14e"/></svg>"##,
        ),
        // The converse: a cube fully in *front* of `z = 0` (cz = +20)
        // paints over a coplanar 2D rect declared earlier as the
        // background — closer-to-viewer Z wins.
        Case::square(
            "cube-in-front-of-2d-rect",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><rect x="30" y="30" width="40" height="40" fill="#c14b2b"/><cube cx="50" cy="50" cz="20" size="30" fill="#f2c14e"/></svg>"##,
        ),
        // 2D-3D depth resolution survives a filter chain. The filtered
        // rect sits at `z = 0`; a cube declared after it that lies fully
        // behind `z = 0` (cz = -20) is occluded by the filter result. The
        // composite shader writes the source's per-pixel NDC depth as
        // `frag_depth`, so subsequent 3D draws depth-test against the
        // filter's spatial Z — addressing review P1.2.
        Case::square(
            "cube-occluded-by-filtered-rect",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><filter id="soft"><feGaussianBlur stdDeviation="3"/></filter><rect x="30" y="30" width="40" height="40" fill="#c14b2b" filter="url(#soft)"/><cube cx="50" cy="50" cz="-20" size="30" fill="#f2c14e"/></svg>"##,
        ),
        // SPEC §5.3 `<ellipsoid>` through the orthographic default. With
        // `rx = ry = rz`, the ellipsoid collapses to its bounding disc; an
        // anisotropic `rx ≠ ry` case follows so the per-axis radii are
        // visible in the silhouette.
        Case::square(
            "ellipsoid-orthographic",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><ellipsoid cx="50" cy="50" cz="0" r="30" fill="#f2c14e"/></svg>"##,
        ),
        Case::square(
            "ellipsoid-anisotropic",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><ellipsoid cx="50" cy="50" cz="0" rx="40" ry="20" rz="20" fill="#c14b2b"/></svg>"##,
        ),
        // Through the camera the ellipsoid's depth is visible: the
        // background `<rect>` at `z = 0` (per SPEC §7.3) occludes any
        // ellipsoid surface behind it, so the off-axis camera reveals
        // only the front hemisphere.
        Case::square(
            "ellipsoid-camera-angled",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><ellipsoid cx="50" cy="50" cz="0" r="25" fill="#f2c14e"/></svg>"##,
        )
        .with_camera({
            let mut c = Camera::facing(100, 100);
            c.eye.x += 60.0;
            c
        }),
        // 2D-3D depth occlusion with `<ellipsoid>`. The ellipsoid lies
        // fully behind the rect plane (cz = -25, r = 20 → z in [-45, -5]),
        // so the rect at z = 0 hides it completely at the overlap.
        Case::square(
            "ellipsoid-occluded-by-2d-rect",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><rect x="30" y="30" width="40" height="40" fill="#c14b2b"/><ellipsoid cx="50" cy="50" cz="-25" r="20" fill="#f2c14e"/></svg>"##,
        ),
        // Converse: an ellipsoid in front of `z = 0` (cz = +25) paints
        // over a coplanar 2D rect declared earlier as the background.
        Case::square(
            "ellipsoid-in-front-of-2d-rect",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><rect x="30" y="30" width="40" height="40" fill="#c14b2b"/><ellipsoid cx="50" cy="50" cz="25" r="20" fill="#f2c14e"/></svg>"##,
        ),
        // SPEC §5.5 `<cylinder>` through the orthographic default. With
        // `r` and no explicit `depth`, the cap radii are `r` and depth is
        // the diameter, so the silhouette is the front circular cap.
        Case::square(
            "cylinder-orthographic",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><cylinder cx="50" cy="50" cz="0" r="28" fill="#f2c14e"/></svg>"##,
        ),
        // Explicit `rx`/`ry`/`depth` produce an elliptical cylinder. The
        // orthographic default shows the elliptical cap and validates the
        // independent cap radii.
        Case::square(
            "cylinder-anisotropic",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><cylinder cx="50" cy="50" cz="0" rx="38" ry="18" depth="34" fill="#c14b2b"/></svg>"##,
        ),
        // Through the camera the side wall becomes visible. The background
        // rect at `z = 0` occludes the back half, leaving the front cap and
        // front side-wall silhouette.
        Case::square(
            "cylinder-camera-angled",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><cylinder cx="50" cy="50" cz="0" r="24" depth="42" fill="#f2c14e"/></svg>"##,
        )
        .with_camera({
            let mut c = Camera::facing(100, 100);
            c.eye.x += 60.0;
            c
        }),
        // 2D-3D depth occlusion with `<cylinder>`. The cylinder lies fully
        // behind the rect plane (cz = -30, depth = 40 → z in [-50, -10]),
        // so the rect at z = 0 hides it completely at the overlap.
        Case::square(
            "cylinder-occluded-by-2d-rect",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><rect x="30" y="30" width="40" height="40" fill="#c14b2b"/><cylinder cx="50" cy="50" cz="-30" r="20" depth="40" fill="#f2c14e"/></svg>"##,
        ),
        // SPEC §5.4 `<surface>` — degree-1 Bezier between two parallel
        // child paths produces a ruled quad strip. Sweeping a horizontal
        // line at y=30 to one at y=70 fills a rectangle in the
        // orthographic projection, confirming that the new dispatch
        // integrates with the depth pipeline.
        Case::square(
            "surface-ruled-flat",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><surface d="M 0 L 1" fill="#f2c14e"><path d="M 20 30 L 80 30"/><path d="M 20 30 L 80 30" transform="translate(0, 40)"/></surface></svg>"##,
        ),
        // SPEC §5.4 — degree-3 Bezier surface through 4 child paths,
        // viewed through the angled-perspective camera. Paths 1 and 2
        // are control rows (not on the surface). Mirrors
        // `ellipsoid-camera-angled` framing for visual consistency.
        Case::square(
            "surface-cubic-camera-angled",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><surface d="M 0 C 1 2 3" fill="#f2c14e"><path d="M 30 30 L 70 30 L 70 70 L 30 70"/><path d="M 30 30 L 70 30 L 70 70 L 30 70" transform="translate3d(15, 0, 20)"/><path d="M 30 30 L 70 30 L 70 70 L 30 70" transform="translate3d(-15, 0, 40)"/><path d="M 30 30 L 70 30 L 70 70 L 30 70" transform="translateZ(60)"/></surface></svg>"##,
        )
        .with_camera({
            let mut c = Camera::facing(100, 100);
            c.eye.x += 60.0;
            c
        }),
        // SPEC §5.4 — chained C+L patches with a shared boundary row.
        // 4 cubic-control paths plus a final linear extension to a 5th
        // path; the join at path 3 has C0 continuity.
        Case::square(
            "surface-chained-patches",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><surface d="M 0 C 1 2 3 L 4" fill="#c14b2b"><path d="M 30 30 L 70 30 L 70 70 L 30 70"/><path d="M 30 30 L 70 30 L 70 70 L 30 70" transform="translate3d(10, -10, 15)"/><path d="M 30 30 L 70 30 L 70 70 L 30 70" transform="translate3d(-10, -10, 30)"/><path d="M 30 30 L 70 30 L 70 70 L 30 70" transform="translateZ(45)"/><path d="M 30 30 L 70 30 L 70 70 L 30 70" transform="translate3d(0, 20, 70)"/></surface></svg>"##,
        )
        .with_camera({
            let mut c = Camera::facing(100, 100);
            c.eye.x += 60.0;
            c
        }),
        // SPEC §5.4 — `Z` closes the sweep direction. Four parallel
        // cross-section paths placed at the four box corners form a
        // closed prism (a box around the y-axis, with the linear-
        // patch chain wrapping back to path 0).
        Case::square(
            "surface-closed-prism",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><surface d="M 0 L 1 L 2 L 3 Z" fill="#f2c14e"><path d="M 35 35 L 65 35"/><path d="M 35 35 L 65 35" transform="translateZ(30)"/><path d="M 35 35 L 65 35" transform="translate3d(0, 30, 30)"/><path d="M 35 35 L 65 35" transform="translate3d(0, 30, 0)"/></surface></svg>"##,
        )
        .with_camera({
            let mut c = Camera::facing(100, 100);
            c.eye.x += 60.0;
            c
        }),
        // SPEC §5.4 — a surface whose `d` references a non-existent
        // path index is in error and is not rendered. The sibling
        // cube must still appear, confirming an in-error surface
        // doesn't poison the rest of the scene.
        Case::square(
            "surface-degenerate-out-of-range",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><surface d="M 0 L 5" fill="#c14b2b"><path d="M 20 20 L 80 20"/><path d="M 20 20 L 80 20" transform="translateZ(40)"/></surface><cube cx="50" cy="50" cz="0" size="40" fill="#f2c14e"/></svg>"##,
        ),
        // SPEC §5.4 — a torus (donut) tiled by EIGHT bicubic Coons
        // patches: 4 quarter-arcs of the major ring × 2 half-arcs of
        // the tube cross-section. Each patch is bounded by 4 cubic
        // Bezier curves declared as child `<path>` elements:
        // - paths 0–3: outer-ring quarter-arcs (radius R+r = 33) in
        //   the z = 0 plane, with kappa-tangent control offsets
        //   κ·(R+r) ≈ 18.225 for the 4-cubic-arc circle approximation;
        // - paths 4–7: inner-ring quarter-arcs (radius R-r = 17),
        //   κ·(R-r) ≈ 9.389;
        // - paths 8–15: tube-cross-section half-arcs (radius r = 8)
        //   at the four θ values {0, 90°, 180°, 270°} × {front, back}
        //   of the tube, with cubic half-circle controls at offset
        //   4r/3 ≈ 10.667. Each tube-arc path is drawn in its
        //   2D-local frame and lifted into 3D by `rotateZ(θ)
        //   rotateX(90)` then `translate3d`.
        // The surface `d` lists 8 `P top right bottom left` commands
        // selecting the 4 boundary paths per tile. Adjacent tiles
        // share boundary paths (e.g., front-half tiles at θ ∈
        // [0,90°] and θ ∈ [90°,180°] share path 10 as the right and
        // left edges respectively), so the 8-tile surface uses only
        // 16 unique boundary curves. Coons blending fills each tile's
        // interior with C0 (positional) continuity across the shared
        // edges. Viewed through a slightly angled perspective camera
        // so the 3D ring depth is visible alongside the annulus.
        Case::square(
            "surface-donut",
            r##"<svg width="100" height="100"><rect width="100%" height="100%" fill="#13294b"/><surface fill="#f2c14e" d="P 0 10 4 8 P 1 12 5 10 P 2 14 6 12 P 3 8 7 14 P 4 11 0 9 P 5 13 1 11 P 6 15 2 13 P 7 9 3 15"><path d="M 83 50 C 83 68.225 68.225 83 50 83"/><path d="M 50 83 C 31.775 83 17 68.225 17 50"/><path d="M 17 50 C 17 31.775 31.775 17 50 17"/><path d="M 50 17 C 68.225 17 83 31.775 83 50"/><path d="M 67 50 C 67 59.389 59.389 67 50 67"/><path d="M 50 67 C 40.611 67 33 59.389 33 50"/><path d="M 33 50 C 33 40.611 40.611 33 50 33"/><path d="M 50 33 C 59.389 33 67 40.611 67 50"/><path d="M 8 0 C 8 10.667 -8 10.667 -8 0" transform="translate3d(75, 50, 0) rotateX(90)"/><path d="M -8 0 C -8 -10.667 8 -10.667 8 0" transform="translate3d(75, 50, 0) rotateX(90)"/><path d="M 8 0 C 8 10.667 -8 10.667 -8 0" transform="translate3d(50, 75, 0) rotateZ(90) rotateX(90)"/><path d="M -8 0 C -8 -10.667 8 -10.667 8 0" transform="translate3d(50, 75, 0) rotateZ(90) rotateX(90)"/><path d="M 8 0 C 8 10.667 -8 10.667 -8 0" transform="translate3d(25, 50, 0) rotateZ(180) rotateX(90)"/><path d="M -8 0 C -8 -10.667 8 -10.667 8 0" transform="translate3d(25, 50, 0) rotateZ(180) rotateX(90)"/><path d="M 8 0 C 8 10.667 -8 10.667 -8 0" transform="translate3d(50, 25, 0) rotateZ(270) rotateX(90)"/><path d="M -8 0 C -8 -10.667 8 -10.667 8 0" transform="translate3d(50, 25, 0) rotateZ(270) rotateX(90)"/></surface></svg>"##,
        )
        .with_camera({
            let mut c = Camera::facing(100, 100);
            c.eye.x += 30.0;
            c.eye.y -= 30.0;
            c
        }),
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
