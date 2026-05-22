//! SVG 1.1 `<line>` — geometry resolution and stroke tessellation.
//!
//! `<line>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<line>`
//! [`Element`] into a stroked quad in SVG user space, applying the SVG 1.1
//! geometry rules ([SVG11] §9.5) with the default butt line cap.
//!
//! Length parsing and stroke paint resolution are shared with the other
//! basic shapes — see [`crate::shape`]. `transform`, grouping, dashed
//! strokes, joins, caps other than the default butt cap, and CSS cascade
//! input are not handled yet — see the crate roadmap.

use svg3_dom::Element;

use crate::shape::{resolve_stroke_width, vertex, Length, Viewport};
use crate::Mesh;

/// A `<line>`'s geometry after SVG 1.1 defaulting. All values are in SVG
/// user units. Resolved geometry is non-degenerate so tessellation can
/// safely normalize the segment direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LineGeometry {
    /// Start x coordinate.
    pub x1: f32,
    /// Start y coordinate.
    pub y1: f32,
    /// End x coordinate.
    pub x2: f32,
    /// End y coordinate.
    pub y2: f32,
    /// Stroke width — always `> 0` for a resolved geometry.
    pub stroke_width: f32,
}

/// Resolve a `<line>`'s raw attributes into a [`LineGeometry`].
///
/// Percentage lengths resolve against `viewport` — `x1`/`x2` against its
/// width, `y1`/`y2` against its height, and `stroke-width` against its
/// normalized diagonal ([SVG11] §7.10). Coordinate attributes default to
/// `0`; `stroke-width` defaults to `1`.
///
/// Returns `None` when the line is not rendered: zero length, or a
/// non-positive `stroke-width`.
pub(crate) fn resolve_line(element: &Element, viewport: Viewport) -> Option<LineGeometry> {
    let length = |name: &str, basis: f32| {
        element
            .attributes
            .get(name)
            .map(String::as_str)
            .and_then(Length::parse)
            .map(|len| len.resolve(basis))
    };

    let x1 = length("x1", viewport.width).unwrap_or(0.0);
    let y1 = length("y1", viewport.height).unwrap_or(0.0);
    let x2 = length("x2", viewport.width).unwrap_or(0.0);
    let y2 = length("y2", viewport.height).unwrap_or(0.0);
    let stroke_width = resolve_stroke_width(element, viewport);

    if stroke_width <= 0.0 || (x1 == x2 && y1 == y2) {
        return None;
    }

    Some(LineGeometry {
        x1,
        y1,
        x2,
        y2,
        stroke_width,
    })
}

