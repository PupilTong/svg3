//! SVG paint-server resolution for shape fills and strokes.
//!
//! The renderer still resolves presentation attributes directly, without the
//! future Stylo cascade. This module covers the SVG paint values that need
//! document-level definitions: `<linearGradient>`, `<radialGradient>`,
//! `<pattern>`, and their `<stop>` children.
//!
//! Current scope intentionally omits `gradientTransform` / `patternTransform`,
//! `href` inheritance, non-pad spread methods, and nested paint definitions
//! inside another paint server's subtree.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::dom::{Document, Element, ElementKind, NodeId};
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::{Path, PathEvent};

use crate::render::mesh::{
    PaintServer, MAX_GRADIENT_STOPS, MAX_PATTERN_ITEMS, PAINT_LINEAR_GRADIENT, PAINT_PATTERN,
    PAINT_RADIAL_GRADIENT, PAINT_SVG_TEXTURE, TEXTURE_MAP_CUBE_CROSS, TEXTURE_MAP_IDENTITY,
};
use crate::render::shapes::cube::{
    resolve_cube_map, CubeMap, CUBE_CROSS_SLOTS, CUBE_FACE_COUNT, CUBE_VERTS_PER_FACE,
};
use crate::render::shapes::{self, rect, stroke::FLATTENING_TOLERANCE, Length, Viewport};
use crate::render::Mesh;

