//! Shared SVG stroke geometry support.
//!
//! The renderer represents strokes as normal triangle meshes emitted by Lyon
//! and uploaded through the same GPU draw path as fills. This module resolves
//! the stroke presentation attributes common to all 2D shapes and handles
//! dashed strokes by slicing flattened paths into visible dash subpaths before
//! tessellation.

use lyon_tessellation::geometry_builder::{BuffersBuilder, VertexBuffers};
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::math::{point, Point, Vector};
use lyon_tessellation::path::{Path, PathEvent};
use lyon_tessellation::{LineCap, LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex};
use svg3_dom::Element;
use svgtypes::{LengthListParser, LengthUnit};

use super::{resolve_stroke_width, vertex, Length, Viewport};
use crate::{Mesh, Vertex};

pub(crate) const FLATTENING_TOLERANCE: f32 = 0.1;
const EPSILON: f32 = 1e-5;

/// Resolved stroke geometry attributes shared by all stroked SVG 2D shapes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StrokeStyle {
    /// Stroke width in user units.
    pub(crate) width: f32,
    /// Line cap applied to open subpath ends and dash ends.
    pub(crate) linecap: LineCap,
    /// Line join applied at vertices within a stroked subpath.
    pub(crate) linejoin: LineJoin,
    /// SVG miter limit, clamped to Lyon's valid range.
    pub(crate) miterlimit: f32,
    /// A repeated dash/gap pattern in user units after percentage resolution.
    dasharray: Option<Vec<f32>>,
    /// Dash offset in user units after percentage resolution.
    dashoffset: f32,
    /// Optional `pathLength` calibration value.
    path_length: Option<f32>,
}

impl StrokeStyle {
    /// Whether this stroke can contribute visible geometry when paint exists.
    pub(crate) fn is_visible(&self) -> bool {
        self.width > 0.0
    }

    /// Whether a single `<line>` can use the analytic SDF segment fast path.
    pub(crate) fn uses_segment_fast_path(&self) -> bool {
        self.linecap == LineCap::Butt && self.dasharray.is_none()
    }

    fn dash_pattern_for_path(&self, path: &Path) -> Option<DashPattern> {
        let dasharray = self.dasharray.as_ref()?;
        let actual_length = path_length(path);
        let scale = self.path_length_scale(actual_length);
        let pattern: Vec<f32> = dasharray.iter().map(|value| value * scale).collect();
        let offset = self.dashoffset * scale;
        DashPattern::new(pattern, offset)
    }

    fn path_length_scale(&self, actual_length: f32) -> f32 {
        self.path_length
            .filter(|path_length| *path_length > 0.0 && actual_length > 0.0)
            .map(|path_length| actual_length / path_length)
            .unwrap_or(1.0)
    }
}

/// Resolve all stroke geometry attributes that are independent of stroke
/// paint. The caller still resolves `stroke` paint separately because SVG's
/// initial stroke paint is `none`.
pub(crate) fn resolve_stroke_style(element: &Element, viewport: Viewport) -> StrokeStyle {
    StrokeStyle {
        width: resolve_stroke_width(element, viewport),
        linecap: resolve_linecap(element),
        linejoin: resolve_linejoin(element),
        miterlimit: resolve_miterlimit(element),
        dasharray: resolve_dasharray(element, viewport),
        dashoffset: resolve_dashoffset(element, viewport),
        path_length: resolve_path_length(element),
    }
}

/// Tessellate a stroked path with the resolved stroke style.
///
/// Non-dashed strokes are sent to Lyon directly. Dashed strokes first flatten
/// curves and split the path into visible dash subpaths, then Lyon tessellates
/// that derived path with the same cap/join/miter options.
pub(crate) fn tessellate_stroke_path(path: &Path, style: &StrokeStyle, color: [f32; 4]) -> Mesh {
    if !style.is_visible() {
        return Mesh::default();
    }

    if let Some(pattern) = style.dash_pattern_for_path(path) {
        let stroke_path = dashed_path(path, &pattern);
        tessellate_plain_stroke(&stroke_path, style, color)
    } else {
        tessellate_plain_stroke(path, style, color)
    }
}

/// Return the flattened length of a path in user units, including explicit
/// close segments.
pub(crate) fn path_length(path: &Path) -> f32 {
    flattened_subpaths(path)
        .into_iter()
        .map(|subpath| subpath.length())
        .sum()
}

