//! SVG 1.1 `<polygon>` — geometry resolution plus fill and stroke tessellation.
//!
//! `<polygon>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<polygon>`
//! [`Element`] into filled and stroked triangle [`Mesh`]es in SVG user space, applying
//! the SVG 1.1 geometry rules ([SVG11] §9.7). The behavioural reference is
//! the SVG WPT suite (`svg/shapes/polygon-0*.svg`).
//!
//! Unlike `<rect>` and `<circle>`, a polygon can be concave, so its fill is
//! tessellated by ear clipping rather than a centre-pivoted triangle fan.
//! `fill` and stroke resolution and the mesh [`Vertex`] constructor are
//! shared with the other basic shapes — see [`crate::render::shapes`]. `transform`
//! and grouping are not handled yet — see the crate roadmap.

use crate::dom::Element;
use lyon_tessellation::path::Path;

use super::stroke::{self, StrokeStyle};
use super::triangulate::triangulate;
use super::vertex;
use crate::render::Mesh;

/// A `<polygon>`'s geometry: its corner points in SVG user units, in
/// document order. A resolved geometry always holds at least three points.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PolygonGeometry {
    /// Corner points `(x, y)` in document order; the closing edge from the
    /// last point back to the first is implicit ([SVG11] §9.7).
    pub points: Vec<(f32, f32)>,
}

/// Resolve a `<polygon>`'s raw attributes into a [`PolygonGeometry`].
///
/// The `points` attribute is a list of `(x, y)` coordinate pairs in the
/// user coordinate system ([SVG11] §9.7); unlike `<length>` attributes it
/// carries no units or percentages, so no viewport is consulted.
///
/// Returns `None` when the polygon is not rendered: a missing `points`
/// attribute, or fewer than three points — two or fewer cannot enclose a
/// fillable area.
pub(crate) fn resolve_polygon(element: &Element) -> Option<PolygonGeometry> {
    let points = parse_points(element.attributes.get("points")?);
    (points.len() >= 3).then_some(PolygonGeometry { points })
}

/// Parse an SVG `points` list into `(x, y)` pairs.
///
/// Numbers are separated by whitespace and/or commas. The SVG number
/// grammar also lets a number end without a separator before the next, so a
/// sign or a second decimal point starts a fresh number (`1.5.5` is
/// `1.5, .5` and `10-5` is `10, -5`). Scanning stops at the first token
/// that is not a finite number, and a trailing unpaired coordinate is
/// dropped — both matching [SVG11] §9.7, where an in-error `points` list is
/// processed up to the first error, as for `<path>` data.
fn parse_points(value: &str) -> Vec<(f32, f32)> {
    let bytes = value.as_bytes();
    let mut coords: Vec<f32> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Skip the comma-whitespace separators between numbers.
        if bytes[i] == b',' || bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if bytes[i] == b'+' || bytes[i] == b'-' {
            i += 1;
        }
        let mut seen_digit = false;
        let mut seen_dot = false;
        while i < bytes.len() {
            match bytes[i] {
                b'0'..=b'9' => seen_digit = true,
                b'.' if !seen_dot => seen_dot = true,
                _ => break,
            }
            i += 1;
        }
        // An exponent only extends the token when a mantissa preceded it
        // and real exponent digits follow.
        if seen_digit && i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            let mut j = i + 1;
            if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                j += 1;
            }
            let exp_digits = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > exp_digits {
                i = j;
            }
        }
        // No digits => this token is not a number; stop the list here.
        if !seen_digit {
            break;
        }
        match value[start..i].parse::<f32>() {
            Ok(number) if number.is_finite() => coords.push(number),
            _ => break,
        }
    }
    coords
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

/// Tessellate a resolved polygon into a filled triangle [`Mesh`].
///
/// Every point becomes one vertex; [`triangulate`] ear-clips the outline
/// into its fill triangles. Positions are in SVG user space with `z = 0`.
pub(crate) fn tessellate_polygon(geo: &PolygonGeometry, color: [f32; 4]) -> Mesh {
    let vertices = geo
        .points
        .iter()
        .map(|&(x, y)| vertex(x, y, color))
        .collect();
    let indices = triangulate(&geo.points)
        .into_iter()
        .flat_map(|tri| tri.map(|index| index as u32))
        .collect();
    Mesh::new(vertices, indices)
}

/// Tessellate a resolved polygon's stroke into triangle geometry.
pub(crate) fn tessellate_polygon_stroke(
    geo: &PolygonGeometry,
    style: &StrokeStyle,
    color: [f32; 4],
) -> Mesh {
    stroke::tessellate_stroke_path(&to_path(geo), style, color)
}

