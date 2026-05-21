//! SVG 1.1 `<circle>` — geometry resolution and fill tessellation.
//!
//! `<circle>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<circle>`
//! [`Element`] into a filled triangle [`Mesh`] in SVG user space, applying
//! the SVG 1.1 geometry rules ([SVG11] §9.3). The behavioural reference is
//! the SVG WPT suite (`svg/shapes/circle-0*.svg`).
//!
//! Length parsing and `fill` resolution are shared with the other basic
//! shapes — see [`crate::shape`]. `transform` and grouping are not handled
//! yet — see the crate roadmap.

use std::f32::consts::TAU;

use svg3_dom::Element;

use crate::shape::{parse_length, vertex};
use crate::Mesh;

/// Segments approximating the circle's perimeter. Fixed so a circle's
/// vertex count is deterministic.
const CIRCLE_SEGMENTS: usize = 64;

/// A `<circle>`'s geometry after SVG 1.1 defaulting. All values are in SVG
/// user units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CircleGeometry {
    /// Centre x coordinate.
    pub cx: f32,
    /// Centre y coordinate.
    pub cy: f32,
    /// Radius — always `> 0` for a resolved geometry.
    pub r: f32,
}

/// Resolve a `<circle>`'s raw attributes into a [`CircleGeometry`].
///
/// Returns `None` when the circle is not rendered. Per [SVG11] §9.3 a zero
/// `r` disables rendering; a negative `r` is a document error, which svg3
/// likewise skips rather than rendering. `cx`/`cy` default to `0`.
pub(crate) fn resolve_circle(element: &Element) -> Option<CircleGeometry> {
    let length = |name: &str| {
        element
            .attributes
            .get(name)
            .map(String::as_str)
            .and_then(parse_length)
    };

    let cx = length("cx").unwrap_or(0.0);
    let cy = length("cy").unwrap_or(0.0);
    let r = length("r").unwrap_or(0.0);

    if r <= 0.0 {
        return None;
    }

    Some(CircleGeometry { cx, cy, r })
}

/// Tessellate a resolved circle into a filled triangle [`Mesh`].
///
/// The disc is a triangle fan from its centre over `CIRCLE_SEGMENTS`
/// evenly-spaced perimeter points. Positions are in SVG user space with
/// `z = 0`.
pub(crate) fn tessellate_circle(geo: &CircleGeometry, color: [f32; 4]) -> Mesh {
    // Vertex 0 is the centre; the fan pivots on it.
    let mut vertices = Vec::with_capacity(CIRCLE_SEGMENTS + 1);
    vertices.push(vertex(geo.cx, geo.cy, color));
    for step in 0..CIRCLE_SEGMENTS {
        let t = TAU * (step as f32 / CIRCLE_SEGMENTS as f32);
        vertices.push(vertex(
            geo.cx + geo.r * t.cos(),
            geo.cy + geo.r * t.sin(),
            color,
        ));
    }

    let n = CIRCLE_SEGMENTS as u32;
    let mut indices = Vec::with_capacity(CIRCLE_SEGMENTS * 3);
    for i in 0..n {
        indices.extend_from_slice(&[0, i + 1, (i + 1) % n + 1]);
    }
    Mesh { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    /// Build a `<circle>` element carrying the given raw attributes.
    fn circle(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Circle);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    #[test]
    fn resolve_circle_applies_position_defaults() {
        // `cx`/`cy` default to 0 ([SVG11] §9.3).
        let geo = resolve_circle(&circle(&[("r", "20")])).unwrap();
        assert_eq!(
            geo,
            CircleGeometry {
                cx: 0.0,
                cy: 0.0,
                r: 20.0,
            }
        );
    }

    #[test]
    fn resolve_circle_skips_degenerate_radius() {
        // Missing, zero, or negative `r` => not rendered ([SVG11] §9.3).
        assert_eq!(resolve_circle(&circle(&[("cx", "10")])), None);
        assert_eq!(resolve_circle(&circle(&[("r", "0")])), None);
        assert_eq!(resolve_circle(&circle(&[("r", "-5")])), None);
    }

    #[test]
    fn tessellate_circle_fans_within_bounds() {
        let geo = resolve_circle(&circle(&[("cx", "50"), ("cy", "40"), ("r", "30")])).unwrap();
        let mesh = tessellate_circle(&geo, [0.0, 0.0, 1.0, 1.0]);
        // Centre + one perimeter point per segment; one fan triangle per edge.
        assert_eq!(mesh.vertices.len(), CIRCLE_SEGMENTS + 1);
        assert_eq!(mesh.indices.len(), CIRCLE_SEGMENTS * 3);
        // The fan pivot is the circle centre.
        assert_eq!(mesh.vertices[0].position, [50.0, 40.0, 0.0]);
        assert_eq!(mesh.vertices[0].color, [0.0, 0.0, 1.0, 1.0]);
        // Every perimeter vertex sits on the circle (distance `r` from the
        // centre), hence inside the bounding box, in the plane `z = 0`.
        for v in &mesh.vertices[1..] {
            let (dx, dy) = (v.position[0] - 50.0, v.position[1] - 40.0);
            assert!((dx.hypot(dy) - 30.0).abs() < 1e-3);
            assert_eq!(v.position[2], 0.0);
        }
    }
}
