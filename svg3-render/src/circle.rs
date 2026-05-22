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

use svg3_dom::Element;

use crate::shape::{sdf_quad, Length, Viewport, KIND_ELLIPSE, SDF_PAD};
use crate::Mesh;

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
/// Percentage lengths resolve against `viewport` — `cx` against its width,
/// `cy` against its height, and `r` against its normalized diagonal
/// ([SVG11] §7.10).
///
/// Returns `None` when the circle is not rendered. Per [SVG11] §9.3 a zero
/// `r` disables rendering; a negative `r` is a document error, which svg3
/// likewise skips rather than rendering. `cx`/`cy` default to `0`.
pub(crate) fn resolve_circle(element: &Element, viewport: Viewport) -> Option<CircleGeometry> {
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
    let r = length("r", viewport.diagonal()).unwrap_or(0.0);

    if r <= 0.0 {
        return None;
    }

    Some(CircleGeometry { cx, cy, r })
}

/// Tessellate a resolved circle into an SDF-covered bounding quad [`Mesh`].
///
/// The disc is a four-vertex quad padded past the radius by [`SDF_PAD`]; the
/// fragment shader computes analytic, anti-aliased coverage from the
/// [`KIND_ELLIPSE`] signed-distance function. Positions are in SVG user space
/// with `z = 0`.
pub(crate) fn tessellate_circle(geo: &CircleGeometry, color: [f32; 4]) -> Mesh {
    // The quad spans the radius plus the anti-aliasing pad; each corner's
    // `local` is its offset from the centre, which the shader compares
    // against the radius carried in `params`.
    let ext = geo.r + SDF_PAD;
    sdf_quad(
        [
            ([geo.cx - ext, geo.cy - ext], [-ext, -ext]),
            ([geo.cx + ext, geo.cy - ext], [ext, -ext]),
            ([geo.cx + ext, geo.cy + ext], [ext, ext]),
            ([geo.cx - ext, geo.cy + ext], [-ext, ext]),
        ],
        [geo.r, geo.r, 0.0, 0.0],
        KIND_ELLIPSE,
        color,
    )
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

    /// A 100×100 viewport. The geometry tests below use absolute lengths, so
    /// the viewport value only matters for the percentage case.
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn resolve_circle_applies_position_defaults() {
        // `cx`/`cy` default to 0 ([SVG11] §9.3).
        let geo = resolve_circle(&circle(&[("r", "20")]), vp()).unwrap();
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
    fn resolve_circle_resolves_percentage_geometry() {
        // `cx` resolves against viewport width, `cy` against viewport height
        // ([SVG11] §7.10).
        let viewport = Viewport {
            width: 200.0,
            height: 100.0,
        };
        let geo = resolve_circle(
            &circle(&[("cx", "50%"), ("cy", "25%"), ("r", "20")]),
            viewport,
        )
        .unwrap();
        assert_eq!(geo.cx, 100.0);
        assert_eq!(geo.cy, 25.0);
        assert_eq!(geo.r, 20.0);
    }

    #[test]
    fn resolve_circle_skips_degenerate_radius() {
        // Missing, zero, or negative `r` => not rendered ([SVG11] §9.3).
        assert_eq!(resolve_circle(&circle(&[("cx", "10")]), vp()), None);
        assert_eq!(resolve_circle(&circle(&[("r", "0")]), vp()), None);
        assert_eq!(resolve_circle(&circle(&[("r", "-5")]), vp()), None);
    }

    #[test]
    fn tessellate_circle_is_an_sdf_quad() {
        let geo =
            resolve_circle(&circle(&[("cx", "50"), ("cy", "40"), ("r", "30")]), vp()).unwrap();
        let mesh = tessellate_circle(&geo, [0.0, 0.0, 1.0, 1.0]);
        // An SDF circle is a four-vertex bounding quad, two triangles.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        for v in &mesh.vertices {
            // The radius drives the ellipse SDF via `params`; the fill colour
            // and the `z = 0` plane are carried on every corner.
            assert_eq!(v.kind, KIND_ELLIPSE);
            assert_eq!(v.params, [30.0, 30.0, 0.0, 0.0]);
            assert_eq!(v.color, [0.0, 0.0, 1.0, 1.0]);
            assert_eq!(v.position[2], 0.0);
            // Each corner's `local` is its offset from the circle centre —
            // the coordinate the shader feeds to the SDF.
            assert_eq!(v.local, [v.position[0] - 50.0, v.position[1] - 40.0]);
        }
    }
}
