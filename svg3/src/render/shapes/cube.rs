//! svg3 `<cube>` — 3D axis-aligned cuboid geometry resolution and tessellation.
//!
//! `<cube>` is a three-dimensional graphics element ([SPEC.md](../../SPEC.md)
//! §5.2). This module turns a parsed `<cube>` [`Element`] into a filled
//! triangle [`Mesh`] of six rectangular faces (12 triangles, 24 vertices —
//! 4 unique per face so each can carry its own face-local UV) in the svg3
//! world coordinate system: +X right, +Y down, +Z toward the viewer
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

/// Selects how each face of a `<cube>` samples its texture paint server.
///
/// Authored via the `cube-map` attribute (`same` / `cross`); defaults to
/// `same`. Resolved by [`resolve_cube_map`] and consumed by paint resolution
/// in [`crate::render::paint`] to populate the
/// [`crate::render::mesh::PaintServer`] `mapping_kind` / `mapping_aux`
/// uniform fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CubeMap {
    /// Every face shows the whole texture, face-local `(s, t)` → `(u, v)`.
    Same,
    /// 4×3 horizontal-cross atlas, six faces in distinct slots.
    Cross,
}

impl CubeMap {
    /// Default mapping when the `cube-map` attribute is absent.
    pub(crate) const fn default() -> Self {
        Self::Same
    }
}

/// Resolve `<cube cube-map="…">` into a [`CubeMap`].
///
/// Unknown values fall back to the default per SVG attribute-handling
/// convention (don't reject the cube — just lose the per-face layout).
pub(crate) fn resolve_cube_map(element: &Element) -> CubeMap {
    match element.attributes.get("cube-map").map(|value| value.trim()) {
        Some(value) if value.eq_ignore_ascii_case("cross") => CubeMap::Cross,
        _ => CubeMap::default(),
    }
}