/// A marker placement on a stroked path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MarkerPlacement {
    /// Which marker property this placement consumes.
    pub(crate) kind: MarkerKind,
    /// Marker origin x coordinate in user units.
    pub(crate) x: f32,
    /// Marker origin y coordinate in user units.
    pub(crate) y: f32,
    /// Auto-orientation angle in radians.
    pub(crate) angle: f32,
}

/// Marker property slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkerKind {
    Start,
    Mid,
    End,
}

/// Compute marker placements from a path's authored vertices.
///
/// Curves contribute markers at their endpoints, with orientation derived
/// from endpoint tangents. This intentionally does not use dash slicing:
/// markers attach to the original shape path, not to individual dashes.
pub(crate) fn marker_placements(path: &Path) -> Vec<MarkerPlacement> {
    let mut placements = Vec::new();
    for subpath in marker_subpaths(path) {
        if subpath.vertices.len() < 2 {
            continue;
        }

        let first = subpath.vertices[0];
        placements.push(MarkerPlacement {
            kind: MarkerKind::Start,
            x: first.point.x,
            y: first.point.y,
            angle: angle(first.outgoing.or(first.incoming)),
        });

        let last_index = subpath.vertices.len() - 1;
        for vertex in &subpath.vertices[1..last_index] {
            placements.push(MarkerPlacement {
                kind: MarkerKind::Mid,
                x: vertex.point.x,
                y: vertex.point.y,
                angle: marker_angle(vertex.incoming, vertex.outgoing),
            });
        }

        let last = subpath.vertices[last_index];
        placements.push(MarkerPlacement {
            kind: MarkerKind::End,
            x: last.point.x,
            y: last.point.y,
            angle: angle(last.incoming.or(last.outgoing)),
        });
    }
    placements
}

/// Build a path from a point list, optionally closing the subpath.
pub(crate) fn path_from_points(points: &[(f32, f32)], close: bool) -> Option<Path> {
    let (&(x, y), rest) = points.split_first()?;
    let mut builder = Path::builder().with_svg();
    builder.move_to(point(x, y));
    for &(x, y) in rest {
        builder.line_to(point(x, y));
    }
    if close {
        builder.close();
    }
    Some(builder.build())
}

fn tessellate_plain_stroke(path: &Path, style: &StrokeStyle, color: [f32; 4]) -> Mesh {
    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let options = StrokeOptions::default()
        .with_line_width(style.width)
        .with_line_cap(style.linecap)
        .with_line_join(style.linejoin)
        .with_miter_limit(style.miterlimit)
        .with_tolerance(FLATTENING_TOLERANCE);
    let mut tessellator = StrokeTessellator::new();
    let mut builder = BuffersBuilder::new(&mut buffers, move |v: StrokeVertex<'_, '_>| {
        let p = v.position();
        vertex(p.x, p.y, color)
    });

    if tessellator
        .tessellate_path(path, &options, &mut builder)
        .is_err()
    {
        return Mesh::default();
    }

    Mesh::new(buffers.vertices, buffers.indices)
}

#[derive(Debug, Clone)]
struct DashPattern {
    values: Vec<f32>,
    offset: f32,
    period: f32,
}

impl DashPattern {
    fn new(values: Vec<f32>, offset: f32) -> Option<Self> {
        let period: f32 = values.iter().sum();
        (period > EPSILON).then_some(Self {
            values,
            offset,
            period,
        })
    }

    fn cursor(&self) -> DashCursor<'_> {
        let mut distance = self.offset.rem_euclid(self.period);
        let mut index = 0;
        while distance > self.values[index] && self.values[index] > EPSILON {
            distance -= self.values[index];
            index = (index + 1) % self.values.len();
        }
        DashCursor {
            pattern: self,
            index,
            remaining: (self.values[index] - distance).max(0.0),
        }
    }
}

struct DashCursor<'a> {
    pattern: &'a DashPattern,
    index: usize,
    remaining: f32,
}

impl DashCursor<'_> {
    fn is_painting(&self) -> bool {
        self.index.is_multiple_of(2)
    }

    fn advance(&mut self, mut distance: f32) {
        while distance > EPSILON {
            if self.remaining <= EPSILON {
                self.next();
                continue;
            }
            let step = distance.min(self.remaining);
            self.remaining -= step;
            distance -= step;
            if self.remaining <= EPSILON {
                self.next();
            }
        }
    }

    fn next(&mut self) {
        self.index = (self.index + 1) % self.pattern.values.len();
        self.remaining = self.pattern.values[self.index];
    }
}

