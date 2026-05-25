//! The document walk.
//!
//! [`build_scene`] turns a parsed [`Document`] into one combined [`Mesh`] by
//! dispatching every supported SVG element to its [`crate::shapes`]
//! tessellator, and [`document_viewport`] resolves the root `<svg>` sizing
//! that percentage lengths resolve against.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use svg3_dom::{Document, Element, ElementKind, NodeId};

use crate::filters::{FilterDefinitions, FilterInput, FilterPrimitive, FilterPrimitiveKind};
use crate::shapes::{self, stroke::MarkerKind};
use crate::{Mesh, Viewport};

/// A headless render operation in SVG painter's order.
#[derive(Debug, Clone)]
pub(crate) enum RenderOp {
    /// Draw a mesh directly into the destination target.
    Mesh(Mesh),
    /// Draw a mesh into an offscreen target, run the filter primitive chain
    /// through ping/pong GPU passes, then composite the result into the
    /// destination target.
    Filter {
        /// The geometry that produces the filter's source graphic.
        mesh: Mesh,
        /// Ordered list of primitive passes to apply.
        primitives: Vec<FilterPrimitive>,
    },
}

struct SceneContext<'a> {
    viewport: Viewport,
    markers: &'a MarkerDefinitions,
}

/// Walk `document` and tessellate every supported 2D SVG shape into one
/// combined [`Mesh`].
///
/// `viewport` is the basis for percentage lengths (e.g. `width="100%"`);
/// callers that want SVG root sizing should pass [`document_viewport`]. The
/// mesh is in SVG user space (origin top-left, y-down, `z = 0`). Shapes are
/// appended in document order, so a later shape paints over an earlier one. A
/// shape that is not rendered — a degenerate size, `fill="none"`, or a
/// missing/`none` stroke on stroke-only geometry — contributes nothing.
/// `transform` and grouping are not applied yet, so a shape is placed at its
/// own coordinates regardless of any ancestor `<g>`.
pub fn build_scene(document: &Document, viewport: Viewport) -> Mesh {
    let markers = MarkerDefinitions::default();
    let context = SceneContext {
        viewport,
        markers: &markers,
    };
    let mut mesh = Mesh::default();
    for child in document.node(document.root()).children.iter().copied() {
        append_subtree_mesh(document, child, &context, true, &mut mesh);
    }
    mesh
}

/// Build headless render operations that preserve SVG painter's order while
/// isolating filtered subtrees into their own GPU post-process pass.
pub(crate) fn build_render_plan(document: &Document, viewport: Viewport) -> Vec<RenderOp> {
    let filters = FilterDefinitions::collect(document);
    let markers = MarkerDefinitions::default();
    let context = SceneContext {
        viewport,
        markers: &markers,
    };
    let mut plan = Vec::new();
    let mut pending_mesh = Mesh::default();
    for child in document.node(document.root()).children.iter().copied() {
        append_render_ops(
            document,
            child,
            &filters,
            &context,
            &mut pending_mesh,
            &mut plan,
        );
    }
    flush_mesh(&mut pending_mesh, &mut plan);
    plan
}

fn append_render_ops(
    document: &Document,
    id: svg3_dom::NodeId,
    filters: &FilterDefinitions,
    context: &SceneContext<'_>,
    pending_mesh: &mut Mesh,
    plan: &mut Vec<RenderOp>,
) {
    let node = document.node(id);
    if is_definition_container(&node.element.kind) {
        return;
    }

    if let Some(chain) = filters.resolve(&node.element) {
        let mut filtered_mesh = Mesh::default();
        append_subtree_mesh(document, id, context, true, &mut filtered_mesh);
        // A primitive affects the chain output if either its parameters are
        // non-identity OR its DAG wiring is non-default. The wiring matters
        // because e.g. `<feGaussianBlur in="SourceAlpha" stdDeviation="0"/>`
        // is parameter-wise a no-op blur but still has to run — it must
        // replace the RGB with SourceAlpha's `(0, 0, 0, src.a)`. A primitive
        // with a `result` attribute is also "live" since a later primitive
        // might reference it.
        let affects_output = |primitive: &FilterPrimitive| {
            primitive.is_visible()
                || !matches!(primitive.input, FilterInput::Default)
                || !matches!(primitive.input2, FilterInput::Default)
                || primitive.result.is_some()
        };
        let visible = chain.iter().any(affects_output);
        let generator = chain.iter().any(|primitive| {
            matches!(
                primitive.kind,
                FilterPrimitiveKind::Image(_)
                    | FilterPrimitiveKind::Flood(_)
                    | FilterPrimitiveKind::Turbulence(_)
            )
        });
        if !filtered_mesh.is_empty() || generator {
            if visible {
                flush_mesh(pending_mesh, plan);
                plan.push(RenderOp::Filter {
                    mesh: filtered_mesh,
                    primitives: chain.to_vec(),
                });
            } else {
                pending_mesh.append(filtered_mesh);
            }
        }
        return;
    }

    append_element_mesh(document, &node.element, context, true, pending_mesh);
    for child in node.children.iter().copied() {
        append_render_ops(document, child, filters, context, pending_mesh, plan);
    }
}

