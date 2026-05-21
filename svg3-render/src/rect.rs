//! SVG 1.1 `<rect>` — geometry resolution and fill tessellation.
//!
//! `<rect>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<rect>`
//! [`Element`] into a filled triangle [`Mesh`] in SVG user space, applying
//! the SVG 1.1 geometry rules ([SVG11] §9.2). The behavioural reference is
//! the SVG WPT suite (`svg/shapes/rect-0*.svg`).
//!
//! Stroke, `fill-opacity`, CSS / `style=""`-set properties, and `transform`
//! are not handled yet — see the crate roadmap.

use std::f32::consts::{FRAC_PI_2, PI};

use svg3_dom::Element;

use crate::{Mesh, Vertex};

/// Segments approximating each rounded corner's quarter-arc. Fixed so a
/// rounded rect's vertex count is deterministic.
const CORNER_SEGMENTS: usize = 8;

/// The SVG 1.1 initial `fill` value — opaque black — in linear RGBA.
const DEFAULT_FILL: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// A `<rect>`'s geometry after SVG 1.1 defaulting and corner-radius
/// clamping. All values are in SVG user units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RectGeometry {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width — always `> 0` for a resolved geometry.
    pub width: f32,
    /// Height — always `> 0` for a resolved geometry.
    pub height: f32,
    /// Horizontal corner radius, clamped to `width / 2`; `0` is sharp.
    pub rx: f32,
    /// Vertical corner radius, clamped to `height / 2`; `0` is sharp.
    pub ry: f32,
}

/// Resolve a `<rect>`'s raw attributes into a [`RectGeometry`].
///
/// Returns `None` when the rectangle is not rendered — a missing, zero, or
/// negative `width`/`height` ([SVG11] §9.2; WPT `shapes/rect-05`).
///
/// `x`/`y` default to `0`. Corner radii follow the `rx`/`ry` "auto" rules:
/// if only one is given the other mirrors it; if neither is given both are
/// `0`; a negative radius is treated as auto. Each radius is then clamped
/// to half its side, independently (WPT `import/shapes-rect-06`).
pub(crate) fn resolve_rect(element: &Element) -> Option<RectGeometry> {
    let length = |name: &str| {
        element
            .attributes
            .get(name)
            .map(String::as_str)
            .and_then(parse_length)
    };

    let x = length("x").unwrap_or(0.0);
    let y = length("y").unwrap_or(0.0);
    let width = length("width").unwrap_or(0.0);
    let height = length("height").unwrap_or(0.0);

    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    // A negative or unparseable radius is "auto"; an auto axis mirrors the
    // other; if both are auto the corners are sharp.
    let rx_attr = length("rx").filter(|v| *v >= 0.0);
    let ry_attr = length("ry").filter(|v| *v >= 0.0);
    let (rx, ry) = match (rx_attr, ry_attr) {
        (Some(rx), Some(ry)) => (rx, ry),
        (Some(rx), None) => (rx, rx),
        (None, Some(ry)) => (ry, ry),
        (None, None) => (0.0, 0.0),
    };

    Some(RectGeometry {
        x,
        y,
        width,
        height,
        rx: rx.min(width / 2.0),
        ry: ry.min(height / 2.0),
    })
}

/// Resolve a `<rect>`'s solid fill as linear RGBA in `[0, 1]`.
///
/// Reads the `fill` presentation attribute only — the CSS `style=""` form
/// and the Stylo cascade are not consulted yet. `fill="none"` yields
/// `None` (no fill geometry). A missing or unparseable value falls back to
/// the SVG 1.1 initial value, opaque black.
pub(crate) fn resolve_fill(element: &Element) -> Option<[f32; 4]> {
    let Some(value) = element.attributes.get("fill") else {
        return Some(DEFAULT_FILL);
    };
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return None;
    }
    Some(parse_color(value).unwrap_or(DEFAULT_FILL))
}