#[derive(Debug, Clone)]
struct FlatSubpath {
    points: Vec<Point>,
}

impl FlatSubpath {
    fn length(&self) -> f32 {
        self.points
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).length())
            .sum()
    }
}

fn dashed_path(path: &Path, pattern: &DashPattern) -> Path {
    let mut builder = Path::builder().with_svg();

    for subpath in flattened_subpaths(path) {
        let mut cursor = pattern.cursor();
        let mut open_dash = false;

        for pair in subpath.points.windows(2) {
            let mut current = pair[0];
            let to = pair[1];
            let vector = to - current;
            let length = vector.length();
            if length <= EPSILON {
                continue;
            }
            let direction = vector / length;
            let mut remaining = length;

            while remaining > EPSILON {
                if cursor.remaining <= EPSILON {
                    cursor.next();
                }
                let step = remaining.min(cursor.remaining);
                let next = current + direction * step;

                if cursor.is_painting() {
                    if !open_dash {
                        builder.move_to(current);
                        open_dash = true;
                    }
                    builder.line_to(next);
                } else if open_dash {
                    open_dash = false;
                }

                current = next;
                remaining -= step;
                cursor.advance(step);
            }
        }
    }

    builder.build()
}

fn flattened_subpaths(path: &Path) -> Vec<FlatSubpath> {
    let mut subpaths = Vec::new();
    let mut current: Vec<Point> = Vec::new();

    for event in path.iter().flattened(FLATTENING_TOLERANCE) {
        match event {
            PathEvent::Begin { at } => {
                flush_flat_subpath(&mut current, &mut subpaths);
                current.push(at);
            }
            PathEvent::Line { to, .. } => {
                if current
                    .last()
                    .is_none_or(|last| (*last - to).length() > EPSILON)
                {
                    current.push(to);
                }
            }
            PathEvent::End { first, close, .. } => {
                if close
                    && current
                        .last()
                        .is_some_and(|last| (*last - first).length() > EPSILON)
                {
                    current.push(first);
                }
                flush_flat_subpath(&mut current, &mut subpaths);
            }
            PathEvent::Quadratic { .. } | PathEvent::Cubic { .. } => unreachable!(),
        }
    }
    flush_flat_subpath(&mut current, &mut subpaths);

    subpaths
}

fn flush_flat_subpath(current: &mut Vec<Point>, subpaths: &mut Vec<FlatSubpath>) {
    if current.len() >= 2 {
        subpaths.push(FlatSubpath {
            points: std::mem::take(current),
        });
    } else {
        current.clear();
    }
}

#[derive(Debug, Clone)]
struct MarkerSubpath {
    vertices: Vec<MarkerVertex>,
}

#[derive(Debug, Clone, Copy)]
struct MarkerVertex {
    point: Point,
    incoming: Option<Vector>,
    outgoing: Option<Vector>,
}

fn marker_subpaths(path: &Path) -> Vec<MarkerSubpath> {
    let mut subpaths = Vec::new();
    let mut current: Vec<MarkerVertex> = Vec::new();

    for event in path.iter() {
        match event {
            PathEvent::Begin { at } => {
                flush_marker_subpath(&mut current, &mut subpaths);
                current.push(MarkerVertex {
                    point: at,
                    incoming: None,
                    outgoing: None,
                });
            }
            PathEvent::Line { from, to } => {
                push_marker_segment(&mut current, from, to, to - from, to - from);
            }
            PathEvent::Quadratic { from, ctrl, to } => {
                push_marker_segment(
                    &mut current,
                    from,
                    to,
                    non_zero_tangent(ctrl - from, to - from),
                    non_zero_tangent(to - ctrl, to - from),
                );
            }
            PathEvent::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            } => {
                push_marker_segment(
                    &mut current,
                    from,
                    to,
                    non_zero_tangent(ctrl1 - from, to - from),
                    non_zero_tangent(to - ctrl2, to - from),
                );
            }
            PathEvent::End { first, last, close } => {
                if close && (first - last).length() > EPSILON {
                    let dir = first - last;
                    let incoming = normalized(dir);
                    let outgoing = current.first().and_then(|vertex| vertex.outgoing);
                    if let Some(last_vertex) = current.last_mut() {
                        last_vertex.outgoing = incoming;
                    }
                    current.push(MarkerVertex {
                        point: first,
                        incoming,
                        outgoing,
                    });
                }
                flush_marker_subpath(&mut current, &mut subpaths);
            }
        }
    }
    flush_marker_subpath(&mut current, &mut subpaths);

    subpaths
}