fn append_subtree_mesh(
    document: &Document,
    id: NodeId,
    context: &SceneContext<'_>,
    include_markers: bool,
    mesh: &mut Mesh,
) {
    let node = document.node(id);
    if is_definition_container(&node.element.kind) {
        return;
    }
    append_element_mesh(document, &node.element, context, include_markers, mesh);
    for child in node.children.iter().copied() {
        // TODO: Nested filters need their own render plan and offscreen pass.
        // This first filter milestone treats a filtered subtree as raw source
        // geometry for the outer filter.
        append_subtree_mesh(document, child, context, include_markers, mesh);
    }
}

fn append_element_mesh(
    document: &Document,
    element: &Element,
    context: &SceneContext<'_>,
    include_markers: bool,
    mesh: &mut Mesh,
) {
    let viewport = context.viewport;
    let markers = context.markers;
    let features = element_features(element, include_markers);
    match &element.kind {
        ElementKind::Rect => {
            if let Some(geo) = shapes::rect::resolve_rect(element, viewport) {
                if let Some(color) =
                    shapes::resolve_fill_with_opacity(element, features.has_opacity_attrs)
                {
                    mesh.append(shapes::rect::tessellate_rect(&geo, color));
                }
                if let Some(color) = features
                    .has_strokes
                    .then(|| {
                        shapes::resolve_stroke_with_opacity(element, features.has_opacity_attrs)
                    })
                    .flatten()
                {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    mesh.append(shapes::rect::tessellate_rect_stroke(
                        &geo,
                        &stroke_style,
                        color,
                    ));
                }
            }
        }
        ElementKind::Circle => {
            if let Some(geo) = shapes::circle::resolve_circle(element, viewport) {
                if let Some(color) =
                    shapes::resolve_fill_with_opacity(element, features.has_opacity_attrs)
                {
                    mesh.append(shapes::circle::tessellate_circle(&geo, color));
                }
                if let Some(color) = features
                    .has_strokes
                    .then(|| {
                        shapes::resolve_stroke_with_opacity(element, features.has_opacity_attrs)
                    })
                    .flatten()
                {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    mesh.append(shapes::circle::tessellate_circle_stroke(
                        &geo,
                        &stroke_style,
                        color,
                    ));
                }
            }
        }
        ElementKind::Ellipse => {
            if let Some(geo) = shapes::ellipse::resolve_ellipse(element, viewport) {
                if let Some(color) =
                    shapes::resolve_fill_with_opacity(element, features.has_opacity_attrs)
                {
                    mesh.append(shapes::ellipse::tessellate_ellipse(&geo, color));
                }
                if let Some(color) = features
                    .has_strokes
                    .then(|| {
                        shapes::resolve_stroke_with_opacity(element, features.has_opacity_attrs)
                    })
                    .flatten()
                {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    mesh.append(shapes::ellipse::tessellate_ellipse_stroke(
                        &geo,
                        &stroke_style,
                        color,
                    ));
                }
            }
        }
        ElementKind::Polygon => {
            if let Some(geo) = shapes::polygon::resolve_polygon(element) {
                if let Some(color) =
                    shapes::resolve_fill_with_opacity(element, features.has_opacity_attrs)
                {
                    mesh.append(shapes::polygon::tessellate_polygon(&geo, color));
                }
                let has_markers = features.has_markers;
                let stroke = features
                    .has_strokes
                    .then(|| {
                        shapes::resolve_stroke_with_opacity(element, features.has_opacity_attrs)
                    })
                    .flatten();
                let stroke_style = (stroke.is_some() || has_markers)
                    .then(|| shapes::stroke::resolve_stroke_style(element, viewport));
                if let Some(color) = stroke {
                    let stroke_style = stroke_style
                        .as_ref()
                        .expect("stroke style should exist when stroke paint exists");
                    mesh.append(shapes::polygon::tessellate_polygon_stroke(
                        &geo,
                        stroke_style,
                        color,
                    ));
                }
                if has_markers {
                    append_marker_instances(
                        document,
                        markers,
                        element,
                        &shapes::polygon::to_path(&geo),
                        stroke_style
                            .as_ref()
                            .expect("stroke style should exist when markers exist")
                            .width,
                        mesh,
                    );
                }
            }
        }
        ElementKind::Polyline => {
            if let Some(geo) = shapes::polyline::resolve_polyline(element) {
                if let Some(color) =
                    shapes::resolve_fill_with_opacity(element, features.has_opacity_attrs)
                {
                    mesh.append(shapes::polyline::tessellate_polyline(&geo, color));
                }
                let has_markers = features.has_markers;
                let stroke = features
                    .has_strokes
                    .then(|| {
                        shapes::resolve_stroke_with_opacity(element, features.has_opacity_attrs)
                    })
                    .flatten();
                let stroke_style = (stroke.is_some() || has_markers)
                    .then(|| shapes::stroke::resolve_stroke_style(element, viewport));
                if let Some(color) = stroke {
                    let stroke_style = stroke_style
                        .as_ref()
                        .expect("stroke style should exist when stroke paint exists");
                    mesh.append(shapes::polyline::tessellate_polyline_stroke(
                        &geo,
                        stroke_style,
                        color,
                    ));
                }
                if has_markers {
                    append_marker_instances(
                        document,
                        markers,
                        element,
                        &shapes::polyline::to_path(&geo),
                        stroke_style
                            .as_ref()
                            .expect("stroke style should exist when markers exist")
                            .width,
                        mesh,
                    );
                }
            }
        }
        ElementKind::Line => {
            if let Some(geo) = shapes::line::resolve_line(element, viewport) {
                // Joins and miter limits are intentionally absent from
                // `has_general_line_strokes`; they do not change a single
                // open segment, so a butt-capped solid line can stay on the
                // SDF segment fast path.
                if !features.has_markers
                    && !features.has_general_line_strokes
                    && !features.has_opacity_attrs
                {
                    if let Some(color) = shapes::resolve_stroke_with_opacity(element, false) {
                        mesh.append(shapes::line::tessellate_segment(
                            &geo,
                            geo.stroke_width,
                            color,
                        ));
                    }
                    return;
                }

                let has_markers = features.has_markers;
                let stroke =
                    shapes::resolve_stroke_with_opacity(element, features.has_opacity_attrs);
                let needs_general_stroke = stroke.is_some() && features.has_general_line_strokes;
                let stroke_width = (!needs_general_stroke && (stroke.is_some() || has_markers))
                    .then_some(geo.stroke_width);
                let stroke_style = (needs_general_stroke && (stroke.is_some() || has_markers))
                    .then(|| shapes::stroke::resolve_stroke_style(element, viewport));
                if let Some(color) = stroke {
                    if let Some(stroke_width) = stroke_width {
                        mesh.append(shapes::line::tessellate_segment(&geo, stroke_width, color));
                    } else {
                        let stroke_style = stroke_style
                            .as_ref()
                            .expect("stroke style should exist when stroke paint exists");
                        mesh.append(shapes::line::tessellate_line(&geo, stroke_style, color));
                    }
                }
                if has_markers {
                    let marker_stroke_width = stroke_width
                        .or_else(|| stroke_style.as_ref().map(|style| style.width))
                        .expect("stroke width should exist when markers exist");
                    append_marker_instances(
                        document,
                        markers,
                        element,
                        &shapes::line::to_path(&geo),
                        marker_stroke_width,
                        mesh,
                    );
                }
            }
        }
        ElementKind::Path => {
            if let Some(geo) = shapes::path::resolve_path(element, viewport) {
                if let Some(color) =
                    shapes::resolve_fill_with_opacity(element, features.has_opacity_attrs)
                {
                    mesh.append(shapes::path::tessellate_path_fill(&geo, color));
                }
                if let Some(color) = features
                    .has_strokes
                    .then(|| {
                        shapes::resolve_stroke_with_opacity(element, features.has_opacity_attrs)
                    })
                    .flatten()
                {
                    mesh.append(shapes::path::tessellate_path_stroke(&geo, color));
                }
                if features.has_markers {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    append_marker_instances(
                        document,
                        markers,
                        element,
                        geo.path(),
                        stroke_style.width,
                        mesh,
                    );
                }
            }
        }
        _ => {}
    }
}