/// Tessellate a resolved rectangle into a filled triangle [`Mesh`].
///
/// A sharp rectangle becomes two triangles (4 vertices). A rounded
/// rectangle is a triangle fan from its centroid over a clockwise outline
/// of `4 * (CORNER_SEGMENTS + 1)` perimeter points. Positions are in SVG
/// user space with `z = 0`.
pub(crate) fn tessellate_rect(geo: &RectGeometry, color: [f32; 4]) -> Mesh {
    if geo.rx == 0.0 || geo.ry == 0.0 {
        sharp_mesh(geo, color)
    } else {
        rounded_mesh(geo, color)
    }
}

fn sharp_mesh(geo: &RectGeometry, color: [f32; 4]) -> Mesh {
    Mesh {
        vertices: vec![
            vertex(geo.x, geo.y, color),
            vertex(geo.x + geo.width, geo.y, color),
            vertex(geo.x + geo.width, geo.y + geo.height, color),
            vertex(geo.x, geo.y + geo.height, color),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn rounded_mesh(geo: &RectGeometry, color: [f32; 4]) -> Mesh {
    let perimeter = rounded_outline(geo);
    let n = perimeter.len() as u32;

    // Vertex 0 is the centroid; the fan pivots on it.
    let mut vertices = Vec::with_capacity(perimeter.len() + 1);
    vertices.push(vertex(
        geo.x + geo.width / 2.0,
        geo.y + geo.height / 2.0,
        color,
    ));
    vertices.extend(perimeter.into_iter().map(|(px, py)| vertex(px, py, color)));

    let mut indices = Vec::with_capacity(n as usize * 3);
    for i in 0..n {
        indices.extend_from_slice(&[0, i + 1, (i + 1) % n + 1]);
    }
    Mesh { vertices, indices }
}

/// Clockwise outline of a rounded rectangle: four quarter-arcs of
/// `CORNER_SEGMENTS + 1` points each, joined by the straight edges
/// (implicit chords between consecutive arcs).
fn rounded_outline(geo: &RectGeometry) -> Vec<(f32, f32)> {
    let RectGeometry {
        x,
        y,
        width,
        height,
        rx,
        ry,
    } = *geo;
    // (centre_x, centre_y, start_angle), clockwise from the top-right
    // corner; each arc sweeps a quarter-turn. The ellipse sample at angle
    // `t` is `(cx + rx*cos t, cy + ry*sin t)` in the y-down user space.
    let corners = [
        (x + width - rx, y + ry, -FRAC_PI_2),
        (x + width - rx, y + height - ry, 0.0),
        (x + rx, y + height - ry, FRAC_PI_2),
        (x + rx, y + ry, PI),
    ];
    let mut points = Vec::with_capacity(4 * (CORNER_SEGMENTS + 1));
    for (cx, cy, start) in corners {
        for step in 0..=CORNER_SEGMENTS {
            let t = start + FRAC_PI_2 * (step as f32 / CORNER_SEGMENTS as f32);
            points.push((cx + rx * t.cos(), cy + ry * t.sin()));
        }
    }
    points
}

fn vertex(x: f32, y: f32, color: [f32; 4]) -> Vertex {
    Vertex {
        position: [x, y, 0.0],
        color,
    }
}

/// Parse an SVG length value into user units. Accepts a plain number or a
/// `px`-suffixed number (1px = 1 user unit). Percentages and other units
/// are not handled yet and yield `None`.
fn parse_length(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let number = trimmed.strip_suffix("px").unwrap_or(trimmed).trim();
    let parsed: f32 = number.parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

/// Parse an sRGB colour — `#rgb`, `#rrggbb`, or a named colour — into
/// linear RGBA. Returns `None` for an unrecognised value.
fn parse_color(value: &str) -> Option<[f32; 4]> {
    match value.strip_prefix('#') {
        Some(hex) => parse_hex(hex),
        None => parse_named(value),
    }
}

fn parse_hex(hex: &str) -> Option<[f32; 4]> {
    if !hex.is_ascii() {
        return None;
    }
    let (r, g, b) = match hex.as_bytes() {
        // `#rgb` shorthand: each nibble is doubled (`0xN` -> `0xNN`).
        [r, g, b] => (nibble(*r)? * 17, nibble(*g)? * 17, nibble(*b)? * 17),
        [r0, r1, g0, g1, b0, b1] => (byte(*r0, *r1)?, byte(*g0, *g1)?, byte(*b0, *b1)?),
        _ => return None,
    };
    Some([
        srgb_to_linear(r as f32 / 255.0),
        srgb_to_linear(g as f32 / 255.0),
        srgb_to_linear(b as f32 / 255.0),
        1.0,
    ])
}

fn nibble(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

fn byte(hi: u8, lo: u8) -> Option<u8> {
    Some(nibble(hi)? * 16 + nibble(lo)?)
}

/// A small subset of the SVG named colours — enough for the WPT `<rect>`
/// tests and common authoring.
fn parse_named(name: &str) -> Option<[f32; 4]> {
    let hex = match name.to_ascii_lowercase().as_str() {
        "black" => "000000",
        "white" => "ffffff",
        "red" => "ff0000",
        "green" => "008000",
        "blue" => "0000ff",
        "yellow" => "ffff00",
        "cyan" | "aqua" => "00ffff",
        "magenta" | "fuchsia" => "ff00ff",
        "gray" | "grey" => "808080",
        "silver" => "c0c0c0",
        "maroon" => "800000",
        "navy" => "000080",
        "orange" => "ffa500",
        "purple" => "800080",
        "lime" => "00ff00",
        "teal" => "008080",
        _ => return None,
    };
    parse_hex(hex)
}

/// Convert one sRGB channel in `[0, 1]` to linear-light.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    /// Build a `<rect>` element carrying the given raw attributes.
    fn rect(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Rect);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    #[test]
    fn parse_length_accepts_numbers_and_px() {
        assert_eq!(parse_length("50"), Some(50.0));
        assert_eq!(parse_length("50px"), Some(50.0));
        assert_eq!(parse_length("  12  "), Some(12.0));
        assert_eq!(parse_length("1e2"), Some(100.0));
        assert_eq!(parse_length("-3"), Some(-3.0));
    }

    #[test]
    fn parse_length_rejects_unsupported_values() {
        assert_eq!(parse_length(""), None);
        assert_eq!(parse_length("abc"), None);
        assert_eq!(parse_length("50%"), None);
        assert_eq!(parse_length("inf"), None);
    }

    #[test]
    fn resolve_rect_applies_position_defaults() {
        // `x`/`y` default to 0 ([SVG11] §9.2).
        let geo = resolve_rect(&rect(&[("width", "40"), ("height", "20")])).unwrap();
        assert_eq!(
            geo,
            RectGeometry {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 20.0,
                rx: 0.0,
                ry: 0.0,
            }
        );
    }

    #[test]
    fn resolve_rect_skips_degenerate_sizes() {
        // Missing, zero, or negative width/height => not rendered
        // (WPT `shapes/rect-05`).
        assert_eq!(resolve_rect(&rect(&[("height", "10")])), None);
        assert_eq!(
            resolve_rect(&rect(&[("width", "0"), ("height", "10")])),
            None
        );
        assert_eq!(
            resolve_rect(&rect(&[("width", "10"), ("height", "0")])),
            None
        );
        assert_eq!(
            resolve_rect(&rect(&[("width", "-5"), ("height", "10")])),
            None
        );
        assert_eq!(
            resolve_rect(&rect(&[("width", "10"), ("height", "-5")])),
            None
        );
    }

    #[test]
    fn resolve_rect_aliases_auto_corner_radii() {
        let radii = |extra: &[(&str, &str)]| {
            let mut attrs = vec![("width", "100"), ("height", "100")];
            attrs.extend_from_slice(extra);
            let g = resolve_rect(&rect(&attrs)).unwrap();
            (g.rx, g.ry)
        };
        // Only `rx` given => `ry` mirrors it; only `ry` => `rx` mirrors it.
        assert_eq!(radii(&[("rx", "12")]), (12.0, 12.0));
        assert_eq!(radii(&[("ry", "9")]), (9.0, 9.0));
        // Neither => sharp; a negative radius is treated as auto.
        assert_eq!(radii(&[]), (0.0, 0.0));
        assert_eq!(radii(&[("rx", "-4")]), (0.0, 0.0));
    }

    #[test]
    fn resolve_rect_clamps_radii_to_half_side() {
        // rx/ry over half the side are clamped, independently per axis
        // (WPT `import/shapes-rect-06`).
        let geo = resolve_rect(&rect(&[
            ("width", "20"),
            ("height", "100"),
            ("rx", "50"),
            ("ry", "20"),
        ]))
        .unwrap();
        assert_eq!((geo.rx, geo.ry), (10.0, 20.0));
    }

    #[test]
    fn resolve_fill_reads_presentation_attribute() {
        // Missing `fill` => SVG 1.1 initial value, opaque black.
        assert_eq!(resolve_fill(&rect(&[])), Some([0.0, 0.0, 0.0, 1.0]));
        // `none` => no fill geometry.
        assert_eq!(resolve_fill(&rect(&[("fill", "none")])), None);
        // Named + hex forms of pure blue resolve identically.
        assert_eq!(
            resolve_fill(&rect(&[("fill", "blue")])),
            Some([0.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            resolve_fill(&rect(&[("fill", "#0000ff")])),
            Some([0.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            resolve_fill(&rect(&[("fill", "#00f")])),
            Some([0.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            resolve_fill(&rect(&[("fill", "#ff0000")])),
            Some([1.0, 0.0, 0.0, 1.0])
        );
        // An unparseable colour falls back to black.
        assert_eq!(
            resolve_fill(&rect(&[("fill", "bogus")])),
            Some([0.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn tessellate_sharp_rect_is_two_triangles() {
        // WPT `shapes/rect-01`: <rect x=10 y=10 width=50 height=50>.
        let geo = resolve_rect(&rect(&[
            ("x", "10"),
            ("y", "10"),
            ("width", "50"),
            ("height", "50"),
        ]))
        .unwrap();
        let mesh = tessellate_rect(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.vertices[0].position, [10.0, 10.0, 0.0]);
        assert_eq!(mesh.vertices[2].position, [60.0, 60.0, 0.0]);
        assert_eq!(mesh.vertices[0].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn tessellate_treats_zero_radius_as_sharp() {
        // An explicit `ry=0` disables rounding even with `rx>0`.
        let geo = resolve_rect(&rect(&[
            ("width", "40"),
            ("height", "40"),
            ("rx", "10"),
            ("ry", "0"),
        ]))
        .unwrap();
        assert_eq!(tessellate_rect(&geo, [1.0; 4]).vertices.len(), 4);
    }

    #[test]
    fn tessellate_rounded_rect_fans_within_bounds() {
        // WPT `shapes/rect-03`: <rect x=10 y=10 width=50 height=50 rx=8 ry=8>.
        let geo = resolve_rect(&rect(&[
            ("x", "10"),
            ("y", "10"),
            ("width", "50"),
            ("height", "50"),
            ("rx", "8"),
            ("ry", "8"),
        ]))
        .unwrap();
        let mesh = tessellate_rect(&geo, [1.0; 4]);
        // Centroid + four quarter-arcs, one fan triangle per perimeter edge.
        let perimeter = 4 * (CORNER_SEGMENTS + 1);
        assert_eq!(mesh.vertices.len(), perimeter + 1);
        assert_eq!(mesh.indices.len(), perimeter * 3);
        // The fan pivot is the rect centroid.
        assert_eq!(mesh.vertices[0].position, [35.0, 35.0, 0.0]);
        // Every vertex stays inside the rect's bounding box.
        for v in &mesh.vertices {
            assert!(v.position[0] >= 10.0 - 1e-3 && v.position[0] <= 60.0 + 1e-3);
            assert!(v.position[1] >= 10.0 - 1e-3 && v.position[1] <= 60.0 + 1e-3);
        }
    }
}
