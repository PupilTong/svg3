//! svg3 `<ellipsoid>` — 3D ellipsoid geometry resolution and tessellation.
//!
//! `<ellipsoid>` is a three-dimensional graphics element
//! ([SPEC.md](../../SPEC.md) §5.3). This module turns a parsed `<ellipsoid>`
//! [`Element`] into a filled triangle [`Mesh`] approximating the implicit
//! surface
//!
//! ```text
//! ((x − cx) / rx)² + ((y − cy) / ry)² + ((z − cz) / rz)² ≤ 1
//! ```
//!
//! in the svg3 world coordinate system: +X right, +Y down, +Z toward the
//! viewer ([SPEC.md](../../SPEC.md) §3.1). Like [`crate::shapes::cube`], the
//! geometry leaves the plane `z = 0`, so the [`Camera`](crate::Camera) is
//! the natural way to view it; the default orthographic projection collapses
//! the ellipsoid to its axis-aligned bounding ellipse projection.
//!
//! The surface is approximated as a UV-parameterised mesh: [`LATITUDE_BANDS`]
//! latitude rings between the poles × [`LONGITUDE_SEGMENTS`] longitude
//! segments around the Z axis. Triangles are wound counter-clockwise as seen
//! from outside, so a future back-face culling pass discards the hidden
//! hemisphere; with a single uniform [`Element::fill`](svg3_dom::Element)
//! colour today, the ellipsoid renders as its outward elliptical silhouette
//! in any view — the occluded back hemisphere overdraws the visible front
//! with the same colour, so the end result is the same union of triangles.
//!
//! Length parsing and `fill` resolution are shared with the 2D basic shapes
//! and [`crate::shapes::cube`] — see [`crate::shapes`]. `transform` and
//! grouping are not applied yet, matching the rest of
//! [`crate::scene::build_scene`].

use svg3_dom::Element;

use super::{resolve_length, Length, Viewport, KIND_SOLID};
use crate::{Mesh, Vertex};

/// Number of latitude rings between (and including) the two poles. The
/// surface contains `LATITUDE_BANDS - 1` rings of quads stacked between
/// pole singularities, giving 16 bands of quads at the default resolution.
const LATITUDE_BANDS: u32 = 17;

/// Number of longitude segments around the Z axis. 32 segments produce a
/// silhouette that's visually smooth for typical 50–200 user-unit radii on
/// a square 100×100 viewport.
const LONGITUDE_SEGMENTS: u32 = 32;

/// An `<ellipsoid>`'s geometry after [SPEC.md](../../SPEC.md) §5.3
/// defaulting. All values are in svg3 user units; radii are the
/// half-extent along each axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EllipsoidGeometry {
    /// Centre x coordinate.
    pub cx: f32,
    /// Centre y coordinate.
    pub cy: f32,
    /// Centre z coordinate.
    pub cz: f32,
    /// Half-extent along X — always `> 0` for a resolved geometry.
    pub rx: f32,
    /// Half-extent along Y — always `> 0` for a resolved geometry.
    pub ry: f32,
    /// Half-extent along Z — always `> 0` for a resolved geometry.
    pub rz: f32,
}

