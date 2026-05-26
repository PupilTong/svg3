//! svg3 `<surface>` — data-driven 3D surface ([SPEC.md](../../SPEC.md) §5.4).
//!
//! `<surface>` is a three-dimensional graphics element whose geometry is
//! built from a sequence of `<path>` children linked by Bezier patches.
//! The surface's own `'d'` attribute reuses SVG 1.1's path-data
//! mini-language (`M`/`L`/`Q`/`C`/`Z`) but with integer indices into the
//! `<path>` child list in place of `(x, y)` coordinates. Each non-`M`
//! command produces one patch:
//!
//! - `L j`     ↦ ruled (degree-1) patch from current row to path `j`.
//! - `Q i j`   ↦ quadratic Bezier patch through control row `i` to `j`.
//! - `C i j k` ↦ cubic Bezier patch through control rows `i`, `j` to `k`.
//! - `Z`       ↦ final linear patch back to the chain's `M` path.
//! - `P t r b l` ↦ bicubic Coons patch with 4 boundary cubic curves
//!   (top, right, bottom, left). Each referenced child path must be a
//!   single cubic Bezier (`M` + one `C`); the patch's interior is filled
//!   by Coons blending of the 4 boundaries.
//!
//! Each referenced child `<path>` is Lyon-flattened at the shared
//! [`FLATTENING_TOLERANCE`]; vertex `i` of each child is the i-th
//! column's Bezier control point. All referenced children must produce
//! the same polyline vertex count and the same open/closed state — if
//! not, the element is in error and produces no geometry, matching
//! `<cube>`/`<ellipsoid>`'s SPEC §5 error behaviour.
//!
//! Each child `<path>` may carry its own `'transform'` attribute (see
//! [`crate::render::transform`]) lifting its local `z = 0` polyline into 3D. The
//! surface's *own* `'transform'` is not yet composed by the scene walker
//! — that's an svg3-wide gap, see
//! [`crate::render::scene::build_scene`].

use std::collections::{BTreeMap, BTreeSet};

use crate::dom::{Document, Element, ElementKind, NodeId};
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::{Path, PathEvent};

use super::path::parse_path_strict;
use super::stroke::FLATTENING_TOLERANCE;
use super::{Viewport, KIND_SOLID};
use crate::render::transform::parse_transform;
use crate::render::{Mesh, Vertex};

/// Number of sweep samples per patch. The full chain's row count is
/// `K · (PATCH_SAMPLES - 1) + 1` for `K` patches (adjacent patches share
/// their boundary row).
const PATCH_SAMPLES: u32 = 32;

/// Number of samples *per parameter direction* in a Coons-patch
/// tessellation. Each `P` command produces a `(COONS_SAMPLES + 1)²`
/// grid of 3D positions, triangulated as two triangles per cell.
const COONS_SAMPLES: u32 = 16;

/// 2D-distance epsilon for de-duplicating consecutive flattened polyline
/// vertices, matching the value used by [`super::stroke`].
const DEDUP_EPSILON: f32 = 1e-5;

/// 3D-distance epsilon for verifying that adjacent boundary curves of a
/// Coons patch share their corner points within tolerance. Tight enough
/// to catch a typo in an author's coordinate, loose enough that
/// transform-composition rounding is forgiven.
const CORNER_EPSILON: f32 = 1e-3;

/// One sweep patch in the surface's `d`-chain. The chain's *current
/// row* is implicit: it's either the start (`M`) path or the previous
/// patch's endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfacePatch {
    Linear { end: u32 },
    Quadratic { ctrl: u32, end: u32 },
    Cubic { ctrl1: u32, ctrl2: u32, end: u32 },
    Close,
}

/// A bicubic Coons patch declaration, separate from the sweep chain.
/// Each `P` command in the surface `d` becomes one `CoonsPatch`. The
/// four indices reference single-cubic boundary `<path>` children
/// (each with `M` + one `C`) in the order `top, right, bottom, left`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CoonsPatch {
    top: u32,
    right: u32,
    bottom: u32,
    left: u32,
}

/// Resolved tessellation data. A surface may carry an (optional)
/// sweep-chain grid and/or one mesh per `P` Coons patch. The renderer
/// concatenates them into one [`Mesh`] in [`tessellate_surface`].
#[derive(Debug, Clone)]
pub(crate) struct SurfaceGeometry {
    sweep: Option<SweepGrid>,
    coons_meshes: Vec<CoonsMesh>,
}

/// Sweep-chain grid: row-major `total_rows × cols` 3D world-space
/// positions, triangulated as a strip of quads.
#[derive(Debug, Clone)]
struct SweepGrid {
    grid: Vec<[f32; 3]>,
    total_rows: u32,
    cols: u32,
}

/// One tessellated Coons-patch mesh. `(samples + 1)²` positions
/// arranged row-major as the bicubic grid at `(u, v) = (i/samples,
/// j/samples)`.
#[derive(Debug, Clone)]
struct CoonsMesh {
    positions: Vec<[f32; 3]>,
    samples: u32,
}

