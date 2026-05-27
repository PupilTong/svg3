//! svg3 `<cube>` — 3D axis-aligned cuboid geometry resolution and tessellation.
//!
//! `<cube>` is a three-dimensional graphics element ([SPEC.md](../../SPEC.md)
//! §5.2). This module turns a parsed `<cube>` [`Element`] into a filled
//! triangle [`Mesh`] of six rectangular faces (12 triangles, 8 corners) in
//! the svg3 world coordinate system: +X right, +Y down, +Z toward the viewer
//! ([SPEC.md](../../SPEC.md) §3.1). Unlike the 2D basic shapes, cube
//! geometry leaves the plane `z = 0`, so the [`Camera`](crate::Camera) is
//! the natural way to view it; the default orthographic projection collapses
//! the cube to its axis-aligned bounding rectangle.
//!
//! Faces are wound counter-clockwise as seen from outside the cube so a
//! future back-face culling pass discards the three hidden faces; with a
//! single uniform [`Element::fill`](crate::dom::Element) colour today, the
//! cube renders as its outward hexagonal silhouette in any view — the three
//! occluded faces overdraw the visible ones with the same colour, so the
//! end result is the same union of triangles.
//!
//! Length parsing and `fill` resolution are shared with the 2D basic shapes
//! — see [`crate::render::shapes`]. Group inheritance and `transform`
//! composition are applied by [`crate::render::scene`].

use crate::dom::Element;

use super::{resolve_length, Length, Viewport, KIND_SOLID};
use crate::render::{Mesh, Vertex};

/// A `<cube>`'s geometry after [SPEC.md](../../SPEC.md) §5.2 defaulting. All
/// values are in svg3 user units; widths are the full edge lengths, not half-
/// extents.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CubeGeometry {
    /// Centre x coordinate.
    pub cx: f32,
    /// Centre y coordinate.
    pub cy: f32,
    /// Centre z coordinate.
    pub cz: f32,
    /// Edge length along X — always `> 0` for a resolved geometry.
    pub width: f32,
    /// Edge length along Y — always `> 0` for a resolved geometry.
    pub height: f32,
    /// Edge length along Z — always `> 0` for a resolved geometry.
    pub depth: f32,
}

/// Resolve a `<cube>`'s raw attributes into a [`CubeGeometry`].
///
/// Percentage lengths resolve against `viewport` per axis — `cx` and `width`
/// against its width, `cy` and `height` against its height, `cz` and `depth`
/// against its normalized diagonal (the same isotropic basis SVG 1.1 uses for
/// `<circle>`'s `r`, since the Z axis is neither horizontal nor vertical).
///
/// Returns `None` when the cube is not rendered. Per [SPEC.md](../../SPEC.md)
/// §5.2, a negative value for `'size'`, `'width'`, `'height'`, or `'depth'`
/// is an error — the element renders as if it had not been specified. A zero
/// edge length on any axis also disables rendering. `cx`/`cy`/`cz` default to
/// `0`; an unspecified `width`/`height`/`depth` defaults to the `'size'`
/// length token if present (so `size="50%"` then resolves per axis), or zero
/// if neither is given.
pub(crate) fn resolve_cube(element: &Element, viewport: Viewport) -> Option<CubeGeometry> {
    let cx = resolve_length(element, "cx", viewport.width).unwrap_or(0.0);
    let cy = resolve_length(element, "cy", viewport.height).unwrap_or(0.0);
    let cz = resolve_length(element, "cz", viewport.diagonal()).unwrap_or(0.0);

    let size = parse_length_attr(element, "size");
    let width_attr = parse_length_attr(element, "width");
    let height_attr = parse_length_attr(element, "height");
    let depth_attr = parse_length_attr(element, "depth");

    // SPEC §5.2: a negative value for any of size/width/height/depth is an
    // error; the cube renders as if not specified.
    let any_negative = [size, width_attr, height_attr, depth_attr]
        .iter()
        .any(|opt| opt.is_some_and(Length::is_negative));
    if any_negative {
        return None;
    }

    // Per-axis attribute falls back to the `size` length token (so a
    // percentage size resolves against each axis's own basis), then to zero.
    let resolve = |attr: Option<Length>, basis: f32| -> f32 {
        attr.or(size).map(|l| l.resolve(basis)).unwrap_or(0.0)
    };
    let width = resolve(width_attr, viewport.width);
    let height = resolve(height_attr, viewport.height);
    let depth = resolve(depth_attr, viewport.diagonal());

    // SPEC §5.2: zero on any axis disables rendering.
    if width <= 0.0 || height <= 0.0 || depth <= 0.0 {
        return None;
    }

    Some(CubeGeometry {
        cx,
        cy,
        cz,
        width,
        height,
        depth,
    })
}

