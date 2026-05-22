//! SVG 1.1 `<path>` — path-data parsing plus fill and stroke tessellation.
//!
//! `<path>` is the general two-dimensional SVG drawing primitive. This
//! module parses its `d` attribute with `svgtypes` and tessellates the
//! resulting path with Lyon, while keeping the rest of the crate's current
//! presentation-attribute model: `fill`, `stroke`, `stroke-width`,
//! `stroke-linecap`, `stroke-linejoin`, and `fill-rule` are read directly
//! from attributes; CSS and transforms are not applied yet.

use lyon_tessellation::geometry_builder::{BuffersBuilder, VertexBuffers};
use lyon_tessellation::path::builder::SvgPathBuilder;
use lyon_tessellation::path::math::{point, vector, Angle};
use lyon_tessellation::path::{ArcFlags, Path};
use lyon_tessellation::{
    FillOptions, FillRule, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex,
};
use svg3_dom::Element;
use svgtypes::{PathParser, PathSegment};

use super::{resolve_stroke_width, vertex, Viewport};
use crate::{Mesh, Vertex};

const FLATTENING_TOLERANCE: f32 = 0.1;

/// A `<path>`'s resolved geometry and path-local paint parameters.
#[derive(Debug)]
pub(crate) struct PathGeometry {
    path: Path,
    fill_rule: FillRule,
    stroke_width: f32,
    stroke_linecap: LineCap,
    stroke_linejoin: LineJoin,
    stroke_miterlimit: f32,
}

/// Resolve a `<path>`'s `d` data and stroke geometry attributes.
///
/// Returns `None` when `d` is absent or contains no usable path segments.
/// Malformed data after a valid prefix is rendered up to the first parser
/// error, matching SVG's "render up to the first error" path-data behavior.
pub(crate) fn resolve_path(element: &Element, viewport: Viewport) -> Option<PathGeometry> {
    let path = parse_path(element.attributes.get("d")?)?;
    Some(PathGeometry {
        path,
        fill_rule: resolve_fill_rule(element),
        stroke_width: resolve_stroke_width(element, viewport),
        stroke_linecap: resolve_linecap(element),
        stroke_linejoin: resolve_linejoin(element),
        stroke_miterlimit: resolve_miterlimit(element),
    })
}

/// Tessellate a resolved path's fill into triangle geometry.
pub(crate) fn tessellate_path_fill(geo: &PathGeometry, color: [f32; 4]) -> Mesh {
    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let options = FillOptions::default()
        .with_fill_rule(geo.fill_rule)
        .with_tolerance(FLATTENING_TOLERANCE);
    let mut tessellator = FillTessellator::new();
    let mut builder = BuffersBuilder::new(&mut buffers, move |v: FillVertex<'_>| {
        let p = v.position();
        vertex(p.x, p.y, color)
    });

    if tessellator
        .tessellate_path(&geo.path, &options, &mut builder)
        .is_err()
    {
        return Mesh::default();
    }

    Mesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