/// Resolve a `<surface>` element into a tessellation-ready
/// [`SurfaceGeometry`].
///
/// Returns `None` if the element is in error per SPEC §5.4:
/// missing/malformed `d`, command argument out of range, no patches
/// produced, any referenced child's `d`/`transform` fails to parse, or
/// the referenced children's flattened polylines disagree on length or
/// open/closed state. Non-`<path>` children of the surface are ignored
/// for indexing (the k-th `<path>` child has index k regardless of any
/// non-path siblings between them).
///
/// `viewport` is accepted for parity with the other shapes' resolvers;
/// the v0 attributes are unitless coordinates so it is not consulted.
pub(crate) fn resolve_surface(
    document: &Document,
    surface_id: NodeId,
    _viewport: Viewport,
) -> Option<SurfaceGeometry> {
    let element = &document.node(surface_id).element;
    let d_attr = element.attributes.get("d")?;
    let parsed = parse_surface_d(d_attr)?;

    let path_children: Vec<&Element> = document
        .node(surface_id)
        .children
        .iter()
        .copied()
        .filter_map(|id| {
            let e = &document.node(id).element;
            (e.kind == ElementKind::Path).then_some(e)
        })
        .collect();
    let n_paths = path_children.len() as u32;

    let sweep = if let Some((start, ref sweep_patches)) = parsed.sweep {
        Some(build_sweep_grid(
            &path_children,
            n_paths,
            start,
            sweep_patches,
        )?)
    } else {
        None
    };

    let mut coons_meshes: Vec<CoonsMesh> = Vec::with_capacity(parsed.coons.len());
    for &patch in &parsed.coons {
        coons_meshes.push(build_coons_mesh(&path_children, n_paths, patch)?);
    }

    if sweep.is_none() && coons_meshes.is_empty() {
        return None;
    }
    Some(SurfaceGeometry {
        sweep,
        coons_meshes,
    })
}

/// Build the sweep-chain grid from the M/L/Q/C/Z patches.
fn build_sweep_grid(
    path_children: &[&Element],
    n_paths: u32,
    start: u32,
    patches: &[SurfacePatch],
) -> Option<SweepGrid> {
    let mut referenced: BTreeSet<u32> = BTreeSet::new();
    referenced.insert(start);
    for patch in patches {
        match *patch {
            SurfacePatch::Linear { end } => {
                referenced.insert(end);
            }
            SurfacePatch::Quadratic { ctrl, end } => {
                referenced.insert(ctrl);
                referenced.insert(end);
            }
            SurfacePatch::Cubic { ctrl1, ctrl2, end } => {
                referenced.insert(ctrl1);
                referenced.insert(ctrl2);
                referenced.insert(end);
            }
            SurfacePatch::Close => {}
        }
    }
    if referenced.iter().any(|&i| i >= n_paths) {
        return None;
    }

    let mut polylines: BTreeMap<u32, Vec<[f32; 3]>> = BTreeMap::new();
    let mut shared_len: Option<usize> = None;
    let mut shared_closed: Option<bool> = None;
    for &idx in &referenced {
        let child = path_children[idx as usize];
        let d = child.attributes.get("d")?;
        let path = parse_path_strict(d)?;
        let (mut points, closed) = flatten_polyline(&path)?;
        if points.len() < 2 {
            return None;
        }
        if let Some(prev) = shared_len {
            if prev != points.len() {
                return None;
            }
        } else {
            shared_len = Some(points.len());
        }
        if let Some(prev) = shared_closed {
            if prev != closed {
                return None;
            }
        } else {
            shared_closed = Some(closed);
        }
        if let Some(transform_str) = child.attributes.get("transform") {
            let m = parse_transform(transform_str)?;
            for p in &mut points {
                *p = m.transform_point(*p);
            }
        }
        polylines.insert(idx, points);
    }

    let cols = shared_len.unwrap() as u32;

    let mut grid: Vec<[f32; 3]> = Vec::new();
    // Seed row: the chain's start polyline.
    grid.extend_from_slice(polylines.get(&start).unwrap());
    let mut current_idx = start;
    for patch in patches {
        let effective = match *patch {
            SurfacePatch::Close => SurfacePatch::Linear { end: start },
            other => other,
        };
        match effective {
            SurfacePatch::Linear { end } => {
                let p0 = polylines.get(&current_idx).unwrap();
                let p1 = polylines.get(&end).unwrap();
                emit_patch(&mut grid, &[p0, p1]);
                current_idx = end;
            }
            SurfacePatch::Quadratic { ctrl, end } => {
                let p0 = polylines.get(&current_idx).unwrap();
                let pc = polylines.get(&ctrl).unwrap();
                let p1 = polylines.get(&end).unwrap();
                emit_patch(&mut grid, &[p0, pc, p1]);
                current_idx = end;
            }
            SurfacePatch::Cubic { ctrl1, ctrl2, end } => {
                let p0 = polylines.get(&current_idx).unwrap();
                let pc1 = polylines.get(&ctrl1).unwrap();
                let pc2 = polylines.get(&ctrl2).unwrap();
                let p1 = polylines.get(&end).unwrap();
                emit_patch(&mut grid, &[p0, pc1, pc2, p1]);
                current_idx = end;
            }
            SurfacePatch::Close => unreachable!(),
        }
    }
    let total_rows = grid.len() as u32 / cols;
    Some(SweepGrid {
        grid,
        total_rows,
        cols,
    })
}