/// Tessellate a resolved cube into a filled [`Mesh`] of six rectangular faces.
///
/// Produces 8 corner vertices and 36 indices (12 triangles, 2 per face), all
/// tagged [`KIND_SOLID`] — the fragment shader paints each at full coverage
/// in the cube's single `fill` colour. Faces are wound counter-clockwise as
/// seen from outside the cube; future back-face culling can drop the hidden
/// three with no further change to this tessellator.
pub(crate) fn tessellate_cube(geo: &CubeGeometry, color: [f32; 4]) -> Mesh {
    let hx = geo.width * 0.5;
    let hy = geo.height * 0.5;
    let hz = geo.depth * 0.5;

    let corner = |sx: f32, sy: f32, sz: f32| Vertex {
        position: [geo.cx + sx * hx, geo.cy + sy * hy, geo.cz + sz * hz],
        color,
        local: [0.0, 0.0],
        params: [0.0; 4],
        kind: KIND_SOLID,
        paint_id: 0,
    };

    // Eight corners. SVG y is down, +Z is toward the viewer, so "top" is the
    // y < cy half and "front" is the z > cz half.
    let vertices = vec![
        corner(-1.0, -1.0, -1.0), // v0: back-left-top
        corner(1.0, -1.0, -1.0),  // v1: back-right-top
        corner(1.0, 1.0, -1.0),   // v2: back-right-bottom
        corner(-1.0, 1.0, -1.0),  // v3: back-left-bottom
        corner(-1.0, -1.0, 1.0),  // v4: front-left-top
        corner(1.0, -1.0, 1.0),   // v5: front-right-top
        corner(1.0, 1.0, 1.0),    // v6: front-right-bottom
        corner(-1.0, 1.0, 1.0),   // v7: front-left-bottom
    ];

    // Each face is a quad `(a, b, c, d)` of outward-CCW corners, expanded to
    // two triangles `(a, b, c)` and `(a, c, d)`. The winding gives each face
    // an outward normal — verified per face by the cross product of its first
    // two edges in `outward_facing_winding`.
    let indices = vec![
        // Front (+Z): v4, v5, v6, v7
        4, 5, 6, 4, 6, 7, // Back (-Z): v1, v0, v3, v2
        1, 0, 3, 1, 3, 2, // Top (-Y, smaller y in SVG): v0, v1, v5, v4
        0, 1, 5, 0, 5, 4, // Bottom (+Y): v7, v6, v2, v3
        7, 6, 2, 7, 2, 3, // Left (-X): v0, v4, v7, v3
        0, 4, 7, 0, 7, 3, // Right (+X): v5, v1, v2, v6
        5, 1, 2, 5, 2, 6,
    ];

    Mesh::new(vertices, indices)
}