fn push_marker_segment(
    current: &mut Vec<MarkerVertex>,
    from: Point,
    to: Point,
    outgoing: Vector,
    incoming: Vector,
) {
    if current.is_empty() {
        current.push(MarkerVertex {
            point: from,
            incoming: None,
            outgoing: None,
        });
    }

    if let Some(last) = current.last_mut() {
        last.outgoing = normalized(outgoing);
    }
    current.push(MarkerVertex {
        point: to,
        incoming: normalized(incoming),
        outgoing: None,
    });
}

fn flush_marker_subpath(current: &mut Vec<MarkerVertex>, subpaths: &mut Vec<MarkerSubpath>) {
    if current.len() >= 2 {
        subpaths.push(MarkerSubpath {
            vertices: std::mem::take(current),
        });
    } else {
        current.clear();
    }
}

fn non_zero_tangent(primary: Vector, fallback: Vector) -> Vector {
    if primary.length() > EPSILON {
        primary
    } else {
        fallback
    }
}

fn normalized(vector: Vector) -> Option<Vector> {
    (vector.length() > EPSILON).then(|| vector.normalize())
}

fn marker_angle(incoming: Option<Vector>, outgoing: Option<Vector>) -> f32 {
    match (incoming, outgoing) {
        (Some(a), Some(b)) => {
            let sum = a + b;
            if sum.length() > EPSILON {
                angle(Some(sum))
            } else {
                angle(Some(b))
            }
        }
        (Some(vector), None) | (None, Some(vector)) => angle(Some(vector)),
        (None, None) => 0.0,
    }
}

fn angle(vector: Option<Vector>) -> f32 {
    vector
        .map(|vector| vector.y.atan2(vector.x))
        .unwrap_or_default()
}

fn resolve_linecap(element: &Element) -> LineCap {
    match element
        .attributes
        .get("stroke-linecap")
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("round") => LineCap::Round,
        Some("square") => LineCap::Square,
        _ => LineCap::Butt,
    }
}

fn resolve_linejoin(element: &Element) -> LineJoin {
    match element
        .attributes
        .get("stroke-linejoin")
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("round") => LineJoin::Round,
        Some("bevel") => LineJoin::Bevel,
        Some("miter-clip") => LineJoin::MiterClip,
        _ => LineJoin::Miter,
    }
}

fn resolve_miterlimit(element: &Element) -> f32 {
    element
        .attributes
        .get("stroke-miterlimit")
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value >= StrokeOptions::MINIMUM_MITER_LIMIT)
        .unwrap_or(StrokeOptions::DEFAULT_MITER_LIMIT)
}

fn resolve_dasharray(element: &Element, viewport: Viewport) -> Option<Vec<f32>> {
    let value = element.attributes.get("stroke-dasharray")?.trim();
    if value.eq_ignore_ascii_case("none") {
        return None;
    }

    let mut values = Vec::new();
    for parsed in LengthListParser::from(value) {
        let length = parsed.ok()?;
        let resolved = resolve_svg_length(length, viewport.diagonal())?;
        if resolved < 0.0 {
            return None;
        }
        values.push(resolved);
    }

    if values.is_empty() || values.iter().all(|value| value.abs() <= EPSILON) {
        return None;
    }
    if values.len() % 2 == 1 {
        values.extend_from_within(..);
    }
    Some(values)
}

fn resolve_dashoffset(element: &Element, viewport: Viewport) -> f32 {
    element
        .attributes
        .get("stroke-dashoffset")
        .map(String::as_str)
        .and_then(Length::parse)
        .map(|length| length.resolve(viewport.diagonal()))
        .unwrap_or(0.0)
}

