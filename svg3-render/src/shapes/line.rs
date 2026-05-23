//! SVG 1.1 `<line>` — geometry resolution and stroke tessellation.
//!
//! `<line>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<line>`
//! [`Element`] into stroked triangle geometry in SVG user space, applying the
//! SVG 1.1 geometry rules ([SVG11] §9.5).
//!
//! Length parsing and stroke paint resolution are shared with the other
//! basic shapes — see [`crate::shapes`]. `transform`, grouping, and CSS
//! cascade input are not handled yet — see the crate roadmap.

use lyon_tessellation::path::Path;
use svg3_dom::Element;

use super::stroke::{self, StrokeStyle};
use super::{resolve_length, Viewport};
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
}

/// Resolve a `<line>`'s raw attributes into a [`LineGeometry`].
///
/// Percentage lengths resolve against `viewport` — `x1`/`x2` against its
/// width and `y1`/`y2` against its height ([SVG11] §7.10). Coordinate
/// attributes default to `0`.
///
/// Returns `None` when the line is not rendered: zero length.
pub(crate) fn resolve_line(element: &Element, viewport: Viewport) -> Option<LineGeometry> {
    let x1 = resolve_length(element, "x1", viewport.width).unwrap_or(0.0);
    let y1 = resolve_length(element, "y1", viewport.height).unwrap_or(0.0);
    let x2 = resolve_length(element, "x2", viewport.width).unwrap_or(0.0);
    let y2 = resolve_length(element, "y2", viewport.height).unwrap_or(0.0);

    if x1 == x2 && y1 == y2 {
        return None;
    }

    Some(LineGeometry { x1, y1, x2, y2 })
}

/// Tessellate a resolved line's stroke into triangle geometry.
pub(crate) fn tessellate_line(geo: &LineGeometry, style: &StrokeStyle, color: [f32; 4]) -> Mesh {
    stroke::tessellate_stroke_path(&to_path(geo), style, color)
}

/// Convert the line segment to an open Lyon path for stroke and marker logic.
pub(crate) fn to_path(geo: &LineGeometry) -> Path {
    stroke::path_from_points(&[(geo.x1, geo.y1), (geo.x2, geo.y2)], false)
        .expect("resolved line has two points")
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
        // x1/y1/x2/y2 default to 0 ([SVG11] §9.5).
        let geo = resolve_line(&line(&[("x2", "20"), ("y2", "10")]), vp()).unwrap();
        assert_eq!(
            geo,
            LineGeometry {
                x1: 0.0,
                y1: 0.0,
                x2: 20.0,
                y2: 10.0,
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
            &line(&[("x1", "10%"), ("y1", "25%"), ("x2", "50%"), ("y2", "75%")]),
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
            }
        );
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
    }

    #[test]
    fn tessellate_line_uses_shared_stroke_style() {
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
        let style = stroke::resolve_stroke_style(&line(&[("stroke-width", "4")]), vp());
        let mesh = tessellate_line(&geo, &style, [0.0, 0.0, 1.0, 1.0]);

        assert!(!mesh.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
        for v in &mesh.vertices {
            assert_eq!(v.color, [0.0, 0.0, 1.0, 1.0]);
            assert_eq!(v.position[2], 0.0);
        }
    }

    #[test]
    fn tessellate_line_supports_dashes_and_caps() {
        let geo = resolve_line(
            &line(&[("x1", "10"), ("y1", "50"), ("x2", "90"), ("y2", "50")]),
            vp(),
        )
        .unwrap();
        let style = stroke::resolve_stroke_style(
            &line(&[
                ("stroke-width", "8"),
                ("stroke-linecap", "round"),
                ("stroke-dasharray", "12 6"),
            ]),
            vp(),
        );
        let mesh = tessellate_line(&geo, &style, [1.0, 0.0, 0.0, 1.0]);

        assert!(!mesh.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
    }
}