/// Build a single Coons-patch mesh from 4 boundary cubic curves.
fn build_coons_mesh(
    path_children: &[&Element],
    n_paths: u32,
    patch: CoonsPatch,
) -> Option<CoonsMesh> {
    let idxs = [patch.top, patch.right, patch.bottom, patch.left];
    if idxs.iter().any(|&i| i >= n_paths) {
        return None;
    }
    let top = extract_boundary_cubic(path_children[patch.top as usize])?;
    let right = extract_boundary_cubic(path_children[patch.right as usize])?;
    let bottom = extract_boundary_cubic(path_children[patch.bottom as usize])?;
    let left = extract_boundary_cubic(path_children[patch.left as usize])?;

    // Per SPEC §5.4: adjacent boundary curves of a patch MUST share
    // the four corner points within tolerance.
    if !corners_match(&top[0], &left[0]) {
        return None;
    }
    if !corners_match(&top[3], &right[0]) {
        return None;
    }
    if !corners_match(&bottom[3], &right[3]) {
        return None;
    }
    if !corners_match(&bottom[0], &left[3]) {
        return None;
    }

    let s = COONS_SAMPLES;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(((s + 1) * (s + 1)) as usize);
    let scale = (s as f32).recip();
    for j in 0..=s {
        let v = j as f32 * scale;
        for i in 0..=s {
            let u = i as f32 * scale;
            positions.push(coons_eval(&top, &right, &bottom, &left, u, v));
        }
    }
    Some(CoonsMesh {
        positions,
        samples: s,
    })
}

/// Extract the 4 control points of a single-cubic boundary `<path>`
/// (`d="M x0 y0 C x1 y1 x2 y2 x3 y3"`) and apply its `transform`
/// attribute (if any) to each control point. Returns `None` for paths
/// whose `d` does not consist of exactly one `M` and one `C`.
fn extract_boundary_cubic(element: &Element) -> Option<[[f32; 3]; 4]> {
    let d = element.attributes.get("d")?;
    let mut controls = parse_single_cubic_d(d)?;
    if let Some(transform_str) = element.attributes.get("transform") {
        let m = parse_transform(transform_str)?;
        for p in &mut controls {
            *p = m.transform_point(*p);
        }
    }
    Some(controls)
}

/// Parse `d="M x0 y0 C x1 y1 x2 y2 x3 y3"` (or its lowercase/relative
/// variants — currently rejected) into 4 3D control points in the
/// path's local frame (z = 0).
fn parse_single_cubic_d(d: &str) -> Option<[[f32; 3]; 4]> {
    use svgtypes::{PathParser, PathSegment};
    let mut p0: Option<[f32; 3]> = None;
    let mut controls: Option<[[f32; 3]; 3]> = None;
    for seg in PathParser::from(d) {
        match seg.ok()? {
            PathSegment::MoveTo { abs: true, x, y } => {
                if p0.is_some() {
                    return None;
                }
                p0 = Some([finite_f32(x)?, finite_f32(y)?, 0.0]);
            }
            PathSegment::CurveTo {
                abs: true,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                if controls.is_some() || p0.is_none() {
                    return None;
                }
                controls = Some([
                    [finite_f32(x1)?, finite_f32(y1)?, 0.0],
                    [finite_f32(x2)?, finite_f32(y2)?, 0.0],
                    [finite_f32(x)?, finite_f32(y)?, 0.0],
                ]);
            }
            _ => return None,
        }
    }
    let p0 = p0?;
    let [p1, p2, p3] = controls?;
    Some([p0, p1, p2, p3])
}

fn finite_f32(v: f64) -> Option<f32> {
    let f = v as f32;
    f.is_finite().then_some(f)
}

fn corners_match(a: &[f32; 3], b: &[f32; 3]) -> bool {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt() <= CORNER_EPSILON
}

/// Evaluate the Coons formula at `(u, v) ∈ [0, 1]²`:
/// `S(u,v) = L_c(u,v) + L_d(u,v) - B(u,v)` where `L_c` is the linear
/// blend of the top and bottom boundary curves along `v`, `L_d` is the
/// linear blend of the left and right boundary curves along `u`, and
/// `B` is the bilinear blend of the four corner points.
fn coons_eval(
    top: &[[f32; 3]; 4],
    right: &[[f32; 3]; 4],
    bottom: &[[f32; 3]; 4],
    left: &[[f32; 3]; 4],
    u: f32,
    v: f32,
) -> [f32; 3] {
    let c_top = bezier3(top, u);
    let c_bot = bezier3(bottom, u);
    let c_left = bezier3(left, v);
    let c_right = bezier3(right, v);
    let p00 = top[0];
    let p10 = top[3];
    let p01 = bottom[0];
    let p11 = bottom[3];
    let s = 1.0 - u;
    let t = 1.0 - v;
    let mut out = [0.0_f32; 3];
    for k in 0..3 {
        let lc = t * c_top[k] + v * c_bot[k];
        let ld = s * c_left[k] + u * c_right[k];
        let bilinear = s * t * p00[k] + u * t * p10[k] + s * v * p01[k] + u * v * p11[k];
        out[k] = lc + ld - bilinear;
    }
    out
}