/// Convert the polygon outline to a closed Lyon path for stroke and marker
/// logic.
pub(crate) fn to_path(geo: &PolygonGeometry) -> Path {
    stroke::path_from_points(&geo.points, true).expect("resolved polygon has points")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::ElementKind;

    /// Build a `<polygon>` element carrying the given raw attributes.
    fn polygon(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Polygon);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    /// Total unsigned area of a triangle index list over a vertex mesh.
    fn tessellated_area(mesh: &Mesh) -> f32 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let p = |i: u32| mesh.vertices[i as usize].position;
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs()
            })
            .sum()
    }

    #[test]
    fn parse_points_accepts_comma_and_whitespace_separators() {
        let expected = vec![(1.0, 2.0), (3.0, 4.0), (5.0, 6.0)];
        assert_eq!(parse_points("1,2 3,4 5,6"), expected);
        assert_eq!(parse_points("1 2 3 4 5 6"), expected);
        assert_eq!(parse_points("1,2,3,4,5,6"), expected);
        assert_eq!(parse_points("  1, 2 \n 3 ,4\t5,6 "), expected);
    }

    #[test]
    fn parse_points_splits_glued_numbers() {
        // The SVG number grammar lets the next number start without a
        // separator when it begins with a sign...
        assert_eq!(
            parse_points("0,0 10-5 3,8"),
            vec![(0.0, 0.0), (10.0, -5.0), (3.0, 8.0)]
        );
        // ...and exponent notation still pairs up.
        assert_eq!(
            parse_points("1e2,3 4,5 6,7"),
            vec![(100.0, 3.0), (4.0, 5.0), (6.0, 7.0)]
        );
    }

    #[test]
    fn parse_points_stops_at_first_error() {
        // [SVG11] §9.7: a malformed list is processed up to the first error
        // — here the `x` — and no further.
        assert_eq!(parse_points("1,2 3,4 x 9,9"), vec![(1.0, 2.0), (3.0, 4.0)]);
        // An odd coordinate count leaves a trailing number unpaired; it is
        // dropped rather than fabricating a partial point.
        assert_eq!(parse_points("1,2 3,4 5"), vec![(1.0, 2.0), (3.0, 4.0)]);
        assert!(parse_points("").is_empty());
    }

    #[test]
    fn resolve_polygon_needs_at_least_three_points() {
        // Two or fewer points cannot enclose a fillable area.
        assert_eq!(resolve_polygon(&polygon(&[("points", "0,0 10,0")])), None);
        assert_eq!(resolve_polygon(&polygon(&[("points", "0,0")])), None);
        // Three points => a resolved geometry, in document order.
        let geo = resolve_polygon(&polygon(&[("points", "0,0 10,0 5,8")])).unwrap();
        assert_eq!(geo.points, vec![(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)]);
    }

    #[test]
    fn resolve_polygon_skips_missing_or_short_points() {
        // A missing `points`, an empty one, or one that parses to too few
        // points is not rendered.
        assert_eq!(resolve_polygon(&polygon(&[])), None);
        assert_eq!(resolve_polygon(&polygon(&[("points", "")])), None);
        assert_eq!(resolve_polygon(&polygon(&[("points", "1 2 3")])), None);
    }

    #[test]
    fn tessellate_convex_polygon_triangulates_fully() {
        // A convex quad: ear clipping yields n-2 = 2 triangles, every index
        // in range, the fill colour on every vertex, all in the z=0 plane.
        let geo = resolve_polygon(&polygon(&[("points", "0,0 10,0 10,10 0,10")])).unwrap();
        let mesh = tessellate_polygon(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 3 * (4 - 2));
        assert!(mesh
            .indices
            .iter()
            .all(|&i| (i as usize) < mesh.vertices.len()));
        for v in &mesh.vertices {
            assert_eq!(v.color, [0.0, 0.0, 1.0, 1.0]);
            assert_eq!(v.position[2], 0.0);
        }
        // The two triangles tile the unit-scaled square exactly.
        assert!((tessellated_area(&mesh) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn tessellate_concave_polygon_covers_its_area() {
        // A concave "dart": the last vertex pokes inward, so a fan from any
        // single corner would spill outside the outline. A correct
        // ear-clipped triangulation tiles the polygon exactly — its
        // triangle areas sum to the polygon's shoelace area (35).
        let geo = resolve_polygon(&polygon(&[("points", "0,0 10,5 0,10 3,5")])).unwrap();
        let mesh = tessellate_polygon(&geo, [1.0; 4]);
        assert_eq!(mesh.indices.len(), 3 * (4 - 2));
        assert!(
            (tessellated_area(&mesh) - 35.0).abs() < 1e-3,
            "triangulation area {} should match the dart's area 35",
            tessellated_area(&mesh),
        );
    }

    #[test]
    fn tessellate_degenerate_polygon_stays_robust() {
        // Inputs with no valid ear-clipping triangulation — self-intersecting,
        // fully collinear, or coincident — must still terminate and emit only
        // valid, in-range indices via the `triangulate` fan fallback, never
        // panicking or looping forever. The collinear and coincident cases
        // have no ear at all, so they exercise that fallback branch directly.
        for points in [
            "0,0 10,10 10,0 0,10", // bowtie: self-intersecting
            "0,0 10,0 20,0 30,0",  // collinear: zero area, no ear exists
            "5,5 5,5 5,5 5,5",     // every corner coincident
        ] {
            let geo = resolve_polygon(&polygon(&[("points", points)])).unwrap();
            let mesh = tessellate_polygon(&geo, [1.0; 4]);
            assert_eq!(mesh.vertices.len(), geo.points.len());
            assert!(!mesh.indices.is_empty(), "`{points}` produced no geometry");
            assert_eq!(
                mesh.indices.len() % 3,
                0,
                "`{points}` left a partial triangle",
            );
            assert!(
                mesh.indices
                    .iter()
                    .all(|&i| (i as usize) < mesh.vertices.len()),
                "`{points}` emitted an out-of-range index",
            );
        }
    }
}