const DEFAULT_FILL: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[derive(Debug, Clone, Copy)]
pub(crate) struct PaintBounds {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl PaintBounds {
    pub(crate) fn new(x: f32, y: f32, width: f32, height: f32) -> Option<Self> {
        (width >= 0.0 && height >= 0.0 && (width > 0.0 || height > 0.0)).then_some(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub(crate) fn from_points(points: &[(f32, f32)]) -> Option<Self> {
        Self::from_points_iter(points.iter().copied())
    }

    pub(crate) fn from_points_iter(points: impl IntoIterator<Item = (f32, f32)>) -> Option<Self> {
        let mut iter = points.into_iter();
        let (first_x, first_y) = iter.next()?;
        let (mut min_x, mut max_x) = (first_x, first_x);
        let (mut min_y, mut max_y) = (first_y, first_y);
        for (x, y) in iter {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        Self::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    pub(crate) fn from_path(path: &Path) -> Option<Self> {
        let mut points = Vec::new();
        for event in path.iter().flattened(FLATTENING_TOLERANCE) {
            match event {
                PathEvent::Begin { at } => points.push((at.x, at.y)),
                PathEvent::Line { from, to } => {
                    points.push((from.x, from.y));
                    points.push((to.x, to.y));
                }
                PathEvent::End { first, last, .. } => {
                    points.push((first.x, first.y));
                    points.push((last.x, last.y));
                }
                PathEvent::Quadratic { .. } | PathEvent::Cubic { .. } => unreachable!(),
            }
        }
        Self::from_points_iter(points)
    }

    pub(crate) fn from_mesh(mesh: &Mesh) -> Option<Self> {
        let mut iter = mesh.vertices.iter();
        let first = iter.next()?;
        let (mut min_x, mut max_x) = (first.position[0], first.position[0]);
        let (mut min_y, mut max_y) = (first.position[1], first.position[1]);
        for vertex in iter {
            let [x, y, _] = vertex.position;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        Self::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    fn origin(self, axis: Axis) -> f32 {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Other => 0.0,
        }
    }

    fn extent(self, axis: Axis) -> f32 {
        match axis {
            Axis::X => self.width,
            Axis::Y => self.height,
            Axis::Other => self.width.min(self.height),
        }
    }
}

/// How a [`Paint::SvgTexture`] paints its surface. Encoded into the
/// `mapping_kind` field of the paint server's `meta` uniform; `CubeCross`
/// also drives a mesh-level UV remap because the per-face slot is baked
/// into the vertex UV at apply time rather than computed in the shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextureMapping {
    /// Sample the vertex UV as-is — used by every 3D primitive except a
    /// `<cube cube-map="cross">`.
    Identity,
    /// Cube `<cube cube-map="cross">`: each face's `(s, t) ∈ [0, 1]²` is
    /// remapped into one of the six 4×3 horizontal-cross atlas slots.
    /// Requires the mesh to be a `tessellate_cube` output: 24 vertices, 4
    /// per face, in the [`CUBE_FACE_*`](crate::render::shapes::cube)
    /// declaration order.
    CubeCross,
}

impl TextureMapping {
    fn kind(self) -> u32 {
        match self {
            Self::Identity => TEXTURE_MAP_IDENTITY,
            Self::CubeCross => TEXTURE_MAP_CUBE_CROSS,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Paint {
    Solid([f32; 4]),
    Server(Box<PaintServer>),
    /// `<svg>` placed in `<defs>` (or anywhere reachable from the document
    /// root other than the root itself) and referenced as `fill="url(#id)"`
    /// on a 3D element. The renderer rasterizes the subtree once into a
    /// texture-array layer and the fragment shader samples the layer at the
    /// vertex UV. `mapping` selects an optional per-primitive remap.
    SvgTexture {
        node_id: NodeId,
        mapping: TextureMapping,
    },
}

impl Paint {
    pub(crate) fn apply_to_mesh(self, mesh: &mut Mesh) {
        if mesh.is_empty() {
            return;
        }
        match self {
            Self::Solid(color) => {
                for vertex in &mut mesh.vertices {
                    vertex.color = color;
                    vertex.paint_id = 0;
                }
            }
            Self::Server(server) => {
                let paint_id = mesh.push_paint_server(*server);
                for vertex in &mut mesh.vertices {
                    vertex.paint_id = paint_id;
                }
            }
            Self::SvgTexture { node_id, mapping } => {
                // `meta[1]` carries the defs node's arena index as a
                // placeholder. The renderer rewrites it to the actual
                // texture-array layer index after collecting and rasterizing
                // the referenced subtrees.
                let mut server = PaintServer::zeroed();
                server.meta = [PAINT_SVG_TEXTURE, node_id.index() as u32, mapping.kind(), 0];
                let paint_id = mesh.push_paint_server(server);
                for vertex in &mut mesh.vertices {
                    vertex.paint_id = paint_id;
                }
                if matches!(mapping, TextureMapping::CubeCross) {
                    apply_cube_cross_uvs(mesh);
                }
            }
        }
    }
}

/// Rewrite the cube's per-face UVs from face-local `[0, 1]²` into the 4×3
/// horizontal-cross atlas. Assumes the mesh is the unmodified output of
/// [`crate::render::shapes::cube::tessellate_cube`] — i.e. 24 vertices, 4
/// per face, in the `CUBE_FACE_*` declaration order. A non-cube mesh that
/// happens to be 24 vertices long would be misinterpreted, but the paint
/// resolver only emits `CubeCross` when the consumer element is `<cube>`,
/// so that mis-pairing can't actually happen at runtime.
fn apply_cube_cross_uvs(mesh: &mut Mesh) {
    let expected = CUBE_FACE_COUNT * CUBE_VERTS_PER_FACE as usize;
    if mesh.vertices.len() != expected {
        return;
    }
    for (face, &(slot_x, slot_y)) in CUBE_CROSS_SLOTS.iter().enumerate() {
        let base = face * CUBE_VERTS_PER_FACE as usize;
        for vertex in &mut mesh.vertices[base..base + CUBE_VERTS_PER_FACE as usize] {
            let s = vertex.uv[0];
            let t = vertex.uv[1];
            vertex.uv = [(slot_x as f32 + s) / 4.0, (slot_y as f32 + t) / 3.0];
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct PaintDefinitions {
    definitions: OnceLock<BTreeMap<String, PaintDefinition>>,
}

impl PaintDefinitions {
    pub(crate) fn resolve_fill(
        &self,
        document: &Document,
        element: &Element,
        bounds: PaintBounds,
        viewport: Viewport,
        apply_opacity: bool,
    ) -> Option<Paint> {
        let opacity = if apply_opacity {
            shapes::resolve_opacity(element, "fill-opacity")
                * shapes::resolve_opacity(element, "opacity")
        } else {
            1.0
        };
        match element.attributes.get("fill").map(String::as_str) {
            Some(value) => self.resolve_paint_value(
                document,
                element,
                value,
                bounds,
                viewport,
                opacity,
                Some(DEFAULT_FILL),
            ),
            None => Some(Paint::Solid(scale_alpha(DEFAULT_FILL, opacity))),
        }
    }

    pub(crate) fn resolve_stroke(
        &self,
        document: &Document,
        element: &Element,
        bounds: PaintBounds,
        viewport: Viewport,
        apply_opacity: bool,
    ) -> Option<Paint> {
        let value = element.attributes.get("stroke")?.trim();
        let opacity = if apply_opacity {
            shapes::resolve_opacity(element, "stroke-opacity")
                * shapes::resolve_opacity(element, "opacity")
        } else {
            1.0
        };
        self.resolve_paint_value(document, element, value, bounds, viewport, opacity, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_paint_value(
        &self,
        document: &Document,
        consumer: &Element,
        value: &str,
        bounds: PaintBounds,
        viewport: Viewport,
        opacity: f32,
        invalid_fallback: Option<[f32; 4]>,
    ) -> Option<Paint> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("none") {
            return None;
        }

        if let Some(reference) = PaintReference::parse(value) {
            if let Some(definition) = self.definitions(document).get(reference.id) {
                if let Some(paint) = self.resolve_definition(
                    document,
                    *definition,
                    consumer,
                    bounds,
                    viewport,
                    opacity,
                ) {
                    return Some(paint);
                }
            }
            if reference
                .fallback
                .is_some_and(|fallback| fallback.eq_ignore_ascii_case("none"))
            {
                return None;
            }
            if let Some(fallback) = reference
                .fallback
                .and_then(shapes::parse_color_value)
                .or(invalid_fallback)
            {
                return Some(Paint::Solid(scale_alpha(fallback, opacity)));
            }
            return None;
        }

        shapes::parse_color_value(value)
            .map(|color| Paint::Solid(scale_alpha(color, opacity)))
            .or_else(|| invalid_fallback.map(|color| Paint::Solid(scale_alpha(color, opacity))))
    }

    fn resolve_definition(
        &self,
        document: &Document,
        definition: PaintDefinition,
        consumer: &Element,
        bounds: PaintBounds,
        viewport: Viewport,
        opacity: f32,
    ) -> Option<Paint> {
        let node = document.node(definition.node);
        match node.element.kind {
            ElementKind::LinearGradient => Some(Paint::Server(Box::new(resolve_linear_gradient(
                document,
                definition.node,
                bounds,
                viewport,
                opacity,
            )))),
            ElementKind::RadialGradient => Some(Paint::Server(Box::new(resolve_radial_gradient(
                document,
                definition.node,
                bounds,
                viewport,
                opacity,
            )))),
            ElementKind::Pattern => {
                resolve_pattern(document, definition.node, bounds, viewport, opacity)
                    .map(|server| Paint::Server(Box::new(server)))
            }
            ElementKind::Svg => resolve_svg_texture(definition.node, consumer),
            _ => None,
        }
    }

    fn definitions(&self, document: &Document) -> &BTreeMap<String, PaintDefinition> {
        self.definitions
            .get_or_init(|| collect_paint_definitions(document))
    }
}

#[derive(Debug, Clone, Copy)]
struct PaintDefinition {
    node: NodeId,
}

fn collect_paint_definitions(document: &Document) -> BTreeMap<String, PaintDefinition> {
    let mut definitions = BTreeMap::new();
    let mut stack = vec![document.root()];
    while let Some(id) = stack.pop() {
        let node = document.node(id);
        if matches!(
            node.element.kind,
            ElementKind::LinearGradient | ElementKind::RadialGradient | ElementKind::Pattern
        ) {
            if let Some(paint_id) = node.element.attributes.get("id") {
                definitions
                    .entry(paint_id.to_owned())
                    .or_insert(PaintDefinition { node: id });
            }
            continue;
        }
        // A nested `<svg id="…">` (anywhere other than the document root) is
        // a texture paint server when referenced via `fill="url(#id)"` on a
        // 3D primitive. We register it here even in normal 2D mode — there
        // it's just an unused definition, consistent with an unused
        // `<linearGradient>`. The root `<svg>` itself is never a definition.
        if matches!(node.element.kind, ElementKind::Svg) && id != document.root() {
            if let Some(paint_id) = node.element.attributes.get("id") {
                definitions
                    .entry(paint_id.to_owned())
                    .or_insert(PaintDefinition { node: id });
            }
            // Don't descend further — the subtree becomes the texture's own
            // content, not a sibling paint-server scope.
            continue;
        }
        stack.extend(node.children.iter().rev().copied());
    }
    definitions
}

/// Build a [`Paint::SvgTexture`] for a `<svg>` paint definition referenced
/// by `consumer`. Reads the consumer's `cube-map` attribute to pick the
/// per-primitive mapping; non-cube 3D consumers always get
/// [`TextureMapping::Identity`].
///
/// Returns `None` for non-3D consumers — `<svg>`-as-texture is restricted
/// to 3D primitives in MVP, matching the texture-on-2D scope note in the
/// plan. The caller then falls through to the SVG paint fallback (the
/// invalid-fallback colour or `none`).
fn resolve_svg_texture(node_id: NodeId, consumer: &Element) -> Option<Paint> {
    let mapping = match consumer.kind {
        ElementKind::Cube => match resolve_cube_map(consumer) {
            CubeMap::Same => TextureMapping::Identity,
            CubeMap::Cross => TextureMapping::CubeCross,
        },
        ElementKind::Ellipsoid | ElementKind::Cylinder | ElementKind::Surface => {
            TextureMapping::Identity
        }
        _ => return None,
    };
    Some(Paint::SvgTexture { node_id, mapping })
}

#[derive(Debug, Clone, Copy)]
struct PaintReference<'a> {
    id: &'a str,
    fallback: Option<&'a str>,
}

impl<'a> PaintReference<'a> {
    fn parse(value: &'a str) -> Option<Self> {
        let value = value.trim();
        let rest = value.strip_prefix("url(")?;
        let close = rest.find(')')?;
        let inner = rest[..close].trim();
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
        if id.is_empty() {
            return None;
        }
        let fallback = rest[close + 1..].trim();
        Some(Self {
            id,
            fallback: (!fallback.is_empty()).then_some(fallback),
        })
    }
}

fn resolve_linear_gradient(
    document: &Document,
    node_id: NodeId,
    bounds: PaintBounds,
    viewport: Viewport,
    opacity: f32,
) -> PaintServer {
    let element = &document.node(node_id).element;
    let units = GradientUnits::resolve(element);
    let x1 = resolve_gradient_position(element, "x1", "0%", units, bounds, viewport, Axis::X);
    let y1 = resolve_gradient_position(element, "y1", "0%", units, bounds, viewport, Axis::Y);
    let x2 = resolve_gradient_position(element, "x2", "100%", units, bounds, viewport, Axis::X);
    let y2 = resolve_gradient_position(element, "y2", "0%", units, bounds, viewport, Axis::Y);
    gradient_server(
        PAINT_LINEAR_GRADIENT,
        [x1, y1, x2, y2],
        collect_stops(document, node_id, opacity),
    )
}

fn resolve_radial_gradient(
    document: &Document,
    node_id: NodeId,
    bounds: PaintBounds,
    viewport: Viewport,
    opacity: f32,
) -> PaintServer {
    let element = &document.node(node_id).element;
    let units = GradientUnits::resolve(element);
    let cx = resolve_gradient_position(element, "cx", "50%", units, bounds, viewport, Axis::X);
    let cy = resolve_gradient_position(element, "cy", "50%", units, bounds, viewport, Axis::Y);
    let r = resolve_gradient_radius(element, "r", "50%", units, bounds, viewport);
    gradient_server(
        PAINT_RADIAL_GRADIENT,
        [cx, cy, r.max(0.0), 0.0],
        collect_stops(document, node_id, opacity),
    )
}

fn gradient_server(kind: u32, geometry: [f32; 4], stops: Vec<GradientStop>) -> PaintServer {
    let mut server = PaintServer::zeroed();
    server.meta = [kind, stops.len().min(MAX_GRADIENT_STOPS) as u32, 0, 0];
    server.geometry = geometry;
    for (index, stop) in stops.into_iter().take(MAX_GRADIENT_STOPS).enumerate() {
        server.colors[index] = stop.color;
        server.offsets[index / 4][index % 4] = stop.offset;
    }
    server
}

fn resolve_pattern(
    document: &Document,
    node_id: NodeId,
    bounds: PaintBounds,
    viewport: Viewport,
    opacity: f32,
) -> Option<PaintServer> {
    let node = document.node(node_id);
    let units = PatternUnits::resolve(&node.element);
    let x = resolve_pattern_position(&node.element, "x", "0", units, bounds, viewport, Axis::X);
    let y = resolve_pattern_position(&node.element, "y", "0", units, bounds, viewport, Axis::Y);
    let width = resolve_pattern_size(&node.element, "width", units, bounds, viewport, Axis::X)?;
    let height = resolve_pattern_size(&node.element, "height", units, bounds, viewport, Axis::Y)?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    let mut server = PaintServer::zeroed();
    server.meta[0] = PAINT_PATTERN;
    server.geometry = [x, y, width, height];

    let pattern_viewport = Viewport { width, height };
    let mut item_count = 0;
    for child in node.children.iter().copied() {
        if item_count == MAX_PATTERN_ITEMS {
            break;
        }
        let child = document.node(child);
        if child.element.kind != ElementKind::Rect {
            continue;
        }
        let Some(geo) = rect::resolve_rect(&child.element, pattern_viewport) else {
            continue;
        };
        let Some(mut color) = shapes::resolve_fill_with_opacity(&child.element, true) else {
            continue;
        };
        color[3] *= opacity;
        server.pattern_rects[item_count] = [geo.x, geo.y, geo.width, geo.height];
        server.pattern_colors[item_count] = color;
        item_count += 1;
    }
    server.meta[2] = item_count as u32;
    Some(server)
}

#[derive(Debug, Clone, Copy)]
struct GradientStop {
    offset: f32,
    color: [f32; 4],
}

fn collect_stops(document: &Document, node_id: NodeId, opacity: f32) -> Vec<GradientStop> {
    let mut stops = Vec::new();
    let mut previous = 0.0;
    for child in document.node(node_id).children.iter().copied() {
        let child = document.node(child);
        if child.element.kind != ElementKind::Stop {
            continue;
        }
        let mut offset = presentation_or_style(&child.element, "offset")
            .as_deref()
            .and_then(parse_stop_offset)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        offset = offset.max(previous);
        previous = offset;

        let mut color = presentation_or_style(&child.element, "stop-color")
            .as_deref()
            .and_then(shapes::parse_color_value)
            .unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let stop_opacity = presentation_or_style(&child.element, "stop-opacity")
            .as_deref()
            .and_then(shapes::parse_opacity)
            .unwrap_or(1.0);
        color[3] *= stop_opacity * opacity;
        stops.push(GradientStop { offset, color });
    }
    stops
}

fn presentation_or_style<'a>(element: &'a Element, name: &str) -> Option<Cow<'a, str>> {
    element
        .attributes
        .get(name)
        .map(|value| Cow::Borrowed(value.as_str()))
        .or_else(|| style_property(element, name).map(Cow::Borrowed))
}

fn style_property<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element.attributes.get("style").and_then(|style| {
        style.split(';').find_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            (property.trim() == name).then(|| value.trim())
        })
    })
}

fn parse_stop_offset(value: &str) -> Option<f32> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent.trim().parse::<f32>().ok().map(|v| v / 100.0);
    }
    value.parse::<f32>().ok().filter(|v| v.is_finite())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GradientUnits {
    ObjectBoundingBox,
    UserSpaceOnUse,
}

impl GradientUnits {
    fn resolve(element: &Element) -> Self {
        match element
            .attributes
            .get("gradientUnits")
            .map(|value| value.trim())
        {
            Some("userSpaceOnUse") => Self::UserSpaceOnUse,
            _ => Self::ObjectBoundingBox,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternUnits {
    ObjectBoundingBox,
    UserSpaceOnUse,
}

impl PatternUnits {
    fn resolve(element: &Element) -> Self {
        match element
            .attributes
            .get("patternUnits")
            .map(|value| value.trim())
        {
            Some("userSpaceOnUse") => Self::UserSpaceOnUse,
            _ => Self::ObjectBoundingBox,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Axis {
    X,
    Y,
    Other,
}

fn resolve_gradient_position(
    element: &Element,
    name: &str,
    default: &str,
    units: GradientUnits,
    bounds: PaintBounds,
    viewport: Viewport,
    axis: Axis,
) -> f32 {
    let value = element
        .attributes
        .get(name)
        .map(String::as_str)
        .unwrap_or(default);
    match units {
        GradientUnits::ObjectBoundingBox => {
            bounds.origin(axis) + resolve_bbox_length(value, bounds.extent(axis)).unwrap_or(0.0)
        }
        GradientUnits::UserSpaceOnUse => resolve_user_length(value, viewport, axis).unwrap_or(0.0),
    }
}

fn resolve_gradient_radius(
    element: &Element,
    name: &str,
    default: &str,
    units: GradientUnits,
    bounds: PaintBounds,
    viewport: Viewport,
) -> f32 {
    let value = element
        .attributes
        .get(name)
        .map(String::as_str)
        .unwrap_or(default);
    match units {
        GradientUnits::ObjectBoundingBox => {
            resolve_bbox_length(value, bounds.extent(Axis::Other)).unwrap_or(0.0)
        }
        GradientUnits::UserSpaceOnUse => {
            resolve_user_length(value, viewport, Axis::Other).unwrap_or(0.0)
        }
    }
}

fn resolve_pattern_position(
    element: &Element,
    name: &str,
    default: &str,
    units: PatternUnits,
    bounds: PaintBounds,
    viewport: Viewport,
    axis: Axis,
) -> f32 {
    let value = element
        .attributes
        .get(name)
        .map(String::as_str)
        .unwrap_or(default);
    match units {
        PatternUnits::ObjectBoundingBox => {
            bounds.origin(axis) + resolve_bbox_length(value, bounds.extent(axis)).unwrap_or(0.0)
        }
        PatternUnits::UserSpaceOnUse => resolve_user_length(value, viewport, axis).unwrap_or(0.0),
    }
}

fn resolve_pattern_size(
    element: &Element,
    name: &str,
    units: PatternUnits,
    bounds: PaintBounds,
    viewport: Viewport,
    axis: Axis,
) -> Option<f32> {
    let value = element.attributes.get(name)?.as_str();
    match units {
        PatternUnits::ObjectBoundingBox => resolve_bbox_length(value, bounds.extent(axis)),
        PatternUnits::UserSpaceOnUse => resolve_user_length(value, viewport, axis),
    }
}

fn resolve_bbox_length(value: &str, basis: f32) -> Option<f32> {
    match Length::parse(value)? {
        Length::Px(value) => Some(value * basis),
        Length::Percent(percent) => Some(percent / 100.0 * basis),
    }
}

fn resolve_user_length(value: &str, viewport: Viewport, axis: Axis) -> Option<f32> {
    let basis = match axis {
        Axis::X => viewport.width,
        Axis::Y => viewport.height,
        Axis::Other => viewport.diagonal(),
    };
    Length::parse(value).map(|length| length.resolve(basis))
}

fn scale_alpha(mut color: [f32; 4], opacity: f32) -> [f32; 4] {
    color[3] *= opacity;
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_reference_parses_url_and_fallback() {
        let reference = PaintReference::parse("url('#grad') red").unwrap();
        assert_eq!(reference.id, "grad");
        assert_eq!(reference.fallback, Some("red"));
    }

    #[test]
    fn gradient_stops_read_attributes_and_style() {
        let document = crate::dom::parse(
            r##"<svg><linearGradient id="g"><stop offset="0" stop-color="red"/><stop style="offset: 100%; stop-color: blue; stop-opacity: 50%"/></linearGradient></svg>"##,
        )
        .unwrap();
        let defs = PaintDefinitions::default();
        let paint = defs
            .resolve_fill(
                &document,
                &element(&[("fill", "url(#g)")]),
                PaintBounds::new(0.0, 0.0, 10.0, 10.0).unwrap(),
                Viewport {
                    width: 10.0,
                    height: 10.0,
                },
                true,
            )
            .unwrap();
        let Paint::Server(server) = paint else {
            panic!("expected gradient server");
        };
        assert_eq!(server.meta[0], PAINT_LINEAR_GRADIENT);
        assert_eq!(server.meta[1], 2);
        assert_eq!(server.offsets[0][0], 0.0);
        assert_eq!(server.offsets[0][1], 1.0);
        assert_eq!(server.colors[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(server.colors[1], [0.0, 0.0, 1.0, 0.5]);
    }

    fn element(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Rect);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }
}
