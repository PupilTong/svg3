//! SVG 1.1 `<polyline>` — point-list resolution plus fill and stroke tessellation.
//!
//! `<polyline>` is a two-dimensional basic shape; per [`SPEC.md`](../../SPEC.md)
//! §3.1 it lies in the plane `z = 0`. This module turns a parsed `<polyline>`
//! [`Element`] into filled and stroked triangle geometry in SVG user space,
//! applying the SVG 1.1 geometry rules ([SVG11] §9.6 and §9.7). SVG fills
//! open subpaths as if they were closed, while strokes keep the path open.

use lyon_tessellation::path::Path;
use svg3_dom::Element;

use super::stroke::{self, StrokeStyle};
use super::triangulate::triangulate;
use super::vertex;
use crate::Mesh;

// A small user-unit tolerance used both for point equality and the
// near-zero outline-area check that rejects a degenerate point list.
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

impl PolylineGeometry {
    pub(crate) fn points(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.points.iter().map(|point| (point.x, point.y))
    }
}

/// Resolve a `<polyline>`'s raw `points` attribute into a [`PolylineGeometry`].
///
/// SVG 1.1 `points` coordinates are plain numbers, not lengths; units and
/// percentages are rejected. Missing, malformed, or degenerate point lists
/// are skipped. A trailing unpaired coordinate is dropped, matching SVG
/// point-list error handling. Two distinct points are enough for stroke
/// geometry; fill tessellation separately rejects lists with no closed area.
pub(crate) fn resolve_polyline(element: &Element) -> Option<PolylineGeometry> {
    let points = element.attributes.get("points")?;
    let points = parse_points(points)?;
    let points = normalize_points(points);
    (points.len() >= 2).then_some(PolylineGeometry { points })
}

/// Tessellate a resolved polyline into a filled triangle [`Mesh`].
///
/// SVG fills open subpaths as if they were closed, so the polyline's first and
/// last points are connected for triangulation. The current tessellator handles
/// simple polygons; self-intersecting point lists may produce incorrect
/// geometry until the fill-rule pipeline exists.
pub(crate) fn tessellate_polyline(geo: &PolylineGeometry, color: [f32; 4]) -> Mesh {
    if geo.points.len() < 3 || signed_area(&geo.points).abs() <= EPSILON {
        return Mesh::default();
    }

    let points: Vec<(f32, f32)> = geo.points.iter().map(|p| (p.x, p.y)).collect();
    let vertices = points.iter().map(|&(x, y)| vertex(x, y, color)).collect();
    let indices = triangulate(&points)
        .into_iter()
        .flat_map(|tri| tri.map(|index| index as u32))
        .collect();
    Mesh::new(vertices, indices)
}

/// Tessellate a resolved polyline's open stroke into triangle geometry.
pub(crate) fn tessellate_polyline_stroke(
    geo: &PolylineGeometry,
    style: &StrokeStyle,
    color: [f32; 4],
) -> Mesh {
    stroke::tessellate_stroke_path(&to_path(geo), style, color)
}

/// Convert the polyline to an open Lyon path for stroke and marker logic.
pub(crate) fn to_path(geo: &PolylineGeometry) -> Path {
    let points: Vec<(f32, f32)> = geo.points.iter().map(|point| (point.x, point.y)).collect();
    stroke::path_from_points(&points, false).expect("resolved polyline has points")
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

    if coordinates.len() < 4 {
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

fn signed_area(points: &[Point]) -> f32 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f32>()
        * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Viewport;
    use svg3_dom::ElementKind;

    fn polyline(points: &str) -> Element {
        let mut element = Element::new(ElementKind::Polyline);
        element
            .attributes
            .insert("points".to_owned(), points.to_owned());
        element
    }

    fn style(attrs: &[(&str, &str)]) -> StrokeStyle {
        let mut element = Element::new(ElementKind::Polyline);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        stroke::resolve_stroke_style(
            &element,
            Viewport {
                width: 100.0,
                height: 100.0,
            },
        )
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
        assert!(resolve_polyline(&polyline("10,10 20,20")).is_some());
        assert!(resolve_polyline(&polyline("10,10 20,20 30,30")).is_some());
        assert_eq!(resolve_polyline(&polyline("0,0 50%,50 100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50px,50 100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50,50+100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50,,50 100,0")), None);
        assert_eq!(resolve_polyline(&polyline("0,0 50,50 100,0,")), None);
    }

    #[test]
    fn resolve_polyline_ignores_trailing_unpaired_coordinate() {
        let geo = resolve_polyline(&polyline("10,10 20,20 30")).unwrap();
        assert_eq!(
            geo.points,
            vec![Point { x: 10.0, y: 10.0 }, Point { x: 20.0, y: 20.0 }]
        );
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
        assert!((mesh_area(&mesh) - signed_area(&geo.points).abs()).abs() < 1e-3);
    }

    #[test]
    fn tessellate_polyline_handles_clockwise_fill() {
        let geo = resolve_polyline(&polyline("10,20 45,70 80,20")).unwrap();
        let mesh = tessellate_polyline(&geo, [1.0; 4]);
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
    }

    #[test]
    fn tessellate_polyline_fill_skips_open_line_area() {
        let geo = resolve_polyline(&polyline("10,10 20,20")).unwrap();
        assert!(tessellate_polyline(&geo, [1.0; 4]).is_empty());

        let geo = resolve_polyline(&polyline("10,10 20,20 30,30")).unwrap();
        assert!(tessellate_polyline(&geo, [1.0; 4]).is_empty());
    }

    #[test]
    fn tessellate_polyline_stroke_keeps_path_open() {
        let geo = resolve_polyline(&polyline("10,10 80,10 80,60")).unwrap();
        let mesh = tessellate_polyline_stroke(
            &geo,
            &style(&[
                ("stroke-width", "6"),
                ("stroke-linejoin", "round"),
                ("stroke-dasharray", "12 4"),
            ]),
            [0.0, 0.0, 1.0, 1.0],
        );

        assert!(!mesh.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
    }

    fn mesh_area(mesh: &Mesh) -> f32 {
        mesh.indices
            .chunks_exact(3)
            .map(|triangle| {
                let a = mesh.vertices[triangle[0] as usize].position;
                let b = mesh.vertices[triangle[1] as usize].position;
                let c = mesh.vertices[triangle[2] as usize].position;
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5
            })
            .sum()
    }
}