/// Resolve an `<ellipsoid>`'s raw attributes into an [`EllipsoidGeometry`].
///
/// Percentage lengths resolve against `viewport` per axis — `cx` and `rx`
/// against its width, `cy` and `ry` against its height, `cz` and `rz`
/// against its normalized diagonal (the same isotropic basis SVG 1.1 uses
/// for `<circle>`'s `r`, since the Z axis is neither horizontal nor
/// vertical).
///
/// Returns `None` when the ellipsoid is not rendered. Per
/// [SPEC.md](../../SPEC.md) §5.3, a negative value for `'r'`, `'rx'`,
/// `'ry'`, or `'rz'` is an error — the element renders as if it had not
/// been specified. A zero radius on any axis also disables rendering.
/// `cx`/`cy`/`cz` default to `0`; an unspecified `rx`/`ry`/`rz` defaults to
/// the `'r'` length token if present (so `r="50%"` then resolves per axis),
/// or zero if neither is given.
pub(crate) fn resolve_ellipsoid(
    element: &Element,
    viewport: Viewport,
) -> Option<EllipsoidGeometry> {
    let cx = resolve_length(element, "cx", viewport.width).unwrap_or(0.0);
    let cy = resolve_length(element, "cy", viewport.height).unwrap_or(0.0);
    let cz = resolve_length(element, "cz", viewport.diagonal()).unwrap_or(0.0);

    let r = parse_length_attr(element, "r");
    let rx_attr = parse_length_attr(element, "rx");
    let ry_attr = parse_length_attr(element, "ry");
    let rz_attr = parse_length_attr(element, "rz");

    // SPEC §5.3: a negative value for any of r/rx/ry/rz is an error; the
    // ellipsoid renders as if not specified.
    let any_negative = [r, rx_attr, ry_attr, rz_attr]
        .iter()
        .any(|opt| opt.is_some_and(Length::is_negative));
    if any_negative {
        return None;
    }

    // Per-axis attribute falls back to the `r` length token (so a
    // percentage `r` resolves against each axis's own basis), then to zero.
    let resolve = |attr: Option<Length>, basis: f32| -> f32 {
        attr.or(r).map(|l| l.resolve(basis)).unwrap_or(0.0)
    };
    let rx = resolve(rx_attr, viewport.width);
    let ry = resolve(ry_attr, viewport.height);
    let rz = resolve(rz_attr, viewport.diagonal());

    // SPEC §5.3: zero radius on any axis disables rendering.
    if rx <= 0.0 || ry <= 0.0 || rz <= 0.0 {
        return None;
    }

    Some(EllipsoidGeometry {
        cx,
        cy,
        cz,
        rx,
        ry,
        rz,
    })
}

