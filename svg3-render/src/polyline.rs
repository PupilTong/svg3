//! SVG 1.1 `<polyline>` — point-list resolution and fill tessellation.
//!
//! `<polyline>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<polyline>`
//! [`Element`] into filled triangle geometry in SVG user space, applying the
//! SVG 1.1 geometry rules ([SVG11] §9.6 and §9.7). Stroke rendering is not
//! implemented yet, so only the SVG fill behavior is represented: the open
//! polyline is closed by the fill operation.

use svg3_dom::Element;

use crate::shape::vertex;
use crate::Mesh;

// A small user-unit tolerance used both for point equality and near-zero
// triangle/polygon area checks in the current scaffold renderer.
const EPSILON: f32 = 1e-5;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

/// A `<polyline>`'s geometry after resolving its `points` list. All values
/// are in SVG user units.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PolylineGeometry {
    points: Vec<Point>,
}

/// Resolve a `<polyline>`'s raw `points` attribute into a [`PolylineGeometry`].
///
/// SVG 1.1 `points` coordinates are plain numbers, not lengths; units and
/// percentages are rejected. Missing, malformed, odd, or degenerate point
/// lists are skipped. Because this renderer does not support stroke yet, fewer
/// than three distinct points have no filled area and contribute no geometry.
pub(crate) fn resolve_polyline(element: &Element) -> Option<PolylineGeometry> {
    let points = element.attributes.get("points")?;
    let points = parse_points(points)?;
    let points = normalize_points(points);
    (points.len() >= 3 && signed_area(&points).abs() > EPSILON)
        .then_some(PolylineGeometry { points })
}

/// Tessellate a resolved polyline into a filled triangle [`Mesh`].
///
/// SVG fills open subpaths as if they were closed, so the polyline's first and
/// last points are connected for triangulation. The current tessellator handles
/// simple polygons; self-intersecting point lists may produce incorrect
/// geometry until the fill-rule pipeline exists.
pub(crate) fn tessellate_polyline(geo: &PolylineGeometry, color: [f32; 4]) -> Mesh {
    let indices = triangulate(&geo.points);
    if indices.is_empty() {
        return Mesh::default();
    }

    Mesh {
        vertices: geo
            .points
            .iter()
            .map(|point| vertex(point.x, point.y, color))
            .collect(),
        indices,
    }
}

fn parse_points(value: &str) -> Option<Vec<Point>> {
    let mut coordinates = Vec::new();
    let mut cursor = skip_wsp(value.as_bytes(), 0);
    let bytes = value.as_bytes();

    loop {
        if cursor == bytes.len() {
            break;
        }

        let (token, next) = next_coordinate_token(value, cursor)?;
        coordinates.push(parse_coordinate(token)?);
        cursor = next;

        cursor = consume_coordinate_separator(bytes, cursor)?;
    }

    if coordinates.len() < 6 || coordinates.len() % 2 != 0 {
        return None;
    }

    Some(
        coordinates
            .chunks_exact(2)
            .map(|pair| Point {
                x: pair[0],
                y: pair[1],
            })
            .collect(),
    )
}

fn consume_coordinate_separator(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    let before_wsp = cursor;
    cursor = skip_wsp(bytes, cursor);
    if cursor == bytes.len() {
        return Some(cursor);
    }
    if bytes[cursor] == b',' {
        cursor = skip_wsp(bytes, cursor + 1);
        return (cursor < bytes.len()).then_some(cursor);
    }
    if cursor != before_wsp {
        return Some(cursor);
    }
    if bytes[cursor] == b'-' {
        return Some(cursor);
    }
    None
}

fn parse_coordinate(token: &str) -> Option<f32> {
    let coordinate: f32 = token.parse().ok()?;
    coordinate.is_finite().then_some(coordinate)
}

fn skip_wsp(bytes: &[u8], mut cursor: usize) -> usize {
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn next_coordinate_token(value: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = value.as_bytes();
    let mut cursor = start;

    if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }

    let mut has_digit = false;
    while matches!(bytes.get(cursor), Some(byte) if byte.is_ascii_digit()) {
        cursor += 1;
        has_digit = true;
    }

    if matches!(bytes.get(cursor), Some(b'.')) {
        cursor += 1;
        while matches!(bytes.get(cursor), Some(byte) if byte.is_ascii_digit()) {
            cursor += 1;
            has_digit = true;
        }
    }

    if !has_digit {
        return None;
    }

    if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        let exponent_start = cursor;
        while matches!(bytes.get(cursor), Some(byte) if byte.is_ascii_digit()) {
            cursor += 1;
        }
        if cursor == exponent_start {
            return None;
        }
    }

    Some((&value[start..cursor], cursor))
}

fn normalize_points(points: Vec<Point>) -> Vec<Point> {
    let mut normalized = Vec::with_capacity(points.len());
    for point in points {
        if normalized
            .last()
            .is_none_or(|last| !same_point(*last, point))
        {
            normalized.push(point);
        }
    }

    if normalized.len() > 1
        && same_point(
            normalized[0],
            *normalized.last().expect("checked non-empty point list"),
        )
    {
        normalized.pop();
    }

    normalized
}

