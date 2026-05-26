//! SVG 1.1 `<ellipse>` — geometry resolution plus fill and stroke tessellation.
//!
//! `<ellipse>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<ellipse>`
//! [`Element`] into filled and stroked triangle [`Mesh`]es in SVG user space, applying
//! the SVG 1.1 geometry rules ([SVG11] §9.4). The behavioural reference is
//! the SVG WPT suite (`svg/shapes/ellipse-0*.svg`).
//!
//! It is the axis-independent radius counterpart of
//! [`crate::render::shapes::circle`]: a circle is the special case `rx == ry`. SVG
//! 1.1 §9.4 requires both radii and gives them no mutual "auto" defaulting —
//! unlike a `<rect>`'s `rx`/`ry` ([`crate::render::shapes::rect`]).
//!
//! Length parsing, `fill`, and stroke resolution are shared with the other
//! basic shapes — see [`crate::render::shapes`]. `transform` and grouping are not
//! handled yet — see the crate roadmap.

use crate::dom::Element;
use lyon_tessellation::path::builder::SvgPathBuilder;
use lyon_tessellation::path::math::{point, vector, Angle};
use lyon_tessellation::path::{ArcFlags, Path};

use super::stroke::{self, StrokeStyle};
use super::{resolve_length, sdf_quad, Viewport, KIND_ELLIPSE, SDF_PAD};
use crate::render::Mesh;

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
    let cx = resolve_length(element, "cx", viewport.width).unwrap_or(0.0);
    let cy = resolve_length(element, "cy", viewport.height).unwrap_or(0.0);
    let rx = resolve_length(element, "rx", viewport.width).unwrap_or(0.0);
    let ry = resolve_length(element, "ry", viewport.height).unwrap_or(0.0);

    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }

    Some(EllipseGeometry { cx, cy, rx, ry })
}

/// Tessellate a resolved ellipse into an SDF-covered bounding quad [`Mesh`].
///
/// The disc is a four-vertex quad padded past the radii by [`SDF_PAD`]; the
/// fragment shader computes analytic, anti-aliased coverage from the
/// [`KIND_ELLIPSE`] signed-distance function. Positions are in SVG user space
/// with `z = 0`.
pub(crate) fn tessellate_ellipse(geo: &EllipseGeometry, color: [f32; 4]) -> Mesh {
    // The quad spans each radius plus the anti-aliasing pad; a corner's
    // `local` is its offset from the centre, which the shader compares
    // against the radii carried in `params`.
    let ex = geo.rx + SDF_PAD;
    let ey = geo.ry + SDF_PAD;
    sdf_quad(
        [
            ([geo.cx - ex, geo.cy - ey], [-ex, -ey]),
            ([geo.cx + ex, geo.cy - ey], [ex, -ey]),
            ([geo.cx + ex, geo.cy + ey], [ex, ey]),
            ([geo.cx - ex, geo.cy + ey], [-ex, ey]),
        ],
        [geo.rx, geo.ry, 0.0, 0.0],
        KIND_ELLIPSE,
        color,
    )
}

/// Tessellate a resolved ellipse's stroke into triangle geometry.
pub(crate) fn tessellate_ellipse_stroke(
    geo: &EllipseGeometry,
    style: &StrokeStyle,
    color: [f32; 4],
) -> Mesh {
    stroke::tessellate_stroke_path(&to_path(geo), style, color)
}

/// Convert the ellipse outline to a Lyon path for stroke and marker logic.
pub(crate) fn to_path(geo: &EllipseGeometry) -> Path {
    ellipse_path(geo.cx, geo.cy, geo.rx, geo.ry)
}

pub(crate) fn ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> Path {
    let mut builder = Path::builder().with_svg();
    let radii = vector(rx, ry);
    let rotation = Angle::degrees(0.0);
    let flags = ArcFlags {
        large_arc: false,
        sweep: true,
    };

    builder.move_to(point(cx + rx, cy));
    builder.arc_to(radii, rotation, flags, point(cx, cy + ry));
    builder.arc_to(radii, rotation, flags, point(cx - rx, cy));
    builder.arc_to(radii, rotation, flags, point(cx, cy - ry));
    builder.arc_to(radii, rotation, flags, point(cx + rx, cy));
    builder.close();
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::ElementKind;

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
    fn tessellate_ellipse_is_an_sdf_quad() {
        let geo = resolve_ellipse(
            &ellipse(&[("cx", "50"), ("cy", "40"), ("rx", "30"), ("ry", "20")]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_ellipse(&geo, [0.0, 0.0, 1.0, 1.0]);
        // An SDF ellipse is a four-vertex bounding quad, two triangles.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        for v in &mesh.vertices {
            // `params` carries `rx` then `ry`, pinning each radius to its
            // axis; the fill colour and `z = 0` plane ride on every corner.
            assert_eq!(v.kind, KIND_ELLIPSE);
            assert_eq!(v.params, [30.0, 20.0, 0.0, 0.0]);
            assert_eq!(v.color, [0.0, 0.0, 1.0, 1.0]);
            assert_eq!(v.position[2], 0.0);
            // Each corner's `local` is its offset from the ellipse centre.
            assert_eq!(v.local, [v.position[0] - 50.0, v.position[1] - 40.0]);
        }
    }

    #[test]
    fn resolve_ellipse_rejects_unparseable_radius() {
        // An unparseable radius is treated like a missing one — the ellipse
        // is not rendered. This holds for either axis.
        assert_eq!(
            resolve_ellipse(&ellipse(&[("rx", "abc"), ("ry", "20")]), vp()),
            None
        );
        assert_eq!(
            resolve_ellipse(&ellipse(&[("rx", "20"), ("ry", "")]), vp()),
            None
        );
        assert_eq!(
            resolve_ellipse(&ellipse(&[("rx", "20"), ("ry", "10 20")]), vp()),
            None
        );
        // An unparseable `cx`/`cy`, by contrast, just falls back to its `0`
        // default — it does not disable an otherwise-valid ellipse.
        let geo = resolve_ellipse(
            &ellipse(&[("cx", "nope"), ("rx", "20"), ("ry", "12")]),
            vp(),
        )
        .unwrap();
        assert_eq!((geo.cx, geo.cy), (0.0, 0.0));
    }

    #[test]
    fn resolve_ellipse_mixes_absolute_and_percentage_lengths() {
        // Absolute and percentage lengths may be mixed freely across one
        // ellipse's attributes; each still resolves on its own axis.
        let viewport = Viewport {
            width: 400.0,
            height: 200.0,
        };
        let geo = resolve_ellipse(
            &ellipse(&[("cx", "30"), ("cy", "10%"), ("rx", "25%"), ("ry", "40")]),
            viewport,
        )
        .unwrap();
        assert_eq!(geo.cx, 30.0); // absolute
        assert_eq!(geo.cy, 20.0); // 10% of viewport height 200
        assert_eq!(geo.rx, 100.0); // 25% of viewport width 400
        assert_eq!(geo.ry, 40.0); // absolute
    }
}
