//! svg3 `<cylinder>` — right elliptical cylinder geometry resolution and tessellation.
//!
//! `<cylinder>` is a three-dimensional graphics element
//! ([SPEC.md](../../SPEC.md) §5.5). This module turns a parsed `<cylinder>`
//! [`Element`] into a filled triangle [`Mesh`] approximating a right
//! elliptical cylinder aligned to the Z axis in the svg3 world coordinate
//! system: +X right, +Y down, +Z toward the viewer ([SPEC.md](../../SPEC.md)
//! §3.1). The default orthographic projection collapses the cylinder to its
//! front cap; a [`Camera`](crate::Camera) reveals the Z-axis depth.
//!
//! The surface is approximated as two triangle-fan caps plus a segmented side
//! wall. Triangles are wound counter-clockwise as seen from outside, matching
//! [`crate::render::shapes::cube`] and [`crate::render::shapes::ellipsoid`].
//! With a single uniform [`Element::fill`](crate::dom::Element) colour today,
//! hidden faces overdraw visible faces with the same colour, so the rendered
//! result is the same outward silhouette.
//!
//! Length parsing and `fill` resolution are shared with the 2D basic shapes
//! and the other 3D primitives — see [`crate::render::shapes`]. `transform`
//! and grouping are not applied yet, matching the rest of
//! [`crate::render::scene::build_scene`].

use crate::dom::Element;

use super::{resolve_length, Length, Viewport, KIND_SOLID};
use crate::render::{Mesh, Vertex};

/// Number of radial segments used for the caps and side wall.
const RADIAL_SEGMENTS: u32 = 32;

/// A `<cylinder>`'s geometry after [SPEC.md](../../SPEC.md) §5.5 defaulting.
/// All values are in svg3 user units; `rx`/`ry` are cap radii and `depth` is
/// the full extent along Z.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CylinderGeometry {
    /// Centre x coordinate.
    pub cx: f32,
    /// Centre y coordinate.
    pub cy: f32,
    /// Centre z coordinate.
    pub cz: f32,
    /// Cap radius along X — always `> 0` for a resolved geometry.
    pub rx: f32,
    /// Cap radius along Y — always `> 0` for a resolved geometry.
    pub ry: f32,
    /// Full extent along Z — always `> 0` for a resolved geometry.
    pub depth: f32,
}

/// Resolve a `<cylinder>`'s raw attributes into a [`CylinderGeometry`].
///
/// Percentage lengths resolve against `viewport` per axis — `cx` and `rx`
/// against its width, `cy` and `ry` against its height, `cz` and `depth`
/// against its normalized diagonal (the same isotropic basis SVG 1.1 uses
/// for `<circle>`'s `r`, since the Z axis is neither horizontal nor
/// vertical).
///
/// Returns `None` when the cylinder is not rendered. Per
/// [SPEC.md](../../SPEC.md) §5.5, a negative value for `'r'`, `'rx'`,
/// `'ry'`, or `'depth'` is an error — the element renders as if it had not
/// been specified. A zero cap radius or zero depth also disables rendering.
/// `cx`/`cy`/`cz` default to `0`; an unspecified `rx`/`ry` defaults to the
/// `'r'` length token if present. When `depth` is omitted but `r` is present,
/// `depth` defaults to twice `r` resolved on the Z axis, giving
/// `<cylinder r="…">` a visible, diameter-tall circular cylinder.
pub(crate) fn resolve_cylinder(element: &Element, viewport: Viewport) -> Option<CylinderGeometry> {
    let cx = resolve_length(element, "cx", viewport.width).unwrap_or(0.0);
    let cy = resolve_length(element, "cy", viewport.height).unwrap_or(0.0);
    let cz = resolve_length(element, "cz", viewport.diagonal()).unwrap_or(0.0);

    let r = parse_length_attr(element, "r");
    let rx_attr = parse_length_attr(element, "rx");
    let ry_attr = parse_length_attr(element, "ry");
    let depth_attr = parse_length_attr(element, "depth");

    // SPEC §5.5: a negative value for any of r/rx/ry/depth is an error; the
    // cylinder renders as if not specified.
    let any_negative = [r, rx_attr, ry_attr, depth_attr]
        .iter()
        .any(|opt| opt.is_some_and(Length::is_negative));
    if any_negative {
        return None;
    }

    let resolve = |attr: Option<Length>, fallback: Option<Length>, basis: f32| -> f32 {
        attr.or(fallback)
            .map(|length| length.resolve(basis))
            .unwrap_or(0.0)
    };
    let rx = resolve(rx_attr, r, viewport.width);
    let ry = resolve(ry_attr, r, viewport.height);
    let depth = depth_attr
        .map(|length| length.resolve(viewport.diagonal()))
        .or_else(|| r.map(|length| length.resolve(viewport.diagonal()) * 2.0))
        .unwrap_or(0.0);

    // SPEC §5.5: zero on any axis disables rendering.
    if rx <= 0.0 || ry <= 0.0 || depth <= 0.0 {
        return None;
    }

    Some(CylinderGeometry {
        cx,
        cy,
        cz,
        rx,
        ry,
        depth,
    })
}