fn same_point(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() <= EPSILON && (a.y - b.y).abs() <= EPSILON
}

fn triangulate(points: &[Point]) -> Vec<u32> {
    let area = signed_area(points);
    if points.len() < 3 || area.abs() <= EPSILON {
        return Vec::new();
    }

    let winding = area.signum();
    let mut remaining: Vec<usize> = (0..points.len()).collect();
    let mut indices = Vec::with_capacity((points.len() - 2) * 3);

    while remaining.len() > 3 {
        let Some(ear) = find_ear(points, &remaining, winding) else {
            return Vec::new();
        };
        let count = remaining.len();
        let prev = remaining[(ear + count - 1) % count];
        let curr = remaining[ear];
        let next = remaining[(ear + 1) % count];
        indices.extend_from_slice(&[prev as u32, curr as u32, next as u32]);
        remaining.remove(ear);
    }

    if remaining.len() == 3 {
        let [a, b, c] = [remaining[0], remaining[1], remaining[2]];
        if winding * cross(points[a], points[b], points[c]) > EPSILON {
            indices.extend_from_slice(&[a as u32, b as u32, c as u32]);
        }
    }

    indices
}

fn find_ear(points: &[Point], remaining: &[usize], winding: f32) -> Option<usize> {
    let count = remaining.len();
    (0..count).find(|&i| {
        let prev = remaining[(i + count - 1) % count];
        let curr = remaining[i];
        let next = remaining[(i + 1) % count];
        is_convex(points[prev], points[curr], points[next], winding)
            && !remaining.iter().copied().any(|candidate| {
                candidate != prev
                    && candidate != curr
                    && candidate != next
                    && point_in_triangle(
                        points[candidate],
                        points[prev],
                        points[curr],
                        points[next],
                        winding,
                    )
            })
    })
}

fn is_convex(a: Point, b: Point, c: Point, winding: f32) -> bool {
    winding * cross(a, b, c) > EPSILON
}

fn point_in_triangle(point: Point, a: Point, b: Point, c: Point, winding: f32) -> bool {
    winding * cross(a, b, point) >= -EPSILON
        && winding * cross(b, c, point) >= -EPSILON
        && winding * cross(c, a, point) >= -EPSILON
}

fn signed_area(points: &[Point]) -> f32 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f32>()
        * 0.5
}

fn cross(a: Point, b: Point, c: Point) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    fn polyline(points: &str) -> Element {
        let mut element = Element::new(ElementKind::Polyline);
        element
            .attributes
            .insert("points".to_owned(), points.to_owned());
        element
    }

    #[test]
    fn resolve_polyline_reads_points_list() {
        let geo = resolve_polyline(&polyline("10,20 80,20 45,70")).unwrap();
        assert_eq!(
            geo.points,
            vec![
                Point { x: 10.0, y: 20.0 },
                Point { x: 80.0, y: 20.0 },
                Point { x: 45.0, y: 70.0 },
            ]
        );
    }

    #[test]
    fn resolve_polyline_accepts_negative_separator_and_exponents() {
        let geo = resolve_polyline(&polyline("0,0 1e2-25 .2,5.")).unwrap();
        assert_eq!(
            geo.points,
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 100.0, y: -25.0 },
                Point { x: 0.2, y: 5.0 },
            ]
        );
    }

    #[test]
    fn resolve_polyline_skips_degenerate_or_malformed_points() {
        assert_eq!(resolve_polyline(&Element::new(ElementKind::Polyline)), None);
        assert_eq!(resolve_polyline(&polyline("10,10 20,20")), None);
        assert_eq!(resolve_polyline(&polyline("10,10 20,20 30")), None);
        assert_eq!(resolve_polyline(&polyline("10,10 20,20 30,30")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50%,50 100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50px,50 100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50,50+100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50,,50 100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50,50 100,0,")), None);
    }

    #[test]
    fn resolve_polyline_removes_explicit_closing_point() {
        let geo = resolve_polyline(&polyline("10,20 80,20 45,70 10,20")).unwrap();
        assert_eq!(geo.points.len(), 3);
        assert_eq!(geo.points[0], Point { x: 10.0, y: 20.0 });
        assert_eq!(geo.points[2], Point { x: 45.0, y: 70.0 });
    }

    #[test]
    fn tessellate_polyline_closes_triangle_for_fill() {
        let geo = resolve_polyline(&polyline("10,20 80,20 45,70")).unwrap();
        let mesh = tessellate_polyline(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
        assert_eq!(mesh.vertices[0].position, [10.0, 20.0, 0.0]);
        assert_eq!(mesh.vertices[0].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn tessellate_polyline_handles_concave_fill() {
        let geo = resolve_polyline(&polyline("10,10 80,10 80,40 50,40 50,80 10,80")).unwrap();
        let mesh = tessellate_polyline(&geo, [1.0; 4]);
        assert_eq!(mesh.vertices.len(), 6);
        assert_eq!(mesh.indices.len(), 12);
    }

    #[test]
    fn tessellate_polyline_handles_clockwise_fill() {
        let geo = resolve_polyline(&polyline("10,20 45,70 80,20")).unwrap();
        let mesh = tessellate_polyline(&geo, [1.0; 4]);
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
    }
}
