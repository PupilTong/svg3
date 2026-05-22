//! SVG 1.1 `<rect>` — geometry resolution and fill tessellation.
//!
//! `<rect>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<rect>`
//! [`Element`] into a filled triangle [`Mesh`] in SVG user space, applying
//! the SVG 1.1 geometry rules ([SVG11] §9.2). The behavioural reference is
//! the SVG WPT suite (`svg/shapes/rect-0*.svg`).
//!
//! Length parsing and `fill` resolution are shared with the other basic
//! shapes — see [`crate::shapes`]. `transform` and grouping are not handled
//! yet — see the crate roadmap.

use svg3_dom::Element;

use super::{resolve_length, sdf_quad, vertex, Length, Viewport, KIND_ROUND_BOX, SDF_PAD};
use crate::Mesh;

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
/// Percentage lengths resolve against `viewport` — `x`/`width`/`rx` against
/// its width, `y`/`height`/`ry` against its height ([SVG11] §7.10).
///
/// Returns `None` when the rectangle is not rendered — a missing, zero, or
/// negative `width`/`height` ([SVG11] §9.2; WPT `shapes/rect-05`).
///
/// `x`/`y` default to `0`. Corner radii follow the `rx`/`ry` "auto" rules
/// ([SVG11] §9.2): if only one is given the other mirrors its `<length>`
/// (so a percentage radius still resolves against its own axis); if
/// neither is given both are `0`; a negative radius is treated as auto.
/// Each radius is then clamped to half its side (WPT `import/shapes-rect-06`).
pub(crate) fn resolve_rect(element: &Element, viewport: Viewport) -> Option<RectGeometry> {
    let x = resolve_length(element, "x", viewport.width).unwrap_or(0.0);
    let y = resolve_length(element, "y", viewport.height).unwrap_or(0.0);
    let width = resolve_length(element, "width", viewport.width).unwrap_or(0.0);
    let height = resolve_length(element, "height", viewport.height).unwrap_or(0.0);

    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    // A negative or unparseable radius is "auto". Per [SVG11] §9.2 an auto
    // axis mirrors the other's `<length>` — the length token, not its
    // resolved pixels — so each radius then resolves against its own axis:
    // `rx` against viewport width, `ry` against height ([SVG11] §7.10).
    // Both auto => sharp corners.
    let radius = |name: &str| {
        element
            .attributes
            .get(name)
            .map(String::as_str)
            .and_then(Length::parse)
            .filter(|len| !len.is_negative())
    };
    let (rx_len, ry_len) = match (radius("rx"), radius("ry")) {
        (Some(rx), Some(ry)) => (rx, ry),
        (Some(rx), None) => (rx, rx),
        (None, Some(ry)) => (ry, ry),
        (None, None) => (Length::Px(0.0), Length::Px(0.0)),
    };
    let rx = rx_len.resolve(viewport.width);
    let ry = ry_len.resolve(viewport.height);

    Some(RectGeometry {
        x,
        y,
        width,
        height,
        rx: rx.min(width / 2.0),
        ry: ry.min(height / 2.0),
    })
}

/// Tessellate a resolved rectangle into a filled [`Mesh`].
///
/// A sharp rectangle becomes two solid triangles (4 vertices). A rounded
/// rectangle becomes an SDF-covered bounding quad whose fragment-shader
/// signed-distance function gives analytic, anti-aliased elliptical corners.
/// Positions are in SVG user space with `z = 0`.
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

