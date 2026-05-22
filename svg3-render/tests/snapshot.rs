//! End-to-end snapshot tests for `<rect>`, `<circle>`, `<ellipse>` and
//! `<polygon>` rendering.
//!
//! Each case parses an svg3 document — the SVG WPT `shapes/rect-*`,
//! `shapes/circle-*`, `shapes/ellipse-*` and `shapes/polygon-*` reference
//! tests, plus the canonical SVG sample — renders it headlessly with
//! [`Renderer::render_to_image`], and compares the result against a
//! committed golden PNG in `tests/snapshots/`. Those PNGs are the
//! reviewable snapshots — open them in a pull request to see what the
//! renderer produces.
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

use std::path::{Path, PathBuf};

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
    svg: &'static str,
    /// Render-target width, in pixels.
    width: u32,
    /// Render-target height, in pixels.
    height: u32,
    /// Optional 3D camera; `None` renders 2D content flat.
    camera: Option<Camera>,
}

impl Case {
    /// A case rendered into a square `CANVAS`×`CANVAS` target.
    const fn square(name: &'static str, svg: &'static str) -> Self {
        Self {
            name,
            svg,
            width: CANVAS,
            height: CANVAS,
            camera: None,
        }
    }

    /// A case rendered into a `width`×`height` target.
    const fn sized(name: &'static str, svg: &'static str, width: u32, height: u32) -> Self {
        Self {
            name,
            svg,
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
    let renderer = Renderer::new();
    let update = std::env::var_os("SVG3_UPDATE_SNAPSHOTS").is_some();

    let mut updated: Vec<&str> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for case in cases() {
        let document = parse(case.svg).expect("snapshot fixture should parse");
        let config = RenderConfig {
            width: case.width,
            height: case.height,
            camera: case.camera,
            ..RenderConfig::default()
        };
        let image = match renderer.render_to_image(&document, config) {
            Ok(image) => image,
            Err(RenderError::NoAdapter) => {
                eprintln!("skipping snapshot tests: no GPU adapter available");
                return;
            }
            Err(error) => panic!("render failed for `{}`: {error}", case.name),
        };

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
    // A small budget absorbs rounded-corner edge pixels a GPU may rasterise
    // a hair differently; a real regression shifts far more than this.
    let budget = (image.width * image.height) as usize / 200;
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