#[derive(Debug, Default)]
struct MarkerDefinitions {
    // Collected on the first actual marker reference. No-marker documents are
    // common and should not pay a separate definition walk.
    markers: OnceLock<BTreeMap<String, MarkerDefinition>>,
}

impl MarkerDefinitions {
    fn marker_refs(&self, document: &Document, element: &Element) -> MarkerRefs {
        let all = element
            .attributes
            .get("marker")
            .and_then(|value| self.resolve_reference(document, value));
        MarkerRefs {
            start: element
                .attributes
                .get("marker-start")
                .map(|value| self.resolve_reference(document, value))
                .unwrap_or(all),
            mid: element
                .attributes
                .get("marker-mid")
                .map(|value| self.resolve_reference(document, value))
                .unwrap_or(all),
            end: element
                .attributes
                .get("marker-end")
                .map(|value| self.resolve_reference(document, value))
                .unwrap_or(all),
        }
    }

    fn resolve_reference(&self, document: &Document, value: &str) -> Option<MarkerDefinition> {
        let id = url_reference_id(value)?;
        self.markers(document).get(id).copied()
    }

    fn markers(&self, document: &Document) -> &BTreeMap<String, MarkerDefinition> {
        self.markers
            .get_or_init(|| collect_marker_definitions(document))
    }
}

fn collect_marker_definitions(document: &Document) -> BTreeMap<String, MarkerDefinition> {
    let mut definitions = BTreeMap::new();
    let mut stack = vec![document.root()];
    while let Some(id) = stack.pop() {
        let node = document.node(id);
        if node.element.kind == ElementKind::Marker {
            if let Some(marker_id) = node.element.attributes.get("id") {
                definitions
                    .entry(marker_id.to_owned())
                    .or_insert_with(|| MarkerDefinition::resolve(id, &node.element));
            }
            continue;
        }
        stack.extend(node.children.iter().rev().copied());
    }
    definitions
}

#[derive(Debug, Clone, Copy)]
struct MarkerDefinition {
    node: NodeId,
    marker_width: f32,
    marker_height: f32,
    ref_x: f32,
    ref_y: f32,
    marker_units: MarkerUnits,
    orient: MarkerOrient,
    view_box: Option<ViewBox>,
}