fn resolve_path_length(element: &Element) -> Option<f32> {
    element
        .attributes
        .get("pathLength")
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn resolve_svg_length(length: svgtypes::Length, percentage_basis: f32) -> Option<f32> {
    let number = length.number as f32;
    if !number.is_finite() {
        return None;
    }
    match length.unit {
        LengthUnit::None | LengthUnit::Px => Some(number),
        LengthUnit::Percent => Some(number / 100.0 * percentage_basis),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    fn element(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Path);
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
    fn resolve_stroke_style_reads_line_cap_join_and_miter() {
        let style = resolve_stroke_style(
            &element(&[
                ("stroke-width", "6"),
                ("stroke-linecap", "round"),
                ("stroke-linejoin", "bevel"),
                ("stroke-miterlimit", "2"),
            ]),
            vp(),
        );

        assert_eq!(style.width, 6.0);
        assert_eq!(style.linecap, LineCap::Round);
        assert_eq!(style.linejoin, LineJoin::Bevel);
        assert_eq!(style.miterlimit, 2.0);
    }

    #[test]
    fn resolve_dasharray_repeats_odd_lists_and_resolves_percentages() {
        let style = resolve_stroke_style(
            &element(&[("stroke-dasharray", "4, 2 10%")]),
            Viewport {
                width: 300.0,
                height: 400.0,
            },
        );

        assert_eq!(
            style.dasharray,
            Some(vec![4.0, 2.0, 35.355_34, 4.0, 2.0, 35.355_34])
        );
    }

    #[test]
    fn invalid_dasharray_falls_back_to_solid_stroke() {
        assert_eq!(
            resolve_stroke_style(&element(&[("stroke-dasharray", "-1 2")]), vp()).dasharray,
            None
        );
        assert_eq!(
            resolve_stroke_style(&element(&[("stroke-dasharray", "5mm 2")]), vp()).dasharray,
            None
        );
        assert_eq!(
            resolve_stroke_style(&element(&[("stroke-dasharray", "0 0")]), vp()).dasharray,
            None
        );
    }

    #[test]
    fn path_length_calibrates_dash_distances() {
        let path = path_from_points(&[(0.0, 0.0), (100.0, 0.0)], false).unwrap();
        let style = resolve_stroke_style(
            &element(&[
                ("stroke-dasharray", "10 10"),
                ("stroke-dashoffset", "5"),
                ("pathLength", "50"),
            ]),
            vp(),
        );
        let pattern = style.dash_pattern_for_path(&path).unwrap();

        assert_eq!(pattern.values, vec![20.0, 20.0]);
        assert_eq!(pattern.offset, 10.0);
    }

    #[test]
    fn dashed_path_emits_visible_subpaths_only() {
        let path = path_from_points(&[(0.0, 0.0), (30.0, 0.0)], false).unwrap();
        let pattern = DashPattern::new(vec![10.0, 5.0], 0.0).unwrap();
        let dashed = dashed_path(&path, &pattern);
        let events: Vec<PathEvent> = dashed.iter().collect();

        assert!(matches!(events[0], PathEvent::Begin { .. }));
        assert!(events
            .iter()
            .any(|event| matches!(event, PathEvent::End { close: false, .. })));
        assert!(path_length(&dashed) < path_length(&path));
    }

    #[test]
    fn marker_placements_reports_start_mid_and_end() {
        let path = path_from_points(&[(10.0, 20.0), (40.0, 20.0), (40.0, 60.0)], false).unwrap();
        let placements = marker_placements(&path);

        assert_eq!(placements.len(), 3);
        assert_eq!(placements[0].kind, MarkerKind::Start);
        assert_eq!((placements[0].x, placements[0].y), (10.0, 20.0));
        assert_eq!(placements[1].kind, MarkerKind::Mid);
        assert_eq!((placements[1].x, placements[1].y), (40.0, 20.0));
        assert_eq!(placements[2].kind, MarkerKind::End);
        assert_eq!((placements[2].x, placements[2].y), (40.0, 60.0));
    }

    #[test]
    fn closed_marker_placements_put_end_at_subpath_start() {
        let path = path_from_points(&[(10.0, 20.0), (40.0, 20.0), (40.0, 60.0)], true).unwrap();
        let placements = marker_placements(&path);

        assert_eq!(placements.len(), 4);
        assert_eq!(placements[0].kind, MarkerKind::Start);
        assert_eq!((placements[0].x, placements[0].y), (10.0, 20.0));
        assert_eq!(placements[1].kind, MarkerKind::Mid);
        assert_eq!((placements[1].x, placements[1].y), (40.0, 20.0));
        assert_eq!(placements[2].kind, MarkerKind::Mid);
        assert_eq!((placements[2].x, placements[2].y), (40.0, 60.0));
        assert_eq!(placements[3].kind, MarkerKind::End);
        assert_eq!((placements[3].x, placements[3].y), (10.0, 20.0));
    }
}
