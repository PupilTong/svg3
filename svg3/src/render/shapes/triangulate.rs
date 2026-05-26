//! Ear-clipping polygon triangulation, shared by the polygonal fill shapes.
//!
//! `<polygon>` and `<polyline>` both fill an outline that may be concave, so
//! neither can reuse the centre-pivoted triangle fan that `<rect>` and
//! `<circle>` use — a fan is correct only for a convex outline. Each instead
//! ear-clips its outline into fill triangles, and that one algorithm lives
//! here so the two shapes share a single tested implementation rather than
//! each carrying a near-identical copy.
//!
//! [`triangulate`] takes an outline as `(x, y)` corners in SVG user space
//! and returns its fill triangles as index triples into that corner list.
//! The closing edge from the last corner back to the first is implicit, so
//! a `<polyline>`'s open outline is triangulated as if it were closed.
//!
//! Two deliberate robustness choices — the convention settled on when the
//! `<polygon>` and `<polyline>` tessellators were unified onto this module —
//! free a caller from special-casing its input:
//!
//! * **A collinear corner is a valid ear.** A corner is rejected only when
//!   it is strictly *reflex*, turning against the outline's winding; a
//!   straight corner is clipped like any other ear, which merely drops a
//!   redundant vertex. Demanding strict convexity instead could strand an
//!   outline whose only remaining ears are collinear.
//! * **Un-clippable input falls back to a fan.** A self-intersecting or
//!   otherwise degenerate outline has no ear-clipping triangulation; rather
//!   than loop forever or emit nothing, [`triangulate`] fans over whatever
//!   corners remain, so every outline still yields some geometry.

/// Triangulate a simple outline by ear clipping, returning each fill
/// triangle as an index triple into `points`.
///
/// A simple (non-self-intersecting) outline of `n` corners yields exactly
/// `n - 2` triangles, whatever its winding and whether it is convex or
/// concave. Fewer than three corners enclose no area and yield nothing; a
/// degenerate outline falls back to a fan over its remaining corners (see
/// the module documentation).
pub(crate) fn triangulate(points: &[(f32, f32)]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }

    // The signed area's sign is the outline's winding; `is_ear` compares a
    // candidate corner's turn against it to tell convex from reflex.
    let winding = signed_area(points);
    let mut triangles = Vec::with_capacity(n - 2);
    // The not-yet-clipped corners, as a ring of indices into `points`.
    let mut ring: Vec<usize> = (0..n).collect();

    while ring.len() > 3 {
        let m = ring.len();
        let ear = (0..m).find(|&i| {
            is_ear(
                points,
                winding,
                ring[(i + m - 1) % m],
                ring[i],
                ring[(i + 1) % m],
                &ring,
            )
        });
        let Some(i) = ear else {
            // No ear: a self-intersecting or degenerate outline. Stop
            // clipping and fan whatever remains so the shape still draws.
            break;
        };
        triangles.push([ring[(i + m - 1) % m], ring[i], ring[(i + 1) % m]]);
        ring.remove(i);
    }
    // The final ear, or — after a no-ear break — a fan over the remainder.
    for pair in ring[1..].windows(2) {
        triangles.push([ring[0], pair[0], pair[1]]);
    }
    triangles
}

/// Whether `tip` is an ear: a convex corner whose triangle `(prev, tip,
/// next)` encloses no other corner still in `ring`.
fn is_ear(
    points: &[(f32, f32)],
    winding: f32,
    prev: usize,
    tip: usize,
    next: usize,
    ring: &[usize],
) -> bool {
    let (a, b, c) = (points[prev], points[tip], points[next]);
    // A reflex corner turns against the outline winding — never an ear. A
    // straight (collinear) corner is allowed: clipping it merely drops a
    // redundant vertex.
    if cross(a, b, c) * winding < 0.0 {
        return false;
    }
    // The ear triangle must enclose no other corner.
    !ring.iter().any(|&idx| {
        idx != prev && idx != tip && idx != next && point_in_triangle(points[idx], a, b, c)
    })
}

/// Twice the signed area of triangle `(a, b, c)`: positive, negative or
/// zero as the corner `a→b→c` turns one way, the other, or runs straight.
fn cross(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// An outline's signed area (shoelace formula). The magnitude is the area;
/// the sign is the winding — opposite for the two orientations.
fn signed_area(points: &[(f32, f32)]) -> f32 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (x0, y0) = points[i];
            let (x1, y1) = points[(i + 1) % n];
            x0 * y1 - x1 * y0
        })
        .sum::<f32>()
        / 2.0
}

/// Whether `p` lies inside triangle `(a, b, c)`, edges included.
fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    // Inside iff `p` is on the same side of every edge. A point exactly on
    // an edge (a zero) counts as inside, so an ear grazed by another corner
    // is rejected rather than clipped over it.
    let any_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let any_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(any_neg && any_pos)
}