/// Evaluate a cubic Bezier through 4 control points at `t ∈ [0, 1]`
/// using direct Bernstein basis.
fn bezier3(p: &[[f32; 3]; 4], t: f32) -> [f32; 3] {
    let s = 1.0 - t;
    let b0 = s * s * s;
    let b1 = 3.0 * s * s * t;
    let b2 = 3.0 * s * t * t;
    let b3 = t * t * t;
    let mut out = [0.0_f32; 3];
    for k in 0..3 {
        out[k] = b0 * p[0][k] + b1 * p[1][k] + b2 * p[2][k] + b3 * p[3][k];
    }
    out
}

/// Tessellate a resolved [`SurfaceGeometry`] into a filled [`Mesh`]
/// tagged [`KIND_SOLID`]. The mesh is the union of the sweep-chain
/// grid (if present) and every Coons-patch mesh, each triangulated
/// as a strip of quads.
pub(crate) fn tessellate_surface(geo: &SurfaceGeometry, color: [f32; 4]) -> Mesh {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    if let Some(sweep) = &geo.sweep {
        triangulate_grid(
            &sweep.grid,
            sweep.total_rows,
            sweep.cols,
            color,
            &mut vertices,
            &mut indices,
        );
    }

    for mesh in &geo.coons_meshes {
        let dim = mesh.samples + 1;
        triangulate_grid(
            &mesh.positions,
            dim,
            dim,
            color,
            &mut vertices,
            &mut indices,
        );
    }

    Mesh::new(vertices, indices)
}

