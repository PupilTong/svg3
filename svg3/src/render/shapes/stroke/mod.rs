//! Shared SVG stroke geometry support.
//!
//! The renderer represents strokes as normal triangle meshes emitted by Lyon
//! and uploaded through the same GPU draw path as fills. This module resolves
//! the stroke presentation attributes common to all 2D shapes and handles
//! dashed strokes by slicing flattened paths into visible dash subpaths before
//! tessellation. Markers attach to the original (non-dashed) shape path; see
//! [`markers`].

use lyon_tessellation::geometry_builder::{BuffersBuilder, VertexBuffers};
use lyon_tessellation::path::math::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{LineCap, LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex};
use svgtypes::{LengthListParser, LengthUnit};

use crate::dom::Element;
use crate::render::shapes::{resolve_stroke_width, vertex, Length, Viewport};
use crate::render::{Mesh, Vertex};

mod dash;
mod markers;

pub(crate) const FLATTENING_TOLERANCE: f32 = 0.1;
/// Shared epsilon for the dash slicer and marker tangent helpers.
pub(super) const EPSILON: f32 = 1e-5;

pub(crate) use markers::{marker_placements, MarkerKind, MarkerPlacement};

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

    fn dash_pattern_for_path(&self, path: &Path) -> Option<dash::DashPattern> {
        let dasharray = self.dasharray.as_ref()?;
        let actual_length = path_length(path);
        let scale = self.path_length_scale(actual_length);
        let pattern: Vec<f32> = dasharray.iter().map(|value| value * scale).collect();
        let offset = self.dashoffset * scale;
        dash::DashPattern::new(pattern, offset)
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
        let stroke_path = dash::dashed_path(path, &pattern);
        tessellate_plain_stroke(&stroke_path, style, color)
    } else {
        tessellate_plain_stroke(path, style, color)
    }
}

/// Return the flattened length of a path in user units, including explicit
/// close segments.
pub(crate) fn path_length(path: &Path) -> f32 {
    dash::flattened_subpaths(path)
        .into_iter()
        .map(|subpath| subpath.length())
        .sum()
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
    use crate::dom::ElementKind;
    use lyon_tessellation::path::PathEvent;

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

        assert_eq!(pattern.values(), &[20.0, 20.0]);
        assert_eq!(pattern.offset(), 10.0);
    }

    #[test]
    fn dashed_path_emits_visible_subpaths_only() {
        let path = path_from_points(&[(0.0, 0.0), (30.0, 0.0)], false).unwrap();
        let pattern = dash::DashPattern::new(vec![10.0, 5.0], 0.0).unwrap();
        let dashed = dash::dashed_path(&path, &pattern);
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