/// Tessellate a resolved rounded rectangle into an SDF-covered bounding quad.
///
/// The rectangle is a four-vertex quad padded past its bounds by [`SDF_PAD`];
/// the fragment shader computes analytic, anti-aliased coverage from the
/// [`KIND_ROUND_BOX`] signed-distance function, whose corners are
/// quarter-ellipses of radii `rx`/`ry`.
fn rounded_mesh(geo: &RectGeometry, color: [f32; 4]) -> Mesh {
    let half_w = geo.width / 2.0;
    let half_h = geo.height / 2.0;
    let cx = geo.x + half_w;
    let cy = geo.y + half_h;
    // The quad spans the half-extents plus the anti-aliasing pad; a corner's
    // `local` is its offset from the rect centre, which the shader compares
    // against the half-extents and corner radii carried in `params`.
    let ex = half_w + SDF_PAD;
    let ey = half_h + SDF_PAD;
    sdf_quad(
        [
            ([cx - ex, cy - ey], [-ex, -ey]),
            ([cx + ex, cy - ey], [ex, -ey]),
            ([cx + ex, cy + ey], [ex, ey]),
            ([cx - ex, cy + ey], [-ex, ey]),
        ],
        [half_w, half_h, geo.rx, geo.ry],
        KIND_ROUND_BOX,
        color,
    )
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

    /// A 100×100 viewport. The geometry tests below use absolute lengths, so
    /// the viewport value only matters for the percentage case.
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn resolve_rect_applies_position_defaults() {
        // `x`/`y` default to 0 ([SVG11] §9.2).
        let geo = resolve_rect(&rect(&[("width", "40"), ("height", "20")]), vp()).unwrap();
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
    fn resolve_rect_resolves_percentage_sizes() {
        // `width`/`x` resolve against viewport width, `height`/`y` against
        // viewport height ([SVG11] §7.10).
        let viewport = Viewport {
            width: 300.0,
            height: 200.0,
        };
        let geo = resolve_rect(
            &rect(&[
                ("x", "10%"),
                ("y", "25%"),
                ("width", "100%"),
                ("height", "50%"),
            ]),
            viewport,
        )
        .unwrap();
        assert_eq!(
            geo,
            RectGeometry {
                x: 30.0,
                y: 50.0,
                width: 300.0,
                height: 100.0,
                rx: 0.0,
                ry: 0.0,
            }
        );
    }

    #[test]
    fn resolve_rect_skips_degenerate_sizes() {
        // Missing, zero, or negative width/height => not rendered
        // (WPT `shapes/rect-05`).
        assert_eq!(resolve_rect(&rect(&[("height", "10")]), vp()), None);
        assert_eq!(
            resolve_rect(&rect(&[("width", "0"), ("height", "10")]), vp()),
            None
        );
        assert_eq!(
            resolve_rect(&rect(&[("width", "10"), ("height", "0")]), vp()),
            None
        );
        assert_eq!(
            resolve_rect(&rect(&[("width", "-5"), ("height", "10")]), vp()),
            None
        );
        assert_eq!(
            resolve_rect(&rect(&[("width", "10"), ("height", "-5")]), vp()),
            None
        );
    }

    #[test]
    fn resolve_rect_aliases_auto_corner_radii() {
        let radii = |extra: &[(&str, &str)]| {
            let mut attrs = vec![("width", "100"), ("height", "100")];
            attrs.extend_from_slice(extra);
            let g = resolve_rect(&rect(&attrs), vp()).unwrap();
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
    fn resolve_rect_aliases_percentage_radius_per_axis() {
        // An auto radius mirrors the *length* of the other, not its resolved
        // pixels: `rx="25%"` with no `ry` sets `ry` to the length `25%`,
        // which resolves against viewport *height* ([SVG11] §9.2, §7.10) —
        // so a 200×100 viewport yields `rx=50`, `ry=25`, not `ry=50`.
        let viewport = Viewport {
            width: 200.0,
            height: 100.0,
        };
        let geo = resolve_rect(
            &rect(&[("width", "200"), ("height", "100"), ("rx", "25%")]),
            viewport,
        )
        .unwrap();
        assert_eq!((geo.rx, geo.ry), (50.0, 25.0));
        // Symmetric: an auto `rx` mirrors `ry`'s length.
        let geo = resolve_rect(
            &rect(&[("width", "200"), ("height", "100"), ("ry", "25%")]),
            viewport,
        )
        .unwrap();
        assert_eq!((geo.rx, geo.ry), (50.0, 25.0));
    }

    #[test]
    fn resolve_rect_clamps_radii_to_half_side() {
        // rx/ry over half the side are clamped, independently per axis
        // (WPT `import/shapes-rect-06`).
        let geo = resolve_rect(
            &rect(&[
                ("width", "20"),
                ("height", "100"),
                ("rx", "50"),
                ("ry", "20"),
            ]),
            vp(),
        )
        .unwrap();
        assert_eq!((geo.rx, geo.ry), (10.0, 20.0));
    }

    #[test]
    fn tessellate_sharp_rect_is_two_triangles() {
        // WPT `shapes/rect-01`: <rect x=10 y=10 width=50 height=50>.
        let geo = resolve_rect(
            &rect(&[("x", "10"), ("y", "10"), ("width", "50"), ("height", "50")]),
            vp(),
        )
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
        let geo = resolve_rect(
            &rect(&[("width", "40"), ("height", "40"), ("rx", "10"), ("ry", "0")]),
            vp(),
        )
        .unwrap();
        assert_eq!(tessellate_rect(&geo, [1.0; 4]).vertices.len(), 4);
    }

    #[test]
    fn tessellate_rounded_rect_is_an_sdf_quad() {
        // WPT `shapes/rect-03`: <rect x=10 y=10 width=50 height=50 rx=8 ry=8>.
        let geo = resolve_rect(
            &rect(&[
                ("x", "10"),
                ("y", "10"),
                ("width", "50"),
                ("height", "50"),
                ("rx", "8"),
                ("ry", "8"),
            ]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_rect(&geo, [1.0; 4]);
        // A rounded rect is a four-vertex SDF bounding quad, two triangles.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        for v in &mesh.vertices {
            // `params` carries the half-extents then the corner radii.
            assert_eq!(v.kind, KIND_ROUND_BOX);
            assert_eq!(v.params, [25.0, 25.0, 8.0, 8.0]);
            assert_eq!(v.position[2], 0.0);
            // Each corner's `local` is its offset from the rect centroid.
            assert_eq!(v.local, [v.position[0] - 35.0, v.position[1] - 35.0]);
        }
    }
}