/// Tessellate a resolved line into a stroked quad [`Mesh`].
///
/// SVG's default `stroke-linecap` is `butt`, so the quad is not extended
/// beyond either endpoint. Positions are in SVG user space with `z = 0`.
pub(crate) fn tessellate_line(geo: &LineGeometry, color: [f32; 4]) -> Mesh {
    let dx = geo.x2 - geo.x1;
    let dy = geo.y2 - geo.y1;
    let length = dx.hypot(dy);
    let half_width = geo.stroke_width / 2.0;
    let nx = -dy / length * half_width;
    let ny = dx / length * half_width;

    // Order the quad to match the rect/circle winding in SVG user space.
    // The current pipeline disables culling, but keeping winding consistent
    // avoids direction-sensitive surprises if basic shapes share a culled
    // pipeline later.
    Mesh {
        vertices: vec![
            vertex(geo.x1 - nx, geo.y1 - ny, color),
            vertex(geo.x2 - nx, geo.y2 - ny, color),
            vertex(geo.x2 + nx, geo.y2 + ny, color),
            vertex(geo.x1 + nx, geo.y1 + ny, color),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    /// Build a `<line>` element carrying the given raw attributes.
    fn line(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Line);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    /// A 100×100 viewport. The geometry tests below use absolute lengths, so
    /// the viewport value only matters for the percentage case.
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn resolve_line_applies_geometry_defaults() {
        // x1/y1/x2/y2 default to 0 and stroke-width defaults to 1
        // ([SVG11] §9.5, §11.4).
        let geo = resolve_line(&line(&[("x2", "20"), ("y2", "10")]), vp()).unwrap();
        assert_eq!(
            geo,
            LineGeometry {
                x1: 0.0,
                y1: 0.0,
                x2: 20.0,
                y2: 10.0,
                stroke_width: 1.0,
            }
        );
    }

    #[test]
    fn resolve_line_resolves_percentage_geometry() {
        // x coordinates resolve against viewport width; y coordinates
        // against viewport height ([SVG11] §7.10).
        let viewport = Viewport {
            width: 200.0,
            height: 100.0,
        };
        let geo = resolve_line(
            &line(&[
                ("x1", "10%"),
                ("y1", "25%"),
                ("x2", "50%"),
                ("y2", "75%"),
                ("stroke-width", "4"),
            ]),
            viewport,
        )
        .unwrap();
        assert_eq!(
            geo,
            LineGeometry {
                x1: 20.0,
                y1: 25.0,
                x2: 100.0,
                y2: 75.0,
                stroke_width: 4.0,
            }
        );
    }

    #[test]
    fn resolve_line_resolves_percentage_stroke_width_against_diagonal() {
        // `stroke-width` is neither horizontal nor vertical, so percentages
        // resolve against the normalized viewport diagonal ([SVG11] §7.10).
        let viewport = Viewport {
            width: 300.0,
            height: 400.0,
        };
        let geo = resolve_line(&line(&[("x2", "100"), ("stroke-width", "10%")]), viewport).unwrap();
        assert_close(geo.stroke_width, 35.355_34);
    }

    #[test]
    fn resolve_line_skips_degenerate_geometry() {
        // Missing endpoints produce a zero-length segment, which the default
        // butt cap cannot render.
        assert_eq!(resolve_line(&line(&[]), vp()), None);
        assert_eq!(
            resolve_line(
                &line(&[("x1", "10"), ("y1", "10"), ("x2", "10"), ("y2", "10")]),
                vp(),
            ),
            None
        );
        assert_eq!(
            resolve_line(
                &line(&[
                    ("x1", "10"),
                    ("y1", "10"),
                    ("x2", "30"),
                    ("y2", "10"),
                    ("stroke-width", "0"),
                ]),
                vp(),
            ),
            None
        );
        assert_eq!(
            resolve_line(
                &line(&[
                    ("x1", "10"),
                    ("y1", "10"),
                    ("x2", "30"),
                    ("y2", "10"),
                    ("stroke-width", "-1"),
                ]),
                vp(),
            ),
            None
        );
    }

    #[test]
    fn tessellate_line_is_a_butt_cap_quad() {
        let geo = resolve_line(
            &line(&[
                ("x1", "10"),
                ("y1", "20"),
                ("x2", "50"),
                ("y2", "20"),
                ("stroke-width", "4"),
            ]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_line(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.vertices[0].position, [10.0, 18.0, 0.0]);
        assert_eq!(mesh.vertices[1].position, [50.0, 18.0, 0.0]);
        assert_eq!(mesh.vertices[2].position, [50.0, 22.0, 0.0]);
        assert_eq!(mesh.vertices[3].position, [10.0, 22.0, 0.0]);
        assert_eq!(mesh.vertices[0].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn tessellate_diagonal_line_offsets_both_axes() {
        let geo = resolve_line(
            &line(&[
                ("x1", "10"),
                ("y1", "20"),
                ("x2", "40"),
                ("y2", "60"),
                ("stroke-width", "10"),
            ]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_line(&geo, [1.0; 4]);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.position).collect();
        assert_positions_close(
            &positions,
            &[
                [14.0, 17.0, 0.0],
                [44.0, 57.0, 0.0],
                [36.0, 63.0, 0.0],
                [6.0, 23.0, 0.0],
            ],
        );
    }

    #[test]
    fn tessellate_diagonal_line_keeps_winding_when_reversed() {
        let forward = resolve_line(
            &line(&[
                ("x1", "10"),
                ("y1", "20"),
                ("x2", "40"),
                ("y2", "60"),
                ("stroke-width", "10"),
            ]),
            vp(),
        )
        .unwrap();
        let reverse = LineGeometry {
            x1: forward.x2,
            y1: forward.y2,
            x2: forward.x1,
            y2: forward.y1,
            stroke_width: forward.stroke_width,
        };

        for mesh in [
            tessellate_line(&forward, [1.0; 4]),
            tessellate_line(&reverse, [1.0; 4]),
        ] {
            for triangle in mesh.indices.chunks_exact(3) {
                let a = mesh.vertices[triangle[0] as usize].position;
                let b = mesh.vertices[triangle[1] as usize].position;
                let c = mesh.vertices[triangle[2] as usize].position;
                assert!(
                    signed_twice_area(a, b, c) > 0.0,
                    "line triangle should keep rect/circle winding"
                );
            }
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_positions_close(actual: &[[f32; 3]], expected: &[[f32; 3]]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "expected {expected}, got {actual}"
                );
            }
        }
    }

    fn signed_twice_area(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }
}