/// Append a row-major `rows × cols` grid of positions as
/// [`KIND_SOLID`] vertices plus two-triangles-per-cell indices to
/// `vertices`/`indices`. The new triangles index into vertices from
/// `vertices.len()` as the base, so concatenating multiple grids
/// works without renumbering.
fn triangulate_grid(
    grid: &[[f32; 3]],
    rows: u32,
    cols: u32,
    color: [f32; 4],
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    if rows < 2 || cols < 2 {
        return;
    }
    let base = vertices.len() as u32;
    for &p in grid {
        vertices.push(Vertex {
            position: p,
            color,
            local: [0.0, 0.0],
            params: [0.0; 4],
            kind: KIND_SOLID,
            paint_id: 0,
        });
    }
    for row in 0..rows - 1 {
        for col in 0..cols - 1 {
            let a = base + row * cols + col;
            let b = base + row * cols + (col + 1);
            let c = base + (row + 1) * cols + col;
            let d = base + (row + 1) * cols + (col + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

/// Append `PATCH_SAMPLES - 1` rows to `grid`, evaluating the
/// degree-(N-1) Bezier through the column-wise controls of `polylines`
/// at `t = step / (PATCH_SAMPLES - 1)` for `step` in `1..PATCH_SAMPLES`.
/// Step 0 is omitted because it duplicates the previous patch's last
/// row (or the seed row, for the first patch).
fn emit_patch(grid: &mut Vec<[f32; 3]>, polylines: &[&Vec<[f32; 3]>]) {
    let degree = polylines.len() - 1;
    if degree == 0 {
        return;
    }
    let cols = polylines[0].len();
    for step in 1..PATCH_SAMPLES {
        let t = step as f32 / (PATCH_SAMPLES - 1) as f32;
        for col in 0..cols {
            let controls: Vec<[f32; 3]> = polylines.iter().map(|p| p[col]).collect();
            grid.push(de_casteljau(&controls, t));
        }
    }
}

/// Evaluate a degree-(N-1) Bezier through `controls` at `t ∈ [0, 1]`
/// using de Casteljau's algorithm.
fn de_casteljau(controls: &[[f32; 3]], t: f32) -> [f32; 3] {
    let mut work: Vec<[f32; 3]> = controls.to_vec();
    let n = work.len();
    let s = 1.0 - t;
    for r in 1..n {
        for i in 0..n - r {
            let a = work[i];
            let b = work[i + 1];
            work[i] = [
                s * a[0] + t * b[0],
                s * a[1] + t * b[1],
                s * a[2] + t * b[2],
            ];
        }
    }
    work[0]
}

/// Flatten a Lyon [`Path`] to a polyline of 3D points (with `z = 0`)
/// and return whether the path closed itself.
///
/// Returns `None` if the path contains more than one subpath (the v0
/// surface design requires single-subpath child paths for unambiguous
/// vertex correspondence across the sweep).
fn flatten_polyline(path: &Path) -> Option<(Vec<[f32; 3]>, bool)> {
    let mut points: Vec<[f32; 3]> = Vec::new();
    let mut subpath_count = 0;
    let mut closed = false;
    for event in path.iter().flattened(FLATTENING_TOLERANCE) {
        match event {
            PathEvent::Begin { at } => {
                subpath_count += 1;
                if subpath_count > 1 {
                    return None;
                }
                points.push([at.x, at.y, 0.0]);
            }
            PathEvent::Line { to, .. } => {
                let p = [to.x, to.y, 0.0];
                if !near_duplicate(points.last(), &p) {
                    points.push(p);
                }
            }
            PathEvent::End { first, close, .. } => {
                if close {
                    let p = [first.x, first.y, 0.0];
                    if !near_duplicate(points.last(), &p) {
                        points.push(p);
                    }
                    closed = true;
                }
            }
            // `flattened(FLATTENING_TOLERANCE)` lowers curve segments to
            // straight lines before yielding events; these arms should
            // never fire.
            PathEvent::Quadratic { .. } | PathEvent::Cubic { .. } => return None,
        }
    }
    Some((points, closed))
}

fn near_duplicate(last: Option<&[f32; 3]>, p: &[f32; 3]) -> bool {
    match last {
        Some(l) => {
            let dx = l[0] - p[0];
            let dy = l[1] - p[1];
            (dx * dx + dy * dy).sqrt() <= DEDUP_EPSILON
        }
        None => false,
    }
}

/// Parsed surface `'d'` attribute split into its sweep chain (if any)
/// and its Coons-patch list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ParsedSurfaceD {
    /// `(start, patches)` for the sweep chain, if the `d` contained at
    /// least one `M` followed by `L`/`Q`/`C`/`Z` commands.
    sweep: Option<(u32, Vec<SurfacePatch>)>,
    /// One `CoonsPatch` per `P` command. Each is independent of the
    /// sweep chain.
    coons: Vec<CoonsPatch>,
}

/// Parse a surface's `'d'` attribute into its sweep chain and Coons
/// patches. Returns `None` if any token is malformed, any command is
/// unsupported, there is more than one `M` (v0 permits a single sweep
/// chain), the `M` is not followed by at least one sweep command, or
/// no patches are produced at all.
fn parse_surface_d(input: &str) -> Option<ParsedSurfaceD> {
    let mut cursor = DCursor::new(input.as_bytes());
    let mut start: Option<u32> = None;
    let mut patches: Vec<SurfacePatch> = Vec::new();
    let mut coons: Vec<CoonsPatch> = Vec::new();
    let mut seen_close = false;
    loop {
        cursor.skip_separators();
        if cursor.at_end() {
            break;
        }
        if seen_close {
            // Anything after `Z` is a malformed chain.
            return None;
        }
        let cmd = cursor.bump()?;
        match cmd {
            b'M' => {
                if start.is_some() {
                    return None;
                }
                start = Some(cursor.read_index()?);
            }
            b'L' => {
                // L before M is in error — bail out via `?`.
                start?;
                let end = cursor.read_index()?;
                patches.push(SurfacePatch::Linear { end });
            }
            b'Q' => {
                start?;
                let ctrl = cursor.read_index()?;
                let end = cursor.read_index()?;
                patches.push(SurfacePatch::Quadratic { ctrl, end });
            }
            b'C' => {
                start?;
                let ctrl1 = cursor.read_index()?;
                let ctrl2 = cursor.read_index()?;
                let end = cursor.read_index()?;
                patches.push(SurfacePatch::Cubic { ctrl1, ctrl2, end });
            }
            b'Z' | b'z' => {
                start?;
                patches.push(SurfacePatch::Close);
                seen_close = true;
            }
            b'P' => {
                let top = cursor.read_index()?;
                let right = cursor.read_index()?;
                let bottom = cursor.read_index()?;
                let left = cursor.read_index()?;
                coons.push(CoonsPatch {
                    top,
                    right,
                    bottom,
                    left,
                });
            }
            _ => return None,
        }
    }

    // Validate sweep chain integrity. `M` without follow-up commands
    // (e.g., `d="M 0"`) is an error.
    let sweep = match (start, patches.is_empty()) {
        (Some(start), false) => Some((start, patches)),
        (Some(_), true) => return None,
        (None, _) => None,
    };

    // Require at least one patch overall (sweep chain produces ≥1
    // patch; standalone Coons is fine).
    if sweep.is_none() && coons.is_empty() {
        return None;
    }
    Some(ParsedSurfaceD { sweep, coons })
}

struct DCursor<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> DCursor<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0 }
    }
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }
    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }
    fn skip_separators(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() || b == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }
    fn read_index(&mut self) -> Option<u32> {
        self.skip_separators();
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return None;
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    fn build_doc(surface_attrs: &[(&str, &str)], paths: &[&[(&str, &str)]]) -> (Document, NodeId) {
        let mut doc = Document::new();
        let surface = doc.append_child(doc.root(), ElementKind::Surface);
        for (k, v) in surface_attrs {
            doc.node_mut(surface)
                .element
                .attributes
                .insert((*k).into(), (*v).into());
        }
        for attrs in paths {
            let path = doc.append_child(surface, ElementKind::Path);
            for (k, v) in *attrs {
                doc.node_mut(path)
                    .element
                    .attributes
                    .insert((*k).into(), (*v).into());
            }
        }
        (doc, surface)
    }

    #[test]
    fn parse_d_rejects_unsupported_command() {
        // Arc not supported in v0.
        assert!(parse_surface_d("M 0 A 1 2 3 4 5 6 7").is_none());
        // Smooth-cubic shorthand deferred to a future milestone.
        assert!(parse_surface_d("M 0 S 1 2").is_none());
    }

    #[test]
    fn parse_d_rejects_bare_move() {
        // `M 0` alone produces no patches → element in error.
        assert!(parse_surface_d("M 0").is_none());
    }

    #[test]
    fn parse_d_rejects_command_before_move() {
        assert!(parse_surface_d("L 0").is_none());
        assert!(parse_surface_d("Z").is_none());
    }

    #[test]
    fn parse_d_accepts_chained_cubics() {
        let parsed = parse_surface_d("M 0 C 1 2 3 C 4 5 6").unwrap();
        let (start, patches) = parsed.sweep.unwrap();
        assert_eq!(start, 0);
        assert!(parsed.coons.is_empty());
        assert_eq!(
            patches,
            vec![
                SurfacePatch::Cubic {
                    ctrl1: 1,
                    ctrl2: 2,
                    end: 3
                },
                SurfacePatch::Cubic {
                    ctrl1: 4,
                    ctrl2: 5,
                    end: 6
                },
            ]
        );
    }

    #[test]
    fn parse_d_accepts_close_only_at_end() {
        let (_, patches) = parse_surface_d("M 0 L 1 L 2 Z").unwrap().sweep.unwrap();
        assert_eq!(patches.last(), Some(&SurfacePatch::Close));
        // Anything after `Z` is malformed.
        assert!(parse_surface_d("M 0 L 1 Z L 2").is_none());
    }

    #[test]
    fn resolve_rejects_out_of_range_index() {
        let (doc, id) = build_doc(
            &[("d", "M 0 L 5")],
            &[&[("d", "M 0 0 L 10 0")], &[("d", "M 0 0 L 10 0")]],
        );
        assert!(resolve_surface(&doc, id, vp()).is_none());
    }

    #[test]
    fn resolve_rejects_mismatched_polyline_lengths() {
        let (doc, id) = build_doc(
            &[("d", "M 0 L 1")],
            &[&[("d", "M 0 0 L 10 0")], &[("d", "M 0 0 L 5 0 L 10 0")]],
        );
        assert!(resolve_surface(&doc, id, vp()).is_none());
    }

    #[test]
    fn resolve_ignores_unreferenced_children() {
        // Third path is unreferenced; even though it has a different
        // length, resolution should succeed.
        let (doc, id) = build_doc(
            &[("d", "M 0 L 1")],
            &[
                &[("d", "M 0 0 L 10 0")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translateZ(10)")],
                &[("d", "M 0 0 L 5 0 L 10 0")],
            ],
        );
        assert!(resolve_surface(&doc, id, vp()).is_some());
    }

    #[test]
    fn resolve_rejects_mixed_open_closed() {
        let (doc, id) = build_doc(
            &[("d", "M 0 L 1")],
            &[
                &[("d", "M 0 0 L 10 0")],   // open
                &[("d", "M 0 0 L 10 0 Z")], // closed (Lyon adds a closing vertex)
            ],
        );
        assert!(resolve_surface(&doc, id, vp()).is_none());
    }

    #[test]
    fn resolve_rejects_malformed_child_d() {
        // SPEC §5.4 — a referenced child whose `d` does not parse
        // cleanly invalidates the whole surface. The standalone
        // `<path>` parser would accept the valid prefix and stop at
        // "BAD"; `<surface>` must reject the parent outright.
        let (doc, id) = build_doc(
            &[("d", "M 0 L 1")],
            &[&[("d", "M 0 0 L 10 0")], &[("d", "M 0 0 L 10 0 BAD")]],
        );
        assert!(resolve_surface(&doc, id, vp()).is_none());
    }

    #[test]
    fn tessellate_linear_patch_row_count() {
        // d="M 0 L 1" → 1 patch → total_rows = 1 * (32-1) + 1 = 32.
        let (doc, id) = build_doc(
            &[("d", "M 0 L 1")],
            &[
                &[("d", "M 0 0 L 10 0 L 10 10 L 0 10")],
                &[
                    ("d", "M 0 0 L 10 0 L 10 10 L 0 10"),
                    ("transform", "translateZ(10)"),
                ],
            ],
        );
        let geo = resolve_surface(&doc, id, vp()).unwrap();
        let sweep = geo.sweep.as_ref().unwrap();
        assert_eq!(sweep.total_rows, 32);
        // Each path flattens to 4 points (M + 3 L, open).
        assert_eq!(sweep.cols, 4);
        let mesh = tessellate_surface(&geo, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 32 * 4);
        // 2 triangles per (row, col) cell, (32-1) × (4-1) cells.
        assert_eq!(mesh.indices.len(), 31 * 3 * 6);
    }

    #[test]
    fn tessellate_chained_cubics_share_row() {
        // d="M 0 C 1 2 3 C 4 5 6" → 2 cubic patches sharing path 3
        // → total_rows = 2 * (32 - 1) + 1 = 63.
        let (doc, id) = build_doc(
            &[("d", "M 0 C 1 2 3 C 4 5 6")],
            &[
                &[("d", "M 0 0 L 10 0")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translate3d(0,0,10)")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translate3d(0,0,20)")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translate3d(0,0,30)")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translate3d(0,0,40)")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translate3d(0,0,50)")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translate3d(0,0,60)")],
            ],
        );
        let geo = resolve_surface(&doc, id, vp()).unwrap();
        assert_eq!(geo.sweep.as_ref().unwrap().total_rows, 63);
    }

    #[test]
    fn tessellate_close_wraps_rows() {
        // d="M 0 L 1 L 2 L 3 Z" → 4 patches (3 L + 1 Close-as-Linear)
        // → total_rows = 4 * (32 - 1) + 1 = 125. Last row equals first.
        let (doc, id) = build_doc(
            &[("d", "M 0 L 1 L 2 L 3 Z")],
            &[
                &[("d", "M 0 0 L 10 0")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translateZ(10)")],
                &[
                    ("d", "M 0 0 L 10 0"),
                    ("transform", "translate3d(10, 0, 10)"),
                ],
                &[
                    ("d", "M 0 0 L 10 0"),
                    ("transform", "translate3d(10, 0, 0)"),
                ],
            ],
        );
        let geo = resolve_surface(&doc, id, vp()).unwrap();
        let sweep = geo.sweep.as_ref().unwrap();
        assert_eq!(sweep.total_rows, 4 * 31 + 1);
        // Last row equals first row (Z closes back to the M path).
        for col in 0..sweep.cols {
            let first = sweep.grid[col as usize];
            let last = sweep.grid[((sweep.total_rows - 1) * sweep.cols + col) as usize];
            for k in 0..3 {
                assert!(
                    (first[k] - last[k]).abs() < 1e-4,
                    "col {col} component {k}: first={first:?} last={last:?}"
                );
            }
        }
    }

    #[test]
    fn de_casteljau_quadratic_midpoint_is_bernstein_weighted() {
        // B_{0,2}(0.5) = 0.25, B_{1,2}(0.5) = 0.5, B_{2,2}(0.5) = 0.25.
        let p0 = [0.0, 0.0, 0.0];
        let p1 = [4.0, 8.0, 0.0];
        let p2 = [16.0, 0.0, 0.0];
        let mid = de_casteljau(&[p0, p1, p2], 0.5);
        // 0.25*0 + 0.5*4 + 0.25*16 = 6.0
        // 0.25*0 + 0.5*8 + 0.25*0  = 4.0
        assert!((mid[0] - 6.0).abs() < 1e-4);
        assert!((mid[1] - 4.0).abs() < 1e-4);
    }

    #[test]
    fn de_casteljau_endpoints_match_endpoints() {
        let p0 = [1.0, 2.0, 3.0];
        let pc = [9.0, 9.0, 9.0];
        let p1 = [4.0, 5.0, 6.0];
        let at_zero = de_casteljau(&[p0, pc, p1], 0.0);
        let at_one = de_casteljau(&[p0, pc, p1], 1.0);
        for k in 0..3 {
            assert!((at_zero[k] - p0[k]).abs() < 1e-5);
            assert!((at_one[k] - p1[k]).abs() < 1e-5);
        }
    }

    #[test]
    fn parse_d_accepts_coons_patch() {
        let parsed = parse_surface_d("P 0 1 2 3").unwrap();
        assert!(parsed.sweep.is_none());
        assert_eq!(
            parsed.coons,
            vec![CoonsPatch {
                top: 0,
                right: 1,
                bottom: 2,
                left: 3,
            }]
        );
    }

    #[test]
    fn parse_d_accepts_multiple_coons_patches() {
        let parsed = parse_surface_d("P 0 1 2 3 P 1 4 5 6").unwrap();
        assert_eq!(parsed.coons.len(), 2);
    }

    #[test]
    fn parse_d_rejects_coons_missing_args() {
        // P needs exactly four arguments.
        assert!(parse_surface_d("P 0 1 2").is_none());
    }

    #[test]
    fn parse_single_cubic_d_extracts_4_control_points() {
        let cps = parse_single_cubic_d("M 10 20 C 30 40 50 60 70 80").unwrap();
        assert_eq!(cps[0], [10.0, 20.0, 0.0]);
        assert_eq!(cps[1], [30.0, 40.0, 0.0]);
        assert_eq!(cps[2], [50.0, 60.0, 0.0]);
        assert_eq!(cps[3], [70.0, 80.0, 0.0]);
    }

    #[test]
    fn parse_single_cubic_d_rejects_non_cubic() {
        // Plain line is not a cubic boundary.
        assert!(parse_single_cubic_d("M 0 0 L 10 0").is_none());
        // Bare moveto without curveto.
        assert!(parse_single_cubic_d("M 0 0").is_none());
    }

    #[test]
    fn coons_eval_at_corners_returns_corner_points() {
        // Four straight-line boundaries forming the unit square in
        // world coordinates (z=0). Corners: (0,0), (1,0), (1,1),
        // (0,1).
        let top = [
            [0.0, 0.0, 0.0],
            [0.33, 0.0, 0.0],
            [0.67, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let right = [
            [1.0, 0.0, 0.0],
            [1.0, 0.33, 0.0],
            [1.0, 0.67, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let bottom = [
            [0.0, 1.0, 0.0],
            [0.33, 1.0, 0.0],
            [0.67, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let left = [
            [0.0, 0.0, 0.0],
            [0.0, 0.33, 0.0],
            [0.0, 0.67, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let tl = coons_eval(&top, &right, &bottom, &left, 0.0, 0.0);
        let tr = coons_eval(&top, &right, &bottom, &left, 1.0, 0.0);
        let br = coons_eval(&top, &right, &bottom, &left, 1.0, 1.0);
        let bl = coons_eval(&top, &right, &bottom, &left, 0.0, 1.0);
        let center = coons_eval(&top, &right, &bottom, &left, 0.5, 0.5);
        for k in 0..3 {
            assert!((tl[k] - [0.0, 0.0, 0.0][k]).abs() < 1e-5);
            assert!((tr[k] - [1.0, 0.0, 0.0][k]).abs() < 1e-5);
            assert!((br[k] - [1.0, 1.0, 0.0][k]).abs() < 1e-5);
            assert!((bl[k] - [0.0, 1.0, 0.0][k]).abs() < 1e-5);
        }
        // For straight-line boundaries the Coons patch is the bilinear
        // patch; centre = (0.5, 0.5, 0).
        assert!((center[0] - 0.5).abs() < 1e-5);
        assert!((center[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn resolve_coons_patch_emits_square_grid_mesh() {
        // Four straight-line boundaries forming the unit square.
        let (doc, id) = build_doc(
            &[("d", "P 0 1 2 3")],
            &[
                &[("d", "M 0 0 C 0.33 0 0.67 0 1 0")], // top
                &[("d", "M 1 0 C 1 0.33 1 0.67 1 1")], // right
                &[("d", "M 0 1 C 0.33 1 0.67 1 1 1")], // bottom
                &[("d", "M 0 0 C 0 0.33 0 0.67 0 1")], // left
            ],
        );
        let geo = resolve_surface(&doc, id, vp()).unwrap();
        assert!(geo.sweep.is_none());
        assert_eq!(geo.coons_meshes.len(), 1);
        let mesh = &geo.coons_meshes[0];
        assert_eq!(mesh.samples, COONS_SAMPLES);
        assert_eq!(mesh.positions.len() as u32, (COONS_SAMPLES + 1).pow(2));
        let rendered = tessellate_surface(&geo, [1.0; 4]);
        // Two triangles per cell, S² cells.
        assert_eq!(
            rendered.indices.len() as u32,
            2 * COONS_SAMPLES * COONS_SAMPLES * 3
        );
    }

    #[test]
    fn resolve_coons_rejects_mismatched_corners() {
        // Top edge ends at (1, 0); right edge starts at (2, 0) — corner mismatch.
        let (doc, id) = build_doc(
            &[("d", "P 0 1 2 3")],
            &[
                &[("d", "M 0 0 C 0.33 0 0.67 0 1 0")],
                &[("d", "M 2 0 C 2 0.33 2 0.67 2 1")], // wrong start
                &[("d", "M 0 1 C 0.33 1 0.67 1 1 1")],
                &[("d", "M 0 0 C 0 0.33 0 0.67 0 1")],
            ],
        );
        assert!(resolve_surface(&doc, id, vp()).is_none());
    }

    #[test]
    fn resolve_coons_applies_child_transform() {
        // A Coons patch where each boundary uses translate3d to lift
        // its corners into 3D. Verify the mesh corners are in 3D.
        let (doc, id) = build_doc(
            &[("d", "P 0 1 2 3")],
            &[
                &[
                    ("d", "M 0 0 C 0.33 0 0.67 0 1 0"),
                    ("transform", "translateZ(5)"),
                ],
                &[
                    ("d", "M 1 0 C 1 0.33 1 0.67 1 1"),
                    ("transform", "translateZ(5)"),
                ],
                &[
                    ("d", "M 0 1 C 0.33 1 0.67 1 1 1"),
                    ("transform", "translateZ(5)"),
                ],
                &[
                    ("d", "M 0 0 C 0 0.33 0 0.67 0 1"),
                    ("transform", "translateZ(5)"),
                ],
            ],
        );
        let geo = resolve_surface(&doc, id, vp()).unwrap();
        // First mesh vertex is the top-left corner = (0, 0, 5).
        let p00 = geo.coons_meshes[0].positions[0];
        assert!((p00[2] - 5.0).abs() < 1e-4);
    }

    #[test]
    fn resolve_applies_child_transform_to_polyline() {
        let (doc, id) = build_doc(
            &[("d", "M 0 L 1")],
            &[
                &[("d", "M 0 0 L 10 0")],
                &[("d", "M 0 0 L 10 0"), ("transform", "translateZ(100)")],
            ],
        );
        let geo = resolve_surface(&doc, id, vp()).unwrap();
        let sweep = geo.sweep.as_ref().unwrap();
        // Row 0 = first polyline at z=0. Row total_rows-1 = second polyline at z=100.
        let last_row_first_col = sweep.grid[((sweep.total_rows - 1) * sweep.cols) as usize];
        assert!((last_row_first_col[2] - 100.0).abs() < 1e-4);
    }
}