impl MarkerDefinition {
    fn resolve(node: NodeId, element: &Element) -> Self {
        let marker_width = shapes::resolve_length(element, "markerWidth", 3.0)
            .filter(|value| *value > 0.0)
            .unwrap_or(3.0);
        let marker_height = shapes::resolve_length(element, "markerHeight", 3.0)
            .filter(|value| *value > 0.0)
            .unwrap_or(3.0);
        let marker_viewport = Viewport {
            width: marker_width,
            height: marker_height,
        };

        Self {
            node,
            marker_width,
            marker_height,
            ref_x: shapes::resolve_length(element, "refX", marker_viewport.width).unwrap_or(0.0),
            ref_y: shapes::resolve_length(element, "refY", marker_viewport.height).unwrap_or(0.0),
            marker_units: MarkerUnits::resolve(element),
            orient: MarkerOrient::resolve(element),
            view_box: element
                .attributes
                .get("viewBox")
                .and_then(|value| ViewBox::parse(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerUnits {
    StrokeWidth,
    UserSpaceOnUse,
}

impl MarkerUnits {
    fn resolve(element: &Element) -> Self {
        match element
            .attributes
            .get("markerUnits")
            .map(|value| value.trim())
        {
            Some("userSpaceOnUse") => Self::UserSpaceOnUse,
            _ => Self::StrokeWidth,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum MarkerOrient {
    Auto,
    AutoStartReverse,
    Angle(f32),
}

impl MarkerOrient {
    fn resolve(element: &Element) -> Self {
        let Some(value) = element.attributes.get("orient").map(|value| value.trim()) else {
            return Self::Angle(0.0);
        };
        if value.eq_ignore_ascii_case("auto") {
            return Self::Auto;
        }
        if value.eq_ignore_ascii_case("auto-start-reverse") {
            return Self::AutoStartReverse;
        }
        parse_angle(value)
            .map(Self::Angle)
            .unwrap_or(Self::Angle(0.0))
    }

    fn angle(self, kind: MarkerKind, auto_angle: f32) -> f32 {
        match self {
            Self::Auto => auto_angle,
            Self::AutoStartReverse if kind == MarkerKind::Start => {
                auto_angle + std::f32::consts::PI
            }
            Self::AutoStartReverse => auto_angle,
            Self::Angle(angle) => angle,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ViewBox {
    min_x: f32,
    min_y: f32,
    width: f32,
    height: f32,
}

impl ViewBox {
    fn parse(value: &str) -> Option<Self> {
        let values: Vec<f32> = value
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .filter(|part| !part.is_empty())
            .map(str::parse::<f32>)
            .collect::<Result<_, _>>()
            .ok()?;
        match values.as_slice() {
            [min_x, min_y, width, height] if *width > 0.0 && *height > 0.0 => Some(Self {
                min_x: *min_x,
                min_y: *min_y,
                width: *width,
                height: *height,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
struct MarkerRefs {
    start: Option<MarkerDefinition>,
    mid: Option<MarkerDefinition>,
    end: Option<MarkerDefinition>,
}

impl MarkerRefs {
    fn get(&self, kind: MarkerKind) -> Option<MarkerDefinition> {
        match kind {
            MarkerKind::Start => self.start,
            MarkerKind::Mid => self.mid,
            MarkerKind::End => self.end,
        }
    }

    fn is_empty(&self) -> bool {
        self.start.is_none() && self.mid.is_none() && self.end.is_none()
    }
}

fn append_marker_instances(
    document: &Document,
    markers: &MarkerDefinitions,
    element: &Element,
    path: &lyon_tessellation::path::Path,
    stroke_width: f32,
    mesh: &mut Mesh,
) {
    let refs = markers.marker_refs(document, element);
    if refs.is_empty() {
        return;
    }

    for placement in shapes::stroke::marker_placements(path) {
        let Some(marker) = refs.get(placement.kind) else {
            continue;
        };
        let mut marker_mesh = Mesh::default();
        let marker_viewport = Viewport {
            width: marker.marker_width,
            height: marker.marker_height,
        };
        let marker_context = SceneContext {
            viewport: marker_viewport,
            markers,
        };
        // Marker subtrees render with marker expansion disabled. This keeps
        // authored marker references inside a marker from recursively
        // instancing other marker definitions.
        for child in document.node(marker.node).children.iter().copied() {
            append_subtree_mesh(document, child, &marker_context, false, &mut marker_mesh);
        }
        if marker_mesh.is_empty() {
            continue;
        }
        transform_marker_mesh(&mut marker_mesh, marker, placement, stroke_width);
        mesh.append(marker_mesh);
    }
}

#[derive(Debug, Default)]
struct ElementFeatures {
    has_strokes: bool,
    has_markers: bool,
    has_opacity_attrs: bool,
    has_general_line_strokes: bool,
}

fn element_features(element: &Element, check_markers: bool) -> ElementFeatures {
    let mut features = ElementFeatures::default();
    let scan_stroke_paint = element.kind != ElementKind::Line;
    for (name, value) in &element.attributes {
        // Attributes are stored in a BTreeMap. Every feature flag watched here
        // sorts before `stroke-width`; update this guard when adding a watched
        // attribute that sorts later.
        if name.as_str() >= "stroke-width" {
            break;
        }
        match name.as_str() {
            "stroke" if scan_stroke_paint => {
                features.has_strokes |= !value.trim().eq_ignore_ascii_case("none");
            }
            "stroke-linecap" => {
                features.has_general_line_strokes |= !value.trim().eq_ignore_ascii_case("butt");
            }
            "stroke-dasharray" => {
                features.has_general_line_strokes |= !value.trim().eq_ignore_ascii_case("none");
            }
            "marker" | "marker-start" | "marker-mid" | "marker-end" if check_markers => {
                features.has_markers |= !value.trim().eq_ignore_ascii_case("none");
            }
            "opacity" | "fill-opacity" | "stroke-opacity" => {
                features.has_opacity_attrs = true;
            }
            _ => {}
        }
    }
    features
}

fn transform_marker_mesh(
    mesh: &mut Mesh,
    marker: MarkerDefinition,
    placement: shapes::stroke::MarkerPlacement,
    stroke_width: f32,
) {
    let (view_sx, view_sy, view_tx, view_ty) = marker
        .view_box
        .map(|view_box| {
            (
                marker.marker_width / view_box.width,
                marker.marker_height / view_box.height,
                -view_box.min_x * marker.marker_width / view_box.width,
                -view_box.min_y * marker.marker_height / view_box.height,
            )
        })
        .unwrap_or((1.0, 1.0, 0.0, 0.0));
    let ref_x = marker.ref_x * view_sx + view_tx;
    let ref_y = marker.ref_y * view_sy + view_ty;
    let unit_scale = match marker.marker_units {
        MarkerUnits::StrokeWidth => stroke_width.max(0.0),
        MarkerUnits::UserSpaceOnUse => 1.0,
    };
    let angle = marker.orient.angle(placement.kind, placement.angle);
    let (sin, cos) = angle.sin_cos();

    for vertex in &mut mesh.vertices {
        let local_x = vertex.position[0] * view_sx + view_tx - ref_x;
        let local_y = vertex.position[1] * view_sy + view_ty - ref_y;
        let x = local_x * unit_scale;
        let y = local_y * unit_scale;
        vertex.position[0] = placement.x + x * cos - y * sin;
        vertex.position[1] = placement.y + x * sin + y * cos;
    }
}

fn parse_angle(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let (number, radians_per_unit) = if let Some(number) = trimmed.strip_suffix("deg") {
        (number.trim(), std::f32::consts::PI / 180.0)
    } else if let Some(number) = trimmed.strip_suffix("grad") {
        (number.trim(), std::f32::consts::PI / 200.0)
    } else if let Some(number) = trimmed.strip_suffix("rad") {
        (number.trim(), 1.0)
    } else {
        (trimmed, std::f32::consts::PI / 180.0)
    };
    let value = number.parse::<f32>().ok()?;
    value.is_finite().then_some(value * radians_per_unit)
}

fn url_reference_id(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return None;
    }
    let inner = value.strip_prefix("url(")?.strip_suffix(')')?.trim();
    let inner = inner
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .or_else(|| {
            inner
                .strip_prefix('\'')
                .and_then(|quoted| quoted.strip_suffix('\''))
        })
        .unwrap_or(inner)
        .trim();
    let id = inner.strip_prefix('#')?.trim();
    (!id.is_empty()).then_some(id)
}

fn is_definition_container(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Defs | ElementKind::Filter | ElementKind::Marker
    )
}

fn flush_mesh(mesh: &mut Mesh, plan: &mut Vec<RenderOp>) {
    if !mesh.is_empty() {
        plan.push(RenderOp::Mesh(std::mem::take(mesh)));
    }
}

/// Resolve the document viewport from the root `<svg width>` / `<svg height>`.
///
/// Missing, unparseable, or non-positive dimensions fall back to `fallback`.
/// Percentage dimensions resolve against `fallback`, matching SVG's default
/// `100%` sizing behavior for a standalone document.
pub fn document_viewport(document: &Document, fallback: Viewport) -> Viewport {
    let root = document.element(document.root());
    Viewport {
        width: root
            .attributes
            .get("width")
            .and_then(|value| shapes::Length::parse(value))
            .map(|length| length.resolve(fallback.width))
            .filter(|value| *value > 0.0)
            .unwrap_or(fallback.width),
        height: root
            .attributes
            .get("height")
            .and_then(|value| shapes::Length::parse(value))
            .map(|length| length.resolve(fallback.height))
            .filter(|value| *value > 0.0)
            .unwrap_or(fallback.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::KIND_ELLIPSE;

    /// A 100×100 viewport for `build_scene` tests, whose fixtures use
    /// absolute lengths (so the viewport value does not affect the result).
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn build_scene_tessellates_each_rect_in_document() {
        // Two rects: one renderable, one zero-width and skipped
        // (WPT `shapes/rect-05`).
        let document = svg3_dom::parse(
            r#"<svg><rect width="10" height="10"/><rect width="0" height="10"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        // Only the first rect contributes: one sharp quad.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn build_scene_tessellates_rect_stroke() {
        // WPT `svg/shapes/rect-04.svg`: a rounded rect with `fill="none"`
        // and a visible stroke should render its stroke outline.
        let document = svg3_dom::parse(
            r#"<svg><rect x="10" y="10" width="50" height="50" rx="8" ry="8" fill="none" stroke="blue" stroke-width="4"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        assert!(
            !mesh.is_empty(),
            "rect stroke geometry should be emitted even when fill is none"
        );
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn build_scene_offsets_indices_across_rects() {
        let document = svg3_dom::parse(
            r#"<svg><rect width="10" height="10"/><rect x="20" width="10" height="10"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        assert_eq!(mesh.vertices.len(), 8);
        // The second quad's indices are offset past the first quad's vertices.
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn build_scene_is_empty_without_shapes() {
        let document = svg3_dom::parse("<svg><g/></svg>").unwrap();
        assert!(build_scene(&document, vp()).is_empty());
    }

    #[test]
    fn build_scene_skips_filter_definition_subtrees() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="unused"><rect width="100" height="100" fill="red"/><feGaussianBlur stdDeviation="4"/></filter><rect width="10" height="10" fill="blue"/></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert_eq!(mesh.vertices.len(), 4);
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn build_scene_skips_defs_subtrees() {
        let document = svg3_dom::parse(
            r##"<svg><defs><rect width="100" height="100" fill="red"/></defs><rect width="10" height="10" fill="blue"/></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert_eq!(mesh.vertices.len(), 4);
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn render_plan_isolates_filtered_subtree_in_painter_order() {
        let document = svg3_dom::parse(
            r##"<svg><rect width="10" height="10" fill="blue"/><filter id="soft"><feGaussianBlur stdDeviation="3"/></filter><g filter="url(#soft)"><rect x="20" width="10" height="10" fill="red"/></g><rect x="40" width="10" height="10" fill="green"/></svg>"##,
        )
        .unwrap();
        let plan = build_render_plan(&document, vp());

        assert_eq!(plan.len(), 3);
        assert!(matches!(plan[0], RenderOp::Mesh(_)));
        assert!(matches!(plan[1], RenderOp::Filter { .. }));
        assert!(matches!(plan[2], RenderOp::Mesh(_)));
    }

    #[test]
    fn render_plan_isolates_fe_image_filter_in_painter_order() {
        let document = svg3_dom::parse(
            r##"<svg><rect width="10" height="10" fill="blue"/><filter id="tex"><feImage href="data:image/png;base64,abc" x="20" y="0" width="10" height="10"/></filter><rect width="10" height="10" fill="red" filter="url(#tex)"/><rect x="40" width="10" height="10" fill="green"/></svg>"##,
        )
        .unwrap();
        let plan = build_render_plan(&document, vp());

        assert_eq!(plan.len(), 3);
        assert!(matches!(plan[0], RenderOp::Mesh(_)));
        let RenderOp::Filter { primitives, .. } = &plan[1] else {
            panic!("expected feImage filter op");
        };
        assert!(matches!(
            primitives.as_slice(),
            [FilterPrimitive {
                kind: FilterPrimitiveKind::Image(_),
                ..
            }]
        ));
        assert!(matches!(plan[2], RenderOp::Mesh(_)));
    }

    #[test]
    fn build_scene_tessellates_circle() {
        // A `<circle>` is dispatched to the circle tessellator and
        // contributes one SDF-covered bounding quad to the combined mesh.
        let document = svg3_dom::parse(r#"<svg><circle cx="20" cy="20" r="10"/></svg>"#).unwrap();
        let mesh = build_scene(&document, vp());
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.vertices[0].kind, KIND_ELLIPSE);
    }

    #[test]
    fn build_scene_tessellates_ellipse() {
        // An `<ellipse>` is dispatched to the ellipse tessellator and
        // contributes one SDF-covered bounding quad to the combined mesh.
        let document =
            svg3_dom::parse(r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/></svg>"#).unwrap();
        let mesh = build_scene(&document, vp());
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.vertices[0].kind, KIND_ELLIPSE);
    }

    #[test]
    fn build_scene_offsets_indices_across_ellipses() {
        // Two `<ellipse>`s combine into one mesh; the second quad's indices
        // are offset past the first quad's vertices so the triangle list
        // stays valid.
        let one = build_scene(
            &svg3_dom::parse(r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/></svg>"#).unwrap(),
            vp(),
        );
        let two = build_scene(
            &svg3_dom::parse(
                r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/><ellipse cx="60" cy="60" rx="12" ry="9"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        let single = one.vertices.len();
        // The combined mesh holds both fans.
        assert_eq!(two.vertices.len(), 2 * single);
        assert_eq!(two.indices.len(), 2 * one.indices.len());
        // The first quad is copied verbatim; the second is that same quad
        // with every index shifted by the first ellipse's vertex count.
        assert_eq!(two.indices[..one.indices.len()], one.indices[..]);
        let shifted: Vec<u32> = one.indices.iter().map(|i| i + single as u32).collect();
        assert_eq!(two.indices[one.indices.len()..], shifted[..]);
    }

    #[test]
    fn build_scene_combines_ellipse_with_rect_and_circle() {
        // A heterogeneous document: `<rect>`, `<circle>` and `<ellipse>` are
        // each dispatched to their own tessellator and appended in document
        // order into one combined mesh.
        let prefix = build_scene(
            &svg3_dom::parse(
                r#"<svg><rect width="10" height="10"/><circle cx="40" cy="40" r="12"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        let full = build_scene(
            &svg3_dom::parse(
                r#"<svg><rect width="10" height="10"/><circle cx="40" cy="40" r="12"/><ellipse cx="70" cy="30" rx="18" ry="9"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        // Adding the ellipse only grows the mesh past the rect+circle prefix.
        assert!(full.vertices.len() > prefix.vertices.len());
        assert!(full.indices.len() > prefix.indices.len());
        // The ellipse is last in document order, so its SDF quad — tagged
        // `KIND_ELLIPSE`, with `rx`/`ry` in `params` — starts past that
        // rect+circle prefix.
        let first = full.vertices[prefix.vertices.len()];
        assert_eq!(first.kind, KIND_ELLIPSE);
        assert_eq!(first.params, [18.0, 9.0, 0.0, 0.0]);
    }

    #[test]
    fn build_scene_finds_ellipse_inside_nested_groups() {
        // `build_scene` walks the whole tree, so an `<ellipse>` buried under
        // `<g>` wrappers is still found and tessellated.
        let document = svg3_dom::parse(
            r#"<svg><g><g><ellipse cx="25" cy="35" rx="10" ry="6"/></g></g></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        // The ellipse, reached despite the `<g>` wrappers, is an SDF quad.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.vertices[0].kind, KIND_ELLIPSE);
    }

    #[test]
    fn build_scene_skips_ellipse_with_fill_none() {
        // `fill="none"` resolves to no paint, so that ellipse contributes no
        // geometry — only the second, filled ellipse is tessellated.
        let document = svg3_dom::parse(
            r#"<svg><ellipse cx="10" cy="10" rx="8" ry="5" fill="none"/><ellipse cx="40" cy="40" rx="8" ry="5" fill="blue"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        // Exactly one SDF quad: the `fill="none"` ellipse contributes no
        // geometry — only the second, blue-filled ellipse is tessellated.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.vertices[0].kind, KIND_ELLIPSE);
        assert_eq!(mesh.vertices[0].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn build_scene_tessellates_polygon() {
        // A `<polygon>` is dispatched to the polygon tessellator; a simple
        // triangle contributes exactly one ear-clipped fill triangle.
        let document = svg3_dom::parse(r#"<svg><polygon points="0,0 20,0 10,16"/></svg>"#).unwrap();
        let mesh = build_scene(&document, vp());
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
    }

    #[test]
    fn build_scene_tessellates_basic_shape_strokes() {
        for source in [
            r##"<svg><rect x="10" y="10" width="40" height="30" fill="none" stroke="red" stroke-width="4"/></svg>"##,
            r##"<svg><circle cx="40" cy="40" r="20" fill="none" stroke="red" stroke-width="4"/></svg>"##,
            r##"<svg><ellipse cx="40" cy="40" rx="24" ry="12" fill="none" stroke="red" stroke-width="4"/></svg>"##,
            r##"<svg><polygon points="10,10 60,10 40,50" fill="none" stroke="red" stroke-width="4"/></svg>"##,
            r##"<svg><polyline points="10,10 60,10 40,50" fill="none" stroke="red" stroke-width="4"/></svg>"##,
        ] {
            let document = svg3_dom::parse(source).unwrap();
            let mesh = build_scene(&document, vp());
            assert!(!mesh.is_empty(), "{source} produced no stroke geometry");
            assert_eq!(mesh.indices.len() % 3, 0);
            assert!(mesh
                .vertices
                .iter()
                .all(|vertex| vertex.color == [1.0, 0.0, 0.0, 1.0]));
        }
    }

    #[test]
    fn build_scene_tessellates_polyline_fill() {
        let document =
            svg3_dom::parse(r#"<svg><polyline points="10,10 50,10 30,40"/></svg>"#).unwrap();
        let mesh = build_scene(&document, vp());
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
        assert_eq!(mesh.vertices[0].position, [10.0, 10.0, 0.0]);
        assert_eq!(mesh.vertices[1].position, [50.0, 10.0, 0.0]);
        assert_eq!(mesh.vertices[2].position, [30.0, 40.0, 0.0]);
    }

    #[test]
    fn build_scene_skips_polyline_without_fill_geometry() {
        let document = svg3_dom::parse(
            r#"<svg><polyline points="10,10 50,10" fill="blue"/><polyline points="10,10 50,10 30,40" fill="none"/></svg>"#,
        )
        .unwrap();
        assert!(build_scene(&document, vp()).is_empty());
    }

    #[test]
    fn build_scene_tessellates_line() {
        // A `<line>` needs renderable stroke paint; the other lines are
        // skipped because SVG's initial `stroke` value is `none`, explicit
        // `none` also paints nothing, and invalid stroke paint falls back to
        // the initial `none`.
        let document = svg3_dom::parse(
            r#"<svg><line x1="10" y1="20" x2="50" y2="20" stroke="blue" stroke-width="4"/><line x1="10" y1="40" x2="50" y2="40"/><line x1="10" y1="50" x2="50" y2="50" stroke="none"/><line x1="10" y1="60" x2="50" y2="60" stroke="bogus"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        // Exactly the one stroked line contributes stroke triangles.
        assert!(!mesh.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn build_scene_applies_dasharray_to_shape_strokes() {
        let solid = build_scene(
            &svg3_dom::parse(
                r#"<svg><line x1="10" y1="50" x2="90" y2="50" stroke="blue" stroke-width="6"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        let dashed = build_scene(
            &svg3_dom::parse(
                r#"<svg><line x1="10" y1="50" x2="90" y2="50" stroke="blue" stroke-width="6" stroke-dasharray="10 10" pathLength="40"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );

        assert!(!dashed.is_empty());
        assert!(dashed.vertices.len() > solid.vertices.len());
    }

    #[test]
    fn build_scene_renders_referenced_markers_after_stroke() {
        let document = svg3_dom::parse(
            r##"<svg><defs><marker id="arrow" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="10" refX="0" refY="0" orient="auto"><path d="M0 0 L4 2 L0 4 Z" fill="red"/></marker></defs><line x1="10" y1="50" x2="90" y2="50" stroke="none" marker-end="url(#arrow)"/></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert!(!mesh.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [1.0, 0.0, 0.0, 1.0]));
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| vertex.position[0] >= 90.0),
            "marker definition geometry should be instanced at the line end, not drawn in <defs>"
        );
    }

    #[test]
    fn marker_start_none_overrides_marker_shorthand() {
        let document = svg3_dom::parse(
            r##"<svg><defs><marker id="dot" markerUnits="userSpaceOnUse" markerWidth="4" markerHeight="4" orient="0"><rect width="4" height="4" fill="red"/></marker></defs><line x1="10" y1="50" x2="90" y2="50" stroke="none" marker="url(#dot)" marker-start="none"/></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        let red_vertices: Vec<_> = mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.color == [1.0, 0.0, 0.0, 1.0])
            .collect();

        assert!(!red_vertices.is_empty());
        assert!(
            red_vertices.iter().all(|vertex| vertex.position[0] >= 90.0),
            "`marker-start=\"none\"` should suppress the shorthand marker at the start"
        );
    }

    #[test]
    fn closed_polygon_marker_end_lands_at_start_vertex() {
        let document = svg3_dom::parse(
            r##"<svg><defs><marker id="dot" markerUnits="userSpaceOnUse" markerWidth="4" markerHeight="4" orient="0"><rect width="4" height="4" fill="red"/></marker></defs><polygon points="20,20 80,20 80,80" fill="none" stroke="none" marker-end="url(#dot)"/></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        let red_vertices: Vec<_> = mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.color == [1.0, 0.0, 0.0, 1.0])
            .collect();

        assert!(!red_vertices.is_empty());
        assert!(
            red_vertices.iter().all(|vertex| {
                (20.0..=24.0).contains(&vertex.position[0])
                    && (20.0..=24.0).contains(&vertex.position[1])
            }),
            "`marker-end` on a closed polygon should be placed at the initial vertex"
        );
    }

    #[test]
    fn parse_angle_accepts_svg_angle_units() {
        fn assert_close(actual: f32, expected: f32) {
            assert!(
                (actual - expected).abs() < 1e-6,
                "expected {expected}, got {actual}"
            );
        }

        assert_close(parse_angle("45").unwrap(), 45.0_f32.to_radians());
        assert_close(parse_angle("45deg").unwrap(), 45.0_f32.to_radians());
        assert_close(
            parse_angle("1.5707964rad").unwrap(),
            std::f32::consts::FRAC_PI_2,
        );
        assert_close(parse_angle("100grad").unwrap(), std::f32::consts::FRAC_PI_2);
        assert!(parse_angle("nan").is_none());
    }

    #[test]
    fn build_scene_tessellates_path_fill() {
        let document =
            svg3_dom::parse(r#"<svg><path d="M 10 10 L 50 10 L 30 40 Z" fill="blue"/></svg>"#)
                .unwrap();
        let mesh = build_scene(&document, vp());
        assert!(!mesh.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.color == [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn build_scene_paints_path_fill_before_stroke() {
        let document = svg3_dom::parse(
            r#"<svg><path d="M 10 10 H 60 V 40 Z" fill="blue" stroke="red" stroke-width="4"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        let first_stroke = mesh
            .vertices
            .iter()
            .position(|v| v.color == [1.0, 0.0, 0.0, 1.0])
            .expect("stroke vertices should be appended after fill vertices");
        assert!(first_stroke > 0);
        assert!(
            mesh.vertices[..first_stroke]
                .iter()
                .all(|v| v.color == [0.0, 0.0, 1.0, 1.0]),
            "fill vertices should precede stroke vertices"
        );
    }

    #[test]
    fn document_viewport_reads_root_width_and_height() {
        let document = svg3_dom::parse(r#"<svg width="300" height="200"/>"#).unwrap();
        let viewport = document_viewport(
            &document,
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(viewport.width, 300.0);
        assert_eq!(viewport.height, 200.0);
    }

    #[test]
    fn document_viewport_resolves_percentage_root_dimensions_against_fallback() {
        let document = svg3_dom::parse(r#"<svg width="50%" height="25%"/>"#).unwrap();
        let viewport = document_viewport(
            &document,
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(viewport.width, 400.0);
        assert_eq!(viewport.height, 150.0);
    }

    #[test]
    fn build_scene_resolves_percentages_against_svg_root_size() {
        let document = svg3_dom::parse(
            r#"<svg width="300" height="200"><rect width="100%" height="100%"/></svg>"#,
        )
        .unwrap();
        let viewport = document_viewport(
            &document,
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        let mesh = build_scene(&document, viewport);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.vertices[0].position, [0.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[1].position, [300.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[2].position, [300.0, 200.0, 0.0]);
        assert_eq!(mesh.vertices[3].position, [0.0, 200.0, 0.0]);
    }
}