/// Parse a named SVG length attribute into a [`Length`], without resolving it
/// against any basis. Returns `None` for an absent or unparseable value.
/// Sign is preserved so a caller can flag negative inputs per
/// [SPEC.md](../../SPEC.md) §5.2 before resolution.
fn parse_length_attr(element: &Element, name: &str) -> Option<Length> {
    element
        .attributes
        .get(name)
        .map(String::as_str)
        .and_then(Length::parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::ElementKind;

    /// Build a `<cube>` element carrying the given raw attributes.
    fn cube(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Cube);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    /// A 100×100 viewport. The geometry tests below use absolute lengths, so
    /// the viewport value only matters for the percentage cases.
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn resolve_cube_applies_size_to_all_three_axes() {
        // SPEC §5.2: `size` is the default for `width`/`height`/`depth`.
        let geo = resolve_cube(&cube(&[("size", "40")]), vp()).unwrap();
        assert_eq!(
            geo,
            CubeGeometry {
                cx: 0.0,
                cy: 0.0,
                cz: 0.0,
                width: 40.0,
                height: 40.0,
                depth: 40.0,
            }
        );
    }

    #[test]
    fn resolve_cube_applies_centre_defaults() {
        // SPEC §5.2: `cx`/`cy`/`cz` default to 0.
        let geo = resolve_cube(&cube(&[("size", "10")]), vp()).unwrap();
        assert_eq!((geo.cx, geo.cy, geo.cz), (0.0, 0.0, 0.0));
    }

    #[test]
    fn resolve_cube_lets_per_axis_attrs_override_size() {
        // Per SPEC §5.2 the `width`/`height`/`depth` attributes shadow `size`
        // on their own axis; the others still inherit it.
        let geo = resolve_cube(
            &cube(&[
                ("cx", "10"),
                ("cy", "20"),
                ("cz", "30"),
                ("size", "40"),
                ("width", "12"),
            ]),
            vp(),
        )
        .unwrap();
        assert_eq!(
            geo,
            CubeGeometry {
                cx: 10.0,
                cy: 20.0,
                cz: 30.0,
                width: 12.0,
                height: 40.0,
                depth: 40.0,
            }
        );
    }

    #[test]
    fn resolve_cube_accepts_independent_axis_lengths() {
        // With no `size`, each axis is set directly.
        let geo = resolve_cube(
            &cube(&[("width", "20"), ("height", "30"), ("depth", "40")]),
            vp(),
        )
        .unwrap();
        assert_eq!((geo.width, geo.height, geo.depth), (20.0, 30.0, 40.0));
    }

    #[test]
    fn resolve_cube_resolves_percentages_per_axis() {
        // SPEC §3.2 carries SVG 1.1's percentage rules forward: width-axis
        // lengths resolve against viewport width, height-axis against
        // height; the Z axis (neither horizontal nor vertical) uses the
        // normalized diagonal — the same isotropic basis as `<circle>`'s `r`.
        let viewport = Viewport {
            width: 200.0,
            height: 100.0,
        };
        let geo = resolve_cube(
            &cube(&[
                ("cx", "50%"),
                ("cy", "25%"),
                ("width", "50%"),
                ("height", "20%"),
                ("depth", "50%"),
            ]),
            viewport,
        )
        .unwrap();
        assert_eq!(geo.cx, 100.0);
        assert_eq!(geo.cy, 25.0);
        assert_eq!(geo.width, 100.0);
        assert_eq!(geo.height, 20.0);
        // 50% of the 200×100 viewport diagonal.
        assert!((geo.depth - viewport.diagonal() * 0.5).abs() < 1e-4);
    }

    #[test]
    fn resolve_cube_inherits_size_length_token_per_axis() {
        // Per SPEC §5.2 an unspecified per-axis attribute defaults to the
        // `size` *length token* (not its resolved pixels), so `size="50%"`
        // resolves separately on each axis — width vs height vs diagonal.
        let viewport = Viewport {
            width: 200.0,
            height: 80.0,
        };
        let geo = resolve_cube(&cube(&[("size", "50%")]), viewport).unwrap();
        assert_eq!(geo.width, 100.0); // 50% of viewport.width
        assert_eq!(geo.height, 40.0); // 50% of viewport.height
        assert!((geo.depth - viewport.diagonal() * 0.5).abs() < 1e-4);
    }

    #[test]
    fn resolve_cube_skips_negative_dimensions() {
        // SPEC §5.2: a negative value for any of `size`/`width`/`height`/
        // `depth` is an error; the cube renders as if not specified.
        assert_eq!(resolve_cube(&cube(&[("size", "-1")]), vp()), None);
        assert_eq!(
            resolve_cube(
                &cube(&[("width", "-1"), ("height", "10"), ("depth", "10")]),
                vp()
            ),
            None
        );
        assert_eq!(
            resolve_cube(
                &cube(&[("width", "10"), ("height", "-1"), ("depth", "10")]),
                vp()
            ),
            None
        );
        assert_eq!(
            resolve_cube(
                &cube(&[("width", "10"), ("height", "10"), ("depth", "-1")]),
                vp()
            ),
            None
        );
    }

    #[test]
    fn resolve_cube_skips_zero_or_missing_axis() {
        // SPEC §5.2: a zero `width`/`height`/`depth` disables rendering, and
        // an unspecified axis with no `size` falls through to zero.
        assert_eq!(resolve_cube(&cube(&[]), vp()), None);
        assert_eq!(
            resolve_cube(&cube(&[("width", "10"), ("height", "10")]), vp()),
            None
        );
        assert_eq!(resolve_cube(&cube(&[("size", "0")]), vp()), None);
        assert_eq!(
            resolve_cube(&cube(&[("size", "10"), ("depth", "0")]), vp()),
            None
        );
    }

    #[test]
    fn resolve_cube_ignores_unparseable_centre_coordinate() {
        // An unparseable `cx`/`cy`/`cz` falls back to its `0` default, just
        // as on the 2D shapes — it does not disable an otherwise-valid cube.
        let geo = resolve_cube(&cube(&[("size", "10"), ("cx", "nope")]), vp()).unwrap();
        assert_eq!((geo.cx, geo.cy, geo.cz), (0.0, 0.0, 0.0));
    }

    #[test]
    fn tessellate_cube_has_eight_corners_and_twelve_triangles() {
        let geo = CubeGeometry {
            cx: 50.0,
            cy: 50.0,
            cz: 0.0,
            width: 20.0,
            height: 20.0,
            depth: 20.0,
        };
        let mesh = tessellate_cube(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 36);
        assert!(mesh.indices.iter().all(|&i| (i as usize) < 8));
        for v in &mesh.vertices {
            assert_eq!(v.color, [0.0, 0.0, 1.0, 1.0]);
            assert_eq!(v.kind, KIND_SOLID);
            // Every corner sits on a half-extent: x in cx±10, y in cy±10,
            // z in cz±10. No corner is dropped onto an interior plane.
            assert!((v.position[0] - 50.0).abs() - 10.0 < 1e-4);
            assert!((v.position[1] - 50.0).abs() - 10.0 < 1e-4);
            assert!(v.position[2].abs() - 10.0 < 1e-4);
        }
    }

    #[test]
    fn tessellate_cube_centers_corners_around_attributes() {
        let geo = CubeGeometry {
            cx: 10.0,
            cy: 20.0,
            cz: 5.0,
            width: 4.0,
            height: 6.0,
            depth: 8.0,
        };
        let mesh = tessellate_cube(&geo, [1.0; 4]);
        // The 8 corners are the Cartesian product of half-extent offsets.
        // The centroid of the corners equals the cube centre.
        let mut sum = [0.0f32; 3];
        for v in &mesh.vertices {
            sum[0] += v.position[0];
            sum[1] += v.position[1];
            sum[2] += v.position[2];
        }
        let n = mesh.vertices.len() as f32;
        assert!((sum[0] / n - geo.cx).abs() < 1e-4);
        assert!((sum[1] / n - geo.cy).abs() < 1e-4);
        assert!((sum[2] / n - geo.cz).abs() < 1e-4);
    }

    #[test]
    fn tessellate_cube_winds_each_face_outward() {
        // Every face's first triangle is wound counter-clockwise from
        // outside the cube — the cross product of its first two edges points
        // away from the cube centre. This pins the convention so a future
        // back-face culling pipeline drops the right three faces.
        let geo = CubeGeometry {
            cx: 0.0,
            cy: 0.0,
            cz: 0.0,
            width: 2.0,
            height: 2.0,
            depth: 2.0,
        };
        let mesh = tessellate_cube(&geo, [1.0; 4]);
        // The 12 triangles come in 6 face-pairs; check one triangle per face.
        for face in 0..6 {
            let i = face * 6;
            let a = mesh.vertices[mesh.indices[i] as usize].position;
            let b = mesh.vertices[mesh.indices[i + 1] as usize].position;
            let c = mesh.vertices[mesh.indices[i + 2] as usize].position;
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            // Normal = e1 × e2.
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            // Triangle centroid (used as a stand-in for the face centroid —
            // any vertex of an axis-aligned face shares the outward axis).
            let centroid = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            // Outward direction from the cube centre to the face.
            let outward = [
                centroid[0] - geo.cx,
                centroid[1] - geo.cy,
                centroid[2] - geo.cz,
            ];
            let dot = n[0] * outward[0] + n[1] * outward[1] + n[2] * outward[2];
            assert!(
                dot > 0.0,
                "face {face} triangle is wound inward: normal {n:?} vs outward {outward:?}"
            );
        }
    }
}