/// Tessellate a resolved ellipsoid into a filled [`Mesh`].
///
/// Produces [`LATITUDE_BANDS`] × ([`LONGITUDE_SEGMENTS`] + 1) vertices and
/// `(LATITUDE_BANDS - 1) * LONGITUDE_SEGMENTS * 2` triangles, all tagged
/// [`KIND_SOLID`] — the fragment shader paints each at full coverage in the
/// ellipsoid's single `fill` colour.
///
/// The surface is parameterised by latitude `θ ∈ [-π/2, π/2]` (south
/// pole → north pole, mapped to the Z axis) and longitude `φ ∈ [0, 2π]`
/// around the Z axis:
///
/// ```text
/// x = cx + rx · cos(θ) · cos(φ)
/// y = cy + ry · cos(θ) · sin(φ)
/// z = cz + rz · sin(θ)
/// ```
///
/// Triangles are wound counter-clockwise as seen from outside the
/// ellipsoid; future back-face culling can drop the hidden hemisphere with
/// no further change to this tessellator. Pole vertices are duplicated per
/// longitude segment so the seam at `φ = 0 / 2π` stays consistent — the
/// degenerate triangles those duplicates produce contribute zero area.
pub(crate) fn tessellate_ellipsoid(geo: &EllipsoidGeometry, color: [f32; 4]) -> Mesh {
    let row_size = LONGITUDE_SEGMENTS + 1;
    let mut vertices = Vec::with_capacity((LATITUDE_BANDS * row_size) as usize);

    for lat in 0..LATITUDE_BANDS {
        // θ ∈ [-π/2, π/2] from south pole (z = cz - rz) to north pole
        // (z = cz + rz).
        let theta = (lat as f32 / (LATITUDE_BANDS - 1) as f32) * std::f32::consts::PI
            - std::f32::consts::FRAC_PI_2;
        let (sin_theta, cos_theta) = theta.sin_cos();
        for long in 0..=LONGITUDE_SEGMENTS {
            // φ ∈ [0, 2π] around the Z axis. The seam at φ = 0 / 2π is
            // duplicated so the U coordinate (if we ever add one) wraps
            // cleanly; here it just keeps the index arithmetic uniform.
            let phi = (long as f32 / LONGITUDE_SEGMENTS as f32) * std::f32::consts::TAU;
            let (sin_phi, cos_phi) = phi.sin_cos();
            vertices.push(Vertex {
                position: [
                    geo.cx + geo.rx * cos_theta * cos_phi,
                    geo.cy + geo.ry * cos_theta * sin_phi,
                    geo.cz + geo.rz * sin_theta,
                ],
                color,
                local: [0.0, 0.0],
                params: [0.0; 4],
                kind: KIND_SOLID,
                paint_id: 0,
            });
        }
    }

    // Triangulate each quad with the diagonal from `(lat, long)` to
    // `(lat + 1, long + 1)`. Winding `(a, b, c)` then `(b, d, c)` where
    // `a = (lat, long)`, `b = (lat, long + 1)`, `c = (lat + 1, long)`,
    // `d = (lat + 1, long + 1)` gives each triangle an outward-facing
    // normal (verified by `tessellate_winds_each_band_outward`).
    let mut indices = Vec::with_capacity(((LATITUDE_BANDS - 1) * LONGITUDE_SEGMENTS * 6) as usize);
    for lat in 0..LATITUDE_BANDS - 1 {
        for long in 0..LONGITUDE_SEGMENTS {
            let a = lat * row_size + long;
            let b = lat * row_size + long + 1;
            let c = (lat + 1) * row_size + long;
            let d = (lat + 1) * row_size + long + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }

    Mesh::new(vertices, indices)
}

/// Parse a named SVG length attribute into a [`Length`], without resolving
/// it against any basis. Returns `None` for an absent or unparseable value.
/// Sign is preserved so a caller can flag negative inputs per
/// [SPEC.md](../../SPEC.md) §5.3 before resolution.
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
    use svg3_dom::ElementKind;

    /// Build an `<ellipsoid>` element carrying the given raw attributes.
    fn ellipsoid(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Ellipsoid);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    /// A 100×100 viewport. Most geometry tests use absolute lengths, so the
    /// viewport value only matters for the percentage cases.
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn resolve_ellipsoid_applies_r_to_all_three_axes() {
        // SPEC §5.3: `r` is the default for `rx`/`ry`/`rz`.
        let geo = resolve_ellipsoid(&ellipsoid(&[("r", "20")]), vp()).unwrap();
        assert_eq!(
            geo,
            EllipsoidGeometry {
                cx: 0.0,
                cy: 0.0,
                cz: 0.0,
                rx: 20.0,
                ry: 20.0,
                rz: 20.0,
            }
        );
    }

    #[test]
    fn resolve_ellipsoid_applies_centre_defaults() {
        // SPEC §5.3: `cx`/`cy`/`cz` default to 0.
        let geo = resolve_ellipsoid(&ellipsoid(&[("r", "10")]), vp()).unwrap();
        assert_eq!((geo.cx, geo.cy, geo.cz), (0.0, 0.0, 0.0));
    }

    #[test]
    fn resolve_ellipsoid_lets_per_axis_attrs_override_r() {
        // Per SPEC §5.3 `rx`/`ry`/`rz` shadow `r` on their own axis; the
        // others still inherit it.
        let geo = resolve_ellipsoid(
            &ellipsoid(&[
                ("cx", "10"),
                ("cy", "20"),
                ("cz", "30"),
                ("r", "40"),
                ("rx", "12"),
            ]),
            vp(),
        )
        .unwrap();
        assert_eq!(
            geo,
            EllipsoidGeometry {
                cx: 10.0,
                cy: 20.0,
                cz: 30.0,
                rx: 12.0,
                ry: 40.0,
                rz: 40.0,
            }
        );
    }

    #[test]
    fn resolve_ellipsoid_accepts_independent_axis_radii() {
        // With no `r`, each axis is set directly.
        let geo = resolve_ellipsoid(
            &ellipsoid(&[("rx", "10"), ("ry", "20"), ("rz", "30")]),
            vp(),
        )
        .unwrap();
        assert_eq!((geo.rx, geo.ry, geo.rz), (10.0, 20.0, 30.0));
    }

    #[test]
    fn resolve_ellipsoid_resolves_percentages_per_axis() {
        // Width-axis lengths resolve against viewport width, height-axis
        // against height; the Z axis (neither horizontal nor vertical) uses
        // the normalized diagonal — the same isotropic basis as
        // `<circle>`'s `r`.
        let viewport = Viewport {
            width: 200.0,
            height: 100.0,
        };
        let geo = resolve_ellipsoid(
            &ellipsoid(&[
                ("cx", "50%"),
                ("cy", "25%"),
                ("rx", "50%"),
                ("ry", "20%"),
                ("rz", "50%"),
            ]),
            viewport,
        )
        .unwrap();
        assert_eq!(geo.cx, 100.0);
        assert_eq!(geo.cy, 25.0);
        assert_eq!(geo.rx, 100.0);
        assert_eq!(geo.ry, 20.0);
        assert!((geo.rz - viewport.diagonal() * 0.5).abs() < 1e-4);
    }

    #[test]
    fn resolve_ellipsoid_inherits_r_length_token_per_axis() {
        // Per SPEC §5.3 an unspecified per-axis attribute defaults to the
        // `r` *length token* (not its resolved pixels), so `r="50%"`
        // resolves separately on each axis — width vs height vs diagonal.
        let viewport = Viewport {
            width: 200.0,
            height: 80.0,
        };
        let geo = resolve_ellipsoid(&ellipsoid(&[("r", "50%")]), viewport).unwrap();
        assert_eq!(geo.rx, 100.0); // 50% of viewport.width
        assert_eq!(geo.ry, 40.0); // 50% of viewport.height
        assert!((geo.rz - viewport.diagonal() * 0.5).abs() < 1e-4);
    }

    #[test]
    fn resolve_ellipsoid_skips_negative_radii() {
        // SPEC §5.3: a negative value for any of `r`/`rx`/`ry`/`rz` is an
        // error; the ellipsoid renders as if not specified.
        assert_eq!(resolve_ellipsoid(&ellipsoid(&[("r", "-1")]), vp()), None);
        assert_eq!(
            resolve_ellipsoid(
                &ellipsoid(&[("rx", "-1"), ("ry", "10"), ("rz", "10")]),
                vp()
            ),
            None
        );
        assert_eq!(
            resolve_ellipsoid(
                &ellipsoid(&[("rx", "10"), ("ry", "-1"), ("rz", "10")]),
                vp()
            ),
            None
        );
        assert_eq!(
            resolve_ellipsoid(
                &ellipsoid(&[("rx", "10"), ("ry", "10"), ("rz", "-1")]),
                vp()
            ),
            None
        );
    }

    #[test]
    fn resolve_ellipsoid_skips_zero_or_missing_axis() {
        // SPEC §5.3: zero `rx`/`ry`/`rz` disables rendering, and an
        // unspecified axis with no `r` falls through to zero.
        assert_eq!(resolve_ellipsoid(&ellipsoid(&[]), vp()), None);
        assert_eq!(
            resolve_ellipsoid(&ellipsoid(&[("rx", "10"), ("ry", "10")]), vp()),
            None
        );
        assert_eq!(resolve_ellipsoid(&ellipsoid(&[("r", "0")]), vp()), None);
        assert_eq!(
            resolve_ellipsoid(&ellipsoid(&[("r", "10"), ("rz", "0")]), vp()),
            None
        );
    }

    #[test]
    fn resolve_ellipsoid_ignores_unparseable_centre_coordinate() {
        // An unparseable `cx`/`cy`/`cz` falls back to its `0` default, just
        // as on the 2D shapes — it does not disable an otherwise-valid
        // ellipsoid.
        let geo = resolve_ellipsoid(&ellipsoid(&[("r", "10"), ("cx", "nope")]), vp()).unwrap();
        assert_eq!((geo.cx, geo.cy, geo.cz), (0.0, 0.0, 0.0));
    }

    #[test]
    fn tessellate_produces_expected_vertex_and_index_counts() {
        let geo = EllipsoidGeometry {
            cx: 50.0,
            cy: 50.0,
            cz: 0.0,
            rx: 20.0,
            ry: 20.0,
            rz: 20.0,
        };
        let mesh = tessellate_ellipsoid(&geo, [0.0, 0.0, 1.0, 1.0]);
        let row_size = (LONGITUDE_SEGMENTS + 1) as usize;
        assert_eq!(mesh.vertices.len(), LATITUDE_BANDS as usize * row_size);
        assert_eq!(
            mesh.indices.len(),
            (LATITUDE_BANDS - 1) as usize * LONGITUDE_SEGMENTS as usize * 6
        );
        assert!(mesh
            .indices
            .iter()
            .all(|&i| (i as usize) < mesh.vertices.len()));
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.kind == KIND_SOLID && v.color == [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn tessellate_keeps_vertices_on_the_ellipsoid_surface() {
        // Every emitted vertex must satisfy the implicit-surface equation
        // `((x − cx) / rx)² + ((y − cy) / ry)² + ((z − cz) / rz)² = 1`
        // exactly (within float precision), regardless of latitude /
        // longitude index — the tessellator is sampling the analytic
        // surface, not approximating it.
        let geo = EllipsoidGeometry {
            cx: 12.5,
            cy: 8.0,
            cz: -3.0,
            rx: 15.0,
            ry: 9.0,
            rz: 7.0,
        };
        let mesh = tessellate_ellipsoid(&geo, [1.0; 4]);
        for v in &mesh.vertices {
            let nx = (v.position[0] - geo.cx) / geo.rx;
            let ny = (v.position[1] - geo.cy) / geo.ry;
            let nz = (v.position[2] - geo.cz) / geo.rz;
            let r2 = nx * nx + ny * ny + nz * nz;
            assert!(
                (r2 - 1.0).abs() < 1e-4,
                "vertex {v:?} off the surface (r² = {r2})"
            );
        }
    }

    #[test]
    fn tessellate_winds_each_band_outward() {
        // Each triangle's `(b − a) × (c − a)` cross product points outward
        // (positive dot product with the triangle centroid's offset from
        // the ellipsoid centre). This pins the convention so a future
        // back-face culling pipeline drops the hidden hemisphere.
        //
        // Triangles near the poles can be degenerate (the duplicated pole
        // vertices collapse one edge to zero length), so a zero or barely
        // negative dot product around the poles is treated as a tie.
        let geo = EllipsoidGeometry {
            cx: 0.0,
            cy: 0.0,
            cz: 0.0,
            rx: 1.0,
            ry: 1.0,
            rz: 1.0,
        };
        let mesh = tessellate_ellipsoid(&geo, [1.0; 4]);
        for triangle in mesh.indices.chunks_exact(3) {
            let a = mesh.vertices[triangle[0] as usize].position;
            let b = mesh.vertices[triangle[1] as usize].position;
            let c = mesh.vertices[triangle[2] as usize].position;
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let centroid = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let dot = normal[0] * centroid[0] + normal[1] * centroid[1] + normal[2] * centroid[2];
            assert!(
                dot >= -1e-5,
                "triangle is wound inward: normal {normal:?} vs centroid {centroid:?}"
            );
        }
    }
}
