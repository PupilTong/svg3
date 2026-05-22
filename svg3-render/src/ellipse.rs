//! SVG 1.1 `<ellipse>` — geometry resolution and fill tessellation.
//!
//! `<ellipse>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<ellipse>`
//! [`Element`] into a filled triangle [`Mesh`] in SVG user space, applying
//! the SVG 1.1 geometry rules ([SVG11] §9.4). The behavioural reference is
//! the SVG WPT suite (`svg/shapes/ellipse-0*.svg`).
//!
//! It is the axis-independent radius counterpart of [`crate::circle`]: a
//! circle is the special case `rx == ry`. SVG 1.1 §9.4 requires both radii
//! and gives them no mutual "auto" defaulting — unlike a `<rect>`'s
//! `rx`/`ry` ([`crate::rect`]).
//!
//! Length parsing and `fill` resolution are shared with the other basic
//! shapes — see [`crate::shape`]. `transform` and grouping are not handled
//! yet — see the crate roadmap.

use std::f32::consts::TAU;

use svg3_dom::Element;

use crate::shape::{vertex, Length, Viewport};
use crate::Mesh;

/// Segments approximating the ellipse's perimeter. Fixed so an ellipse's
/// vertex count is deterministic.
const ELLIPSE_SEGMENTS: usize = 64;

/// An `<ellipse>`'s geometry after SVG 1.1 defaulting. All values are in SVG
/// user units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EllipseGeometry {
    /// Centre x coordinate.
    pub cx: f32,
    /// Centre y coordinate.
    pub cy: f32,
    /// X-axis radius — always `> 0` for a resolved geometry.
    pub rx: f32,
    /// Y-axis radius — always `> 0` for a resolved geometry.
    pub ry: f32,
}

/// Resolve an `<ellipse>`'s raw attributes into an [`EllipseGeometry`].
///
/// Percentage lengths resolve against `viewport` — the horizontal `cx`/`rx`
/// against its width, the vertical `cy`/`ry` against its height
/// ([SVG11] §7.10).
///
/// Returns `None` when the ellipse is not rendered. Per [SVG11] §9.4 a zero
/// `rx` or `ry` disables rendering; a negative radius is a document error,
/// which svg3 likewise skips rather than rendering. `rx` and `ry` are both
/// required and have no mutual defaulting; an omitted radius is treated as
/// zero and so also disables rendering. `cx`/`cy` default to `0`.
pub(crate) fn resolve_ellipse(element: &Element, viewport: Viewport) -> Option<EllipseGeometry> {
    let length = |name: &str, basis: f32| {
        element
            .attributes
            .get(name)
            .map(String::as_str)
            .and_then(Length::parse)
            .map(|len| len.resolve(basis))
    };

    let cx = length("cx", viewport.width).unwrap_or(0.0);
    let cy = length("cy", viewport.height).unwrap_or(0.0);
    let rx = length("rx", viewport.width).unwrap_or(0.0);
    let ry = length("ry", viewport.height).unwrap_or(0.0);

    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }

    Some(EllipseGeometry { cx, cy, rx, ry })
}