/// Tessellate a resolved cylinder into filled cap and side triangles.
///
/// Three regions share the geometry but carry independent UV spans for a
/// texture paint server:
///
/// - **Side wall** — duplicated `(front, back)` ring rows, `u = φ / 2π`
///   wrapping around, `v` from `0` at the back cap to `1` at the front cap.
/// - **Top cap** (front, `z = z_max`) — polar disk projection: cap centre →
///   texture centre `(0.5, 0.5)`, cap rim → texture edges. The disk's `+X`
///   on the cap goes to the texture's `+u`; SVG `+Y` (down) goes to texture
///   `+v` (so `v = 0.5 − 0.5 · y'`).
/// - **Bottom cap** (back, `z = z_min`) — same polar disk with `x` flipped so
///   the texture reads upright when viewed from `-Z` (looking up at the
///   back cap mirrors looking down at the front cap).
///
/// Cap centres and ring vertices are *not* shared between the cap and side
/// regions: each ring vertex is duplicated so the cap row can carry its
/// disk UV while the side row carries its wrap UV. All vertices are tagged
/// [`KIND_SOLID`]. The front cap is at `cz + depth / 2` and the back cap at
/// `cz - depth / 2`, following svg3's +Z-toward-viewer convention.
pub(crate) fn tessellate_cylinder(geo: &CylinderGeometry, color: [f32; 4]) -> Mesh {
    let front_z = geo.cz + geo.depth * 0.5;
    let back_z = geo.cz - geo.depth * 0.5;

    // Layout: 2 cap centres + 2 cap rings + 2 side rings, each
    // `RADIAL_SEGMENTS + 1` long.
    let ring_len = (RADIAL_SEGMENTS + 1) as usize;
    let mut vertices = Vec::with_capacity(2 + 4 * ring_len);
    // 0: front cap centre.
    vertices.push(textured_vertex(
        [geo.cx, geo.cy, front_z],
        color,
        [0.5, 0.5],
    ));
    // 1: back cap centre.
    vertices.push(textured_vertex([geo.cx, geo.cy, back_z], color, [0.5, 0.5]));

    let front_ring_base = vertices.len() as u32;
    // Cap rings — UV is the polar disk projection of the rim point.
    for segment in 0..=RADIAL_SEGMENTS {
        let phi = (segment as f32 / RADIAL_SEGMENTS as f32) * std::f32::consts::TAU;
        let (sin_phi, cos_phi) = phi.sin_cos();
        let x = geo.cx + geo.rx * cos_phi;
        let y = geo.cy + geo.ry * sin_phi;
        // Polar disk: `x' = cos_phi`, `y' = sin_phi` (the unit-rim point on
        // the cap-local frame). Front-cap UV maps `+x' → +u`, `+y' → +v`
        // (SVG y is already screen-down).
        let front_u = 0.5 + 0.5 * cos_phi;
        let front_v = 0.5 + 0.5 * sin_phi;
        vertices.push(textured_vertex([x, y, front_z], color, [front_u, front_v]));
    }
    let back_ring_base = vertices.len() as u32;
    for segment in 0..=RADIAL_SEGMENTS {
        let phi = (segment as f32 / RADIAL_SEGMENTS as f32) * std::f32::consts::TAU;
        let (sin_phi, cos_phi) = phi.sin_cos();
        let x = geo.cx + geo.rx * cos_phi;
        let y = geo.cy + geo.ry * sin_phi;
        // Back cap UV mirrors `x'` so the texture reads upright when the
        // cylinder is viewed from `-Z`.
        let back_u = 0.5 - 0.5 * cos_phi;
        let back_v = 0.5 + 0.5 * sin_phi;
        vertices.push(textured_vertex([x, y, back_z], color, [back_u, back_v]));
    }
    // Side wall rings — UV is the equirectangular wrap (`u = φ / 2π`,
    // `v ∈ {0, 1}` for back / front).
    let side_front_base = vertices.len() as u32;
    for segment in 0..=RADIAL_SEGMENTS {
        let segment_t = segment as f32 / RADIAL_SEGMENTS as f32;
        let phi = segment_t * std::f32::consts::TAU;
        let (sin_phi, cos_phi) = phi.sin_cos();
        let x = geo.cx + geo.rx * cos_phi;
        let y = geo.cy + geo.ry * sin_phi;
        // Side-wall front row: `v = 1.0` (front cap edge maps to `v = 1`).
        vertices.push(textured_vertex([x, y, front_z], color, [segment_t, 1.0]));
    }
    let side_back_base = vertices.len() as u32;
    for segment in 0..=RADIAL_SEGMENTS {
        let segment_t = segment as f32 / RADIAL_SEGMENTS as f32;
        let phi = segment_t * std::f32::consts::TAU;
        let (sin_phi, cos_phi) = phi.sin_cos();
        let x = geo.cx + geo.rx * cos_phi;
        let y = geo.cy + geo.ry * sin_phi;
        // Side-wall back row: `v = 0.0`.
        vertices.push(textured_vertex([x, y, back_z], color, [segment_t, 0.0]));
    }

    let mut indices = Vec::with_capacity((RADIAL_SEGMENTS * 12) as usize);
    for segment in 0..RADIAL_SEGMENTS {
        let next = segment + 1;
        let front_a = front_ring_base + segment;
        let front_b = front_ring_base + next;
        let back_a = back_ring_base + segment;
        let back_b = back_ring_base + next;
        // Cap windings: front cap normal +Z; back cap normal -Z.
        indices.extend_from_slice(&[0, front_a, front_b, 1, back_b, back_a]);

        // Side wall uses its own ring vertices to carry the wrap UV.
        let side_front_a = side_front_base + segment;
        let side_front_b = side_front_base + next;
        let side_back_a = side_back_base + segment;
        let side_back_b = side_back_base + next;
        indices.extend_from_slice(&[
            side_front_a,
            side_back_a,
            side_front_b,
            side_front_b,
            side_back_a,
            side_back_b,
        ]);
    }

    Mesh::new(vertices, indices)
}