/// Tessellate a resolved cube into a filled [`Mesh`] of six rectangular faces.
///
/// Produces **24 vertices** (4 per face) and 36 indices (12 triangles, 2 per
/// face), all tagged [`KIND_SOLID`] — the fragment shader paints each at full
/// coverage in the cube's single `fill` colour. Each face carries its own
/// `(s, t)` UV span in `[0, 1]²` so a texture paint server can sample it
/// (corners can't be shared because the UV at, say, `(−hx, −hy, +hz)` differs
/// between the `+Z`, `−X`, and `−Y` faces). Faces are wound counter-clockwise
/// as seen from outside the cube; future back-face culling can drop the
/// hidden three with no further change to this tessellator.
pub(crate) fn tessellate_cube(geo: &CubeGeometry, color: [f32; 4]) -> Mesh {
    let hx = geo.width * 0.5;
    let hy = geo.height * 0.5;
    let hz = geo.depth * 0.5;

    let position = |sx: f32, sy: f32, sz: f32| -> [f32; 3] {
        [geo.cx + sx * hx, geo.cy + sy * hy, geo.cz + sz * hz]
    };

    // For each face we list four CCW corners (`+Z` outward etc.) starting
    // from the bottom-left of the face's own `(s, t)` frame: index 0 →
    // `(0,0)`, 1 → `(1,0)`, 2 → `(1,1)`, 3 → `(0,1)`. Faces are emitted in
    // the order documented by the `CUBE_FACE_*` constants.
    const FACE_UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut vertices: Vec<Vertex> = Vec::with_capacity(24);
    let mut indices: Vec<u32> = Vec::with_capacity(36);
    let mut push_face = |corners: [[f32; 3]; 4]| {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.iter().zip(FACE_UVS.iter()) {
            vertices.push(Vertex {
                position: *corner,
                color,
                local: [0.0, 0.0],
                params: [0.0; 4],
                kind: KIND_SOLID,
                paint_id: 0,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    // For each face, the 4 corners are listed in CCW-from-outside order so
    // the first triangle's cross product points outward, AND they walk the
    // face's screen-natural (TL → TR → BR → BL) order so the
    // `FACE_UVS = [(0,0),(1,0),(1,1),(0,1)]` table maps the texture upright
    // when viewed from the outside-normal direction.
    //
    // `+Z` (front), viewer at +Z looking toward -Z; screen-right = +X,
    // screen-down = +Y (SVG already y-down).
    push_face([
        position(-1.0, -1.0, 1.0),
        position(1.0, -1.0, 1.0),
        position(1.0, 1.0, 1.0),
        position(-1.0, 1.0, 1.0),
    ]);
    // `-Z` (back), viewer at -Z looking toward +Z; screen-right = -X (the
    // back of the cube is mirrored from the front), screen-down = +Y.
    push_face([
        position(1.0, -1.0, -1.0),
        position(-1.0, -1.0, -1.0),
        position(-1.0, 1.0, -1.0),
        position(1.0, 1.0, -1.0),
    ]);
    // `-Y` (top in screen, since SVG y is down), viewer above looking down;
    // screen-right = +X, screen-down = +Z (forward into the scene).
    push_face([
        position(-1.0, -1.0, -1.0),
        position(1.0, -1.0, -1.0),
        position(1.0, -1.0, 1.0),
        position(-1.0, -1.0, 1.0),
    ]);
    // `+Y` (bottom in screen), viewer below looking up; screen-right = +X,
    // screen-down = -Z (back of scene reads as "below" looking up).
    push_face([
        position(-1.0, 1.0, 1.0),
        position(1.0, 1.0, 1.0),
        position(1.0, 1.0, -1.0),
        position(-1.0, 1.0, -1.0),
    ]);
    // `-X` (left), viewer on the -X side looking toward +X; screen-right =
    // +Z (front of scene reads to the right from this viewpoint),
    // screen-down = +Y.
    push_face([
        position(-1.0, -1.0, -1.0),
        position(-1.0, -1.0, 1.0),
        position(-1.0, 1.0, 1.0),
        position(-1.0, 1.0, -1.0),
    ]);
    // `+X` (right), viewer on the +X side looking toward -X; screen-right =
    // -Z, screen-down = +Y.
    push_face([
        position(1.0, -1.0, 1.0),
        position(1.0, -1.0, -1.0),
        position(1.0, 1.0, -1.0),
        position(1.0, 1.0, 1.0),
    ]);

    Mesh::new(vertices, indices)
}

/// Face index in the order `tessellate_cube` emits faces. Used by the
/// paint resolver to pack the cube-cross atlas slot into each face's UV
/// span before the GPU buffer is built.
///
/// Order: `+Z` (front), `-Z` (back), `-Y` (top), `+Y` (bottom),
/// `-X` (left), `+X` (right).
pub(crate) const CUBE_FACE_COUNT: usize = 6;
pub(crate) const CUBE_FACE_PLUS_Z: usize = 0;
pub(crate) const CUBE_FACE_MINUS_Z: usize = 1;
pub(crate) const CUBE_FACE_MINUS_Y: usize = 2;
pub(crate) const CUBE_FACE_PLUS_Y: usize = 3;
pub(crate) const CUBE_FACE_MINUS_X: usize = 4;
pub(crate) const CUBE_FACE_PLUS_X: usize = 5;

/// `(slot_x, slot_y)` of each face in the 4×3 horizontal-cross atlas:
///
/// ```text
///   .  +Y  .  .
///  -X  +Z +X -Z
///   .  -Y  .  .
/// ```
///
/// Indexed by the `CUBE_FACE_*` constants above.
pub(crate) const CUBE_CROSS_SLOTS: [(u32, u32); CUBE_FACE_COUNT] = {
    let mut slots = [(0, 0); CUBE_FACE_COUNT];
    slots[CUBE_FACE_PLUS_Z] = (1, 1);
    slots[CUBE_FACE_MINUS_Z] = (3, 1);
    slots[CUBE_FACE_MINUS_Y] = (1, 0);
    slots[CUBE_FACE_PLUS_Y] = (1, 2);
    slots[CUBE_FACE_MINUS_X] = (0, 1);
    slots[CUBE_FACE_PLUS_X] = (2, 1);
    slots
};

/// Per-face vertex range emitted by [`tessellate_cube`]: face `i` owns
/// `vertices[i * 4 .. (i + 1) * 4]`. Helper for the paint resolver, which
/// remaps each face's UVs into the cube-cross atlas slot when
/// `cube-map="cross"`.
pub(crate) const CUBE_VERTS_PER_FACE: u32 = 4;

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
    fn resolve_cube_map_defaults_to_same_and_recognises_cross() {
        assert_eq!(resolve_cube_map(&cube(&[])), CubeMap::Same);
        assert_eq!(
            resolve_cube_map(&cube(&[("cube-map", "same")])),
            CubeMap::Same
        );
        assert_eq!(
            resolve_cube_map(&cube(&[("cube-map", "cross")])),
            CubeMap::Cross
        );
        // Unknown values fall back to the default rather than rejecting the
        // cube — matches SVG's general "ignore unknown attribute value"
        // convention.
        assert_eq!(
            resolve_cube_map(&cube(&[("cube-map", "unknown")])),
            CubeMap::Same
        );
    }

    #[test]
    fn tessellate_cube_has_twenty_four_corners_and_twelve_triangles() {
        let geo = CubeGeometry {
            cx: 50.0,
            cy: 50.0,
            cz: 0.0,
            width: 20.0,
            height: 20.0,
            depth: 20.0,
        };
        let mesh = tessellate_cube(&geo, [0.0, 0.0, 1.0, 1.0]);
        // 4 vertices per face × 6 faces, so the per-face UVs can differ.
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
        assert!(mesh
            .indices
            .iter()
            .all(|&i| (i as usize) < mesh.vertices.len()));
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
    fn tessellate_cube_emits_each_face_uv_corner_once() {
        let geo = CubeGeometry {
            cx: 0.0,
            cy: 0.0,
            cz: 0.0,
            width: 2.0,
            height: 2.0,
            depth: 2.0,
        };
        let mesh = tessellate_cube(&geo, [1.0; 4]);
        // Each face contributes exactly the four UV corners (0,0)/(1,0)/
        // (1,1)/(0,1); 6 faces × 4 unique UVs = the same multiset of 24.
        let mut uv_counts = [0u32; 4];
        for v in &mesh.vertices {
            let slot = match (v.uv[0], v.uv[1]) {
                (u, vv) if u == 0.0 && vv == 0.0 => 0,
                (u, vv) if u == 1.0 && vv == 0.0 => 1,
                (u, vv) if u == 1.0 && vv == 1.0 => 2,
                (u, vv) if u == 0.0 && vv == 1.0 => 3,
                other => panic!("unexpected uv corner {other:?}"),
            };
            uv_counts[slot] += 1;
        }
        assert_eq!(uv_counts, [6, 6, 6, 6]);
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
        // The 24 corners are 4 copies of each of the 8 unit-cube corners.
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
        for face in 0..CUBE_FACE_COUNT {
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