/// Tessellate a resolved path's stroke into triangle geometry.
pub(crate) fn tessellate_path_stroke(geo: &PathGeometry, color: [f32; 4]) -> Mesh {
    if geo.stroke_width <= 0.0 {
        return Mesh::default();
    }

    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let options = StrokeOptions::default()
        .with_line_width(geo.stroke_width)
        .with_line_cap(geo.stroke_linecap)
        .with_line_join(geo.stroke_linejoin)
        .with_miter_limit(geo.stroke_miterlimit)
        .with_tolerance(FLATTENING_TOLERANCE);
    let mut tessellator = StrokeTessellator::new();
    let mut builder = BuffersBuilder::new(&mut buffers, move |v: StrokeVertex<'_, '_>| {
        let p = v.position();
        vertex(p.x, p.y, color)
    });

    if tessellator
        .tessellate_path(&geo.path, &options, &mut builder)
        .is_err()
    {
        return Mesh::default();
    }

    Mesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

fn parse_path(data: &str) -> Option<Path> {
    let mut builder = Path::builder().with_svg();
    let mut saw_segment = false;

    for segment in PathParser::from(data) {
        let Ok(segment) = segment else {
            break;
        };
        if apply_segment(segment, &mut builder).is_none() {
            break;
        }
        saw_segment = true;
    }

    saw_segment.then(|| builder.build())
}

fn apply_segment(segment: PathSegment, builder: &mut impl SvgPathBuilder) -> Option<()> {
    match segment {
        PathSegment::MoveTo { abs, x, y } => {
            if abs {
                builder.move_to(to_point(x, y)?);
            } else {
                builder.relative_move_to(to_vector(x, y)?);
            }
        }
        PathSegment::LineTo { abs, x, y } => {
            if abs {
                builder.line_to(to_point(x, y)?);
            } else {
                builder.relative_line_to(to_vector(x, y)?);
            }
        }
        PathSegment::HorizontalLineTo { abs, x } => {
            if abs {
                builder.horizontal_line_to(to_f32(x)?);
            } else {
                builder.relative_horizontal_line_to(to_f32(x)?);
            }
        }
        PathSegment::VerticalLineTo { abs, y } => {
            if abs {
                builder.vertical_line_to(to_f32(y)?);
            } else {
                builder.relative_vertical_line_to(to_f32(y)?);
            }
        }
        PathSegment::CurveTo {
            abs,
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => {
            if abs {
                builder.cubic_bezier_to(to_point(x1, y1)?, to_point(x2, y2)?, to_point(x, y)?);
            } else {
                builder.relative_cubic_bezier_to(
                    to_vector(x1, y1)?,
                    to_vector(x2, y2)?,
                    to_vector(x, y)?,
                );
            }
        }
        PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
            if abs {
                builder.smooth_cubic_bezier_to(to_point(x2, y2)?, to_point(x, y)?);
            } else {
                builder.smooth_relative_cubic_bezier_to(to_vector(x2, y2)?, to_vector(x, y)?);
            }
        }
        PathSegment::Quadratic { abs, x1, y1, x, y } => {
            if abs {
                builder.quadratic_bezier_to(to_point(x1, y1)?, to_point(x, y)?);
            } else {
                builder.relative_quadratic_bezier_to(to_vector(x1, y1)?, to_vector(x, y)?);
            }
        }
        PathSegment::SmoothQuadratic { abs, x, y } => {
            if abs {
                builder.smooth_quadratic_bezier_to(to_point(x, y)?);
            } else {
                builder.smooth_relative_quadratic_bezier_to(to_vector(x, y)?);
            }
        }
        PathSegment::EllipticalArc {
            abs,
            rx,
            ry,
            x_axis_rotation,
            large_arc,
            sweep,
            x,
            y,
        } => {
            let radii = vector(to_f32(rx)?.abs(), to_f32(ry)?.abs());
            let rotation = Angle::degrees(to_f32(x_axis_rotation)?);
            let flags = ArcFlags { large_arc, sweep };
            if abs {
                builder.arc_to(radii, rotation, flags, to_point(x, y)?);
            } else {
                builder.relative_arc_to(radii, rotation, flags, to_vector(x, y)?);
            }
        }
        PathSegment::ClosePath { .. } => builder.close(),
    }
    Some(())
}

fn to_point(x: f64, y: f64) -> Option<lyon_tessellation::path::math::Point> {
    Some(point(to_f32(x)?, to_f32(y)?))
}

fn to_vector(x: f64, y: f64) -> Option<lyon_tessellation::path::math::Vector> {
    Some(vector(to_f32(x)?, to_f32(y)?))
}

fn to_f32(value: f64) -> Option<f32> {
    let value = value as f32;
    value.is_finite().then_some(value)
}

fn resolve_fill_rule(element: &Element) -> FillRule {
    match element
        .attributes
        .get("fill-rule")
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("evenodd") => FillRule::EvenOdd,
        _ => FillRule::NonZero,
    }
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
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(4.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    fn path(attrs: &[(&str, &str)]) -> Element {
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
    fn resolve_path_requires_d_attribute() {
        assert!(resolve_path(&path(&[]), vp()).is_none());
    }

    #[test]
    fn tessellate_path_fill_draws_closed_triangle() {
        let geo = resolve_path(&path(&[("d", "M 10 10 L 50 10 L 30 40 Z")]), vp()).unwrap();
        let mesh = tessellate_path_fill(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert!(!mesh.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.color == [0.0, 0.0, 1.0, 1.0]));
        assert_eq!(mesh.indices.len() % 3, 0);
    }

    #[test]
    fn tessellate_path_fill_closes_open_subpath() {
        let geo = resolve_path(&path(&[("d", "M 10 10 L 50 10 L 30 40")]), vp()).unwrap();
        let mesh = tessellate_path_fill(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert!(!mesh.is_empty());
    }

    #[test]
    fn path_data_error_keeps_valid_prefix() {
        let geo = resolve_path(&path(&[("d", "M 10 10 L 50 10 L 30 40 nope")]), vp()).unwrap();
        let mesh = tessellate_path_fill(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert!(!mesh.is_empty());
    }

    #[test]
    fn non_finite_f32_coordinate_keeps_valid_prefix() {
        let geo = resolve_path(&path(&[("d", "M 10 10 L 50 10 L 30 40 L 1e39 20")]), vp()).unwrap();
        let mesh = tessellate_path_fill(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert!(!mesh.is_empty());
    }

    #[test]
    fn tessellate_path_fill_supports_curves() {
        let geo = resolve_path(
            &path(&[("d", "M 20 60 C 20 15 80 15 80 60 Q 50 85 20 60 Z")]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_path_fill(&geo, [0.0, 0.0, 1.0, 1.0]);
        assert!(mesh.vertices.len() > 3);
        assert!(!mesh.indices.is_empty());
    }

    #[test]
    fn tessellate_path_stroke_supports_arcs() {
        let geo = resolve_path(
            &path(&[("d", "M 10 50 A 40 40 0 0 1 90 50"), ("stroke-width", "4")]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_path_stroke(&geo, [1.0, 0.0, 0.0, 1.0]);
        assert!(!mesh.is_empty());
    }

    #[test]
    fn tessellate_path_stroke_uses_stroke_width() {
        let geo = resolve_path(
            &path(&[
                ("d", "M 10 10 H 60 V 40"),
                ("stroke-width", "6"),
                ("stroke-linecap", "round"),
                ("stroke-linejoin", "bevel"),
            ]),
            vp(),
        )
        .unwrap();
        let mesh = tessellate_path_stroke(&geo, [1.0, 0.0, 0.0, 1.0]);
        assert!(!mesh.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.color == [1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn stroke_width_percentage_resolves_against_viewport_diagonal() {
        let geo = resolve_path(
            &path(&[("d", "M 0 0 L 10 0"), ("stroke-width", "10%")]),
            Viewport {
                width: 300.0,
                height: 400.0,
            },
        )
        .unwrap();
        assert!((geo.stroke_width - 35.355_34).abs() < 1e-4);
    }
}