fn textured_vertex(position: [f32; 3], color: [f32; 4], uv: [f32; 2]) -> Vertex {
    Vertex {
        position,
        color,
        local: [0.0, 0.0],
        params: [0.0; 4],
        kind: KIND_SOLID,
        paint_id: 0,
        uv,
    }
}

/// Parse a named SVG length attribute into a [`Length`], without resolving
/// it against any basis. Returns `None` for an absent or unparseable value.
/// Sign is preserved so a caller can flag negative inputs per
/// [SPEC.md](../../SPEC.md) §5.5 before resolution.
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

    /// Build a `<cylinder>` element carrying the given raw attributes.
    fn cylinder(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Cylinder);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn resolve_cylinder_applies_r_to_cap_radii_and_diameter_depth() {
        // SPEC §5.5: `r` is the default for `rx`/`ry`, and omitted `depth`
        // becomes twice `r` on the Z-axis basis.
        let geo = resolve_cylinder(&cylinder(&[("r", "20")]), vp()).unwrap();
        assert_eq!(
            geo,
            CylinderGeometry {
                cx: 0.0,
                cy: 0.0,
                cz: 0.0,
                rx: 20.0,
                ry: 20.0,
                depth: 40.0,
            }
        );
    }

    #[test]
    fn resolve_cylinder_applies_centre_defaults() {
        let geo = resolve_cylinder(&cylinder(&[("r", "10")]), vp()).unwrap();
        assert_eq!((geo.cx, geo.cy, geo.cz), (0.0, 0.0, 0.0));
    }

    #[test]
    fn resolve_cylinder_lets_per_axis_attrs_override_r() {
        let geo = resolve_cylinder(
            &cylinder(&[
                ("cx", "10"),
                ("cy", "20"),
                ("cz", "30"),
                ("r", "40"),
                ("rx", "12"),
                ("depth", "30"),
            ]),
            vp(),
        )
        .unwrap();
        assert_eq!(
            geo,
            CylinderGeometry {
                cx: 10.0,
                cy: 20.0,
                cz: 30.0,
                rx: 12.0,
                ry: 40.0,
                depth: 30.0,
            }
        );
    }

    #[test]
    fn resolve_cylinder_accepts_independent_axis_lengths() {
        let geo = resolve_cylinder(
            &cylinder(&[("rx", "10"), ("ry", "20"), ("depth", "30")]),
            vp(),
        )
        .unwrap();
        assert_eq!((geo.rx, geo.ry, geo.depth), (10.0, 20.0, 30.0));
    }

    #[test]
    fn resolve_cylinder_resolves_percentages_per_axis() {
        let viewport = Viewport {
            width: 200.0,
            height: 100.0,
        };
        let geo = resolve_cylinder(
            &cylinder(&[
                ("cx", "50%"),
                ("cy", "25%"),
                ("rx", "50%"),
                ("ry", "20%"),
                ("depth", "50%"),
            ]),
            viewport,
        )
        .unwrap();
        assert_eq!(geo.cx, 100.0);
        assert_eq!(geo.cy, 25.0);
        assert_eq!(geo.rx, 100.0);
        assert_eq!(geo.ry, 20.0);
        assert!((geo.depth - viewport.diagonal() * 0.5).abs() < 1e-4);
    }

    #[test]
    fn resolve_cylinder_inherits_r_length_token_per_axis() {
        let viewport = Viewport {
            width: 200.0,
            height: 80.0,
        };
        let geo = resolve_cylinder(&cylinder(&[("r", "50%")]), viewport).unwrap();
        assert_eq!(geo.rx, 100.0);
        assert_eq!(geo.ry, 40.0);
        assert!((geo.depth - viewport.diagonal()).abs() < 1e-4);
    }

    #[test]
    fn resolve_cylinder_skips_negative_dimensions() {
        assert_eq!(resolve_cylinder(&cylinder(&[("r", "-1")]), vp()), None);
        assert_eq!(
            resolve_cylinder(
                &cylinder(&[("rx", "-1"), ("ry", "10"), ("depth", "10")]),
                vp()
            ),
            None
        );
        assert_eq!(
            resolve_cylinder(
                &cylinder(&[("rx", "10"), ("ry", "-1"), ("depth", "10")]),
                vp()
            ),
            None
        );
        assert_eq!(
            resolve_cylinder(
                &cylinder(&[("rx", "10"), ("ry", "10"), ("depth", "-1")]),
                vp()
            ),
            None
        );
    }

    #[test]
    fn resolve_cylinder_skips_zero_or_missing_axis() {
        assert_eq!(resolve_cylinder(&cylinder(&[]), vp()), None);
        assert_eq!(
            resolve_cylinder(&cylinder(&[("rx", "10"), ("ry", "10")]), vp()),
            None
        );
        assert_eq!(resolve_cylinder(&cylinder(&[("r", "0")]), vp()), None);
        assert_eq!(
            resolve_cylinder(&cylinder(&[("r", "10"), ("depth", "0")]), vp()),
            None
        );
    }

    #[test]
    fn resolve_cylinder_ignores_unparseable_centre_coordinate() {
        let geo = resolve_cylinder(&cylinder(&[("r", "10"), ("cx", "nope")]), vp()).unwrap();
        assert_eq!((geo.cx, geo.cy, geo.cz), (0.0, 0.0, 0.0));
    }

    #[test]
    fn tessellate_produces_caps_and_side_wall() {
        let geo = CylinderGeometry {
            cx: 50.0,
            cy: 50.0,
            cz: 0.0,
            rx: 20.0,
            ry: 20.0,
            depth: 30.0,
        };
        let mesh = tessellate_cylinder(&geo, [0.0, 0.0, 1.0, 1.0]);
        // Each region carries its own ring so cap and side wall can hold
        // independent UVs: 2 cap centres + 4 rings of (RADIAL_SEGMENTS + 1).
        let ring_len = RADIAL_SEGMENTS as usize + 1;
        assert_eq!(mesh.vertices.len(), 2 + 4 * ring_len);
        assert_eq!(mesh.indices.len(), RADIAL_SEGMENTS as usize * 12);
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
    fn tessellate_places_vertices_on_caps_or_side() {
        let geo = CylinderGeometry {
            cx: 12.5,
            cy: 8.0,
            cz: -3.0,
            rx: 15.0,
            ry: 9.0,
            depth: 7.0,
        };
        let mesh = tessellate_cylinder(&geo, [1.0; 4]);
        let front_z = geo.cz + geo.depth * 0.5;
        let back_z = geo.cz - geo.depth * 0.5;
        assert_eq!(mesh.vertices[0].position, [geo.cx, geo.cy, front_z]);
        assert_eq!(mesh.vertices[1].position, [geo.cx, geo.cy, back_z]);
        for v in &mesh.vertices[2..] {
            let nx = (v.position[0] - geo.cx) / geo.rx;
            let ny = (v.position[1] - geo.cy) / geo.ry;
            let r2 = nx * nx + ny * ny;
            assert!(
                (r2 - 1.0).abs() < 1e-4,
                "ring vertex {v:?} off the cap ellipse (r² = {r2})"
            );
            assert!(
                (v.position[2] - front_z).abs() < 1e-4 || (v.position[2] - back_z).abs() < 1e-4,
                "ring vertex {v:?} not on either cap"
            );
        }
    }

    #[test]
    fn tessellate_winds_caps_and_side_wall_outward() {
        let geo = CylinderGeometry {
            cx: 12.5,
            cy: -7.0,
            cz: 4.0,
            rx: 2.0,
            ry: 3.0,
            depth: 4.0,
        };
        let mesh = tessellate_cylinder(&geo, [1.0; 4]);
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
            let outward = [
                centroid[0] - geo.cx,
                centroid[1] - geo.cy,
                centroid[2] - geo.cz,
            ];
            let dot = normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2];
            assert!(
                dot > 0.0,
                "triangle is wound inward: normal {normal:?} vs outward {outward:?}"
            );
        }
    }
}