/// Tessellate a resolved ellipse into a filled triangle [`Mesh`].
///
/// The disc is a triangle fan from its centre over `ELLIPSE_SEGMENTS`
/// evenly-spaced perimeter points — sample `t` lands at
/// `(cx + rx·cos t, cy + ry·sin t)`. Positions are in SVG user space with
/// `z = 0`.
pub(crate) fn tessellate_ellipse(geo: &EllipseGeometry, color: [f32; 4]) -> Mesh {
    // Vertex 0 is the centre; the fan pivots on it.
    let mut vertices = Vec::with_capacity(ELLIPSE_SEGMENTS + 1);
    vertices.push(vertex(geo.cx, geo.cy, color));
    for step in 0..ELLIPSE_SEGMENTS {
        let t = TAU * (step as f32 / ELLIPSE_SEGMENTS as f32);
        vertices.push(vertex(
            geo.cx + geo.rx * t.cos(),
            geo.cy + geo.ry * t.sin(),
            color,
        ));
    }

    let n = ELLIPSE_SEGMENTS as u32;
    let mut indices = Vec::with_capacity(ELLIPSE_SEGMENTS * 3);
    for i in 0..n {
        indices.extend_from_slice(&[0, i + 1, (i + 1) % n + 1]);
    }
    Mesh { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    /// Build an `<ellipse>` element carrying the given raw attributes.
    fn ellipse(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Ellipse);
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
    fn resolve_ellipse_applies_position_defaults() {
        // `cx`/`cy` default to 0 ([SVG11] §9.4).
        let geo = resolve_ellipse(&ellipse(&[("rx", "20"), ("ry", "10")]), vp()).unwrap();
        assert_eq!(
            geo,
            EllipseGeometry {
                cx: 0.0,
                cy: 0.0,
                rx: 20.0,
                ry: 10.0,
            }
        );
    }

    #[test]
    fn resolve_ellipse_resolves_percentage_geometry() {
        // The horizontal `cx`/`rx` resolve against viewport width, the
        // vertical `cy`/`ry` against viewport height ([SVG11] §7.10).
        let viewport = Viewport {
            width: 200.0,
            height: 100.0,
        };
        let geo = resolve_ellipse(
            &ellipse(&[("cx", "50%"), ("cy", "25%"), ("rx", "50%"), ("ry", "10%")]),
            viewport,
        )
        .unwrap();
        assert_eq!(geo.cx, 100.0);
        assert_eq!(geo.cy, 25.0);
        assert_eq!(geo.rx, 100.0);
        assert_eq!(geo.ry, 10.0);
    }

    #[test]
    fn resolve_ellipse_skips_degenerate_radius() {
        // A missing, zero, or negative `rx` or `ry` => not rendered. Both
        // radii are required and neither defaults to the other ([SVG11] §9.4).
        assert_eq!(resolve_ellipse(&ellipse(&[("rx", "20")]), vp()), None);
        assert_eq!(resolve_ellipse(&ellipse(&[("ry", "20")]), vp()), None);
        assert_eq!(
            resolve_ellipse(&ellipse(&[("rx", "0"), ("ry", "20")]), vp()),
            None
        );
        assert_eq!(
            resolve_ellipse(&ellipse(&[("rx", "20"), ("ry", "0")]), vp()),
            None
        );
        assert_eq!(
            resolve_ellipse(&ellipse(&[("rx", "-5"), ("ry", "20")]), vp()),
            None
        );
        assert_eq!(
            resolve_ellipse(&ellipse(&[("rx", "20"), ("ry", "-5")]), vp()),
            None
        );
    }

    #[test]
    fn tessellate_ellipse_fans_within_bounds() {
        let geo = resolve_ellipse(
            &ellipse(&[("cx", "50"), ("cy", "40"), ("rx", "30"), ("ry", "20")]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_ellipse(&geo, [0.0, 0.0, 1.0, 1.0]);
        // Centre + one perimeter point per segment; one fan triangle per edge.
        assert_eq!(mesh.vertices.len(), ELLIPSE_SEGMENTS + 1);
        assert_eq!(mesh.indices.len(), ELLIPSE_SEGMENTS * 3);
        // The fan pivot is the ellipse centre.
        assert_eq!(mesh.vertices[0].position, [50.0, 40.0, 0.0]);
        assert_eq!(mesh.vertices[0].color, [0.0, 0.0, 1.0, 1.0]);
        // Every perimeter vertex satisfies the ellipse equation
        // `((x-cx)/rx)² + ((y-cy)/ry)² == 1`, hence lies inside the bounding
        // box, in the plane `z = 0`.
        for v in &mesh.vertices[1..] {
            let nx = (v.position[0] - 50.0) / 30.0;
            let ny = (v.position[1] - 40.0) / 20.0;
            assert!((nx * nx + ny * ny - 1.0).abs() < 1e-3);
            assert_eq!(v.position[2], 0.0);
        }
    }
}
