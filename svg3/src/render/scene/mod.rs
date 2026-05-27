//! The document walk.
//!
//! [`build_scene`] turns a parsed [`Document`] into one combined [`Mesh`] by
//! dispatching every supported SVG element to its [`crate::render::shapes`]
//! tessellator, and [`document_viewport`] resolves the root `<svg>` sizing
//! that percentage lengths resolve against.
//!
//! `<marker>` resolution — the start/mid/end definition collection and
//! per-placement instancing — lives in the [`markers`] submodule.

mod markers;

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::BTreeMap;

use crate::dom::{Document, Element, ElementKind, NodeId};

use crate::render::filters::{
    ClipPathDefinitions, FilterDefinitions, FilterInput, FilterPrimitive, FilterPrimitiveKind,
    FilterResolution,
};
use crate::render::paint::{Paint, PaintBounds, PaintDefinitions};
use crate::render::shapes;
use crate::render::transform::{parse_svg_transform, parse_transform, Mat4};
use crate::render::{Mesh, Viewport};

use markers::{append_marker_instances, MarkerDefinitions};

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
        /// Optional UV-space `[x, y, width, height]` clip applied to the
        /// filter's source texture before primitives run (SVG 2 render
        /// order: clip-path applies before filter). `None` means no clip.
        clip_uv: Option<[f32; 4]>,
    },
}

/// Per-2D-shape forward Z stride applied to push painter's order through
/// the depth test (in svg3 user units, in the `+Z`-toward-viewer
/// convention of [SPEC.md](../../SPEC.md) §3.1).
///
/// 2D content is conceptually coplanar at `z = 0`, so under
/// `depth_compare: LessEqual` consecutive 2D shapes would z-fight (the
/// perspective-correct interpolation produces tiny per-triangle NDC depth
/// drift even on the same world plane). Each 2D shape is shifted forward
/// by `(shape_index * Z_PAINTER_STRIDE)` user units instead, so painter's
/// order maps cleanly to depth order: later shapes have larger world Z,
/// smaller NDC depth, and win [`LessEqual`].
///
/// Sized so the resulting NDC depth delta is comfortably above
/// `f32`-precision noise from perspective interpolation at the default
/// camera distance (≈120 user units → `dNDC/dz ≈ 7e-6`, so a stride of
/// `0.1` gives a delta around `7e-7`, ~10× the precision floor). Still
/// well below typical 3D primitive sizes (a `<cube size="10">` extends ±5
/// user units in Z), so a 2D shape's bias does not visibly disturb its
/// position relative to nearby 3D content.
const Z_PAINTER_STRIDE: f32 = 0.1;
const DUMMY_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
// Pre-Stylo approximation: these are inherited as authored strings because
// shape resolvers still read presentation attributes directly.
const INHERITED_PRESENTATION_ATTRS: &[&str] = &[
    "color",
    "fill",
    "fill-opacity",
    "fill-rule",
    "marker",
    "marker-end",
    "marker-mid",
    "marker-start",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
];

/// Whether an element belongs to the 2D plane (`z = 0`) or the 3D
/// graphics-element set ([SPEC.md](../../SPEC.md) §5). Drives whether a
/// shape's vertices get the per-shape forward Z bias for 2D painter's
/// ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementDimension {
    /// Lies in the plane `z = 0` (all of SVG 1.1's graphics elements).
    TwoD,
    /// One of svg3's 3D graphics elements (SPEC §5) — uses its authored
    /// world Z.
    ThreeD,
}

impl ElementDimension {
    fn of(kind: &ElementKind) -> Self {
        match kind {
            ElementKind::Cube
            | ElementKind::Ellipsoid
            | ElementKind::Cylinder
            | ElementKind::Surface => Self::ThreeD,
            _ => Self::TwoD,
        }
    }
}

/// Shift each vertex of `mesh` (in the range `[start..]`) forward in svg3
/// world Z by `bias`. Used to give each 2D shape a unique forward-Z slot
/// for painter's ordering under the unified `LessEqual` depth test.
fn apply_painter_bias(mesh: &mut Mesh, start: usize, bias: f32) {
    for vertex in &mut mesh.vertices[start..] {
        vertex.position[2] += bias;
    }
}

struct SceneContext<'a> {
    viewport: Viewport,
    markers: &'a MarkerDefinitions,
    paints: &'a PaintDefinitions,
    svg3_extension_enabled: bool,
    /// Monotonic counter for the next 2D shape's painter-order Z bias
    /// slot. [`Cell`] so the immutable `&SceneContext` plumbing already
    /// established here can mutate it as we walk.
    twod_index: std::cell::Cell<u32>,
}

#[derive(Debug)]
pub(super) struct TraversalState {
    transform: Mat4,
    inherited_attrs: BTreeMap<String, String>,
}

impl Default for TraversalState {
    fn default() -> Self {
        Self {
            transform: Mat4::identity(),
            inherited_attrs: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
struct TraversalFrame {
    previous_transform: Mat4,
    restored_attrs: Vec<(String, Option<String>)>,
}

#[derive(Clone, Copy)]
struct RenderDefinitions<'a> {
    filters: &'a FilterDefinitions,
    clips: &'a ClipPathDefinitions,
}

impl TraversalState {
    fn enter_element(&mut self, element: &Element, svg3_extension_enabled: bool) -> TraversalFrame {
        let previous_transform = self.transform;
        if let Some(local) = element.attributes.get("transform").and_then(|value| {
            if svg3_extension_enabled {
                parse_transform(value)
            } else {
                parse_svg_transform(value)
            }
        }) {
            self.transform = self.transform.mul(&local);
        }

        let mut restored_attrs = Vec::new();
        if inherits_to_children(&element.kind) {
            for &name in INHERITED_PRESENTATION_ATTRS {
                if let Some(value) = element.attributes.get(name) {
                    restored_attrs.push((
                        name.to_owned(),
                        self.inherited_attrs.insert(name.to_owned(), value.clone()),
                    ));
                }
            }
        }

        TraversalFrame {
            previous_transform,
            restored_attrs,
        }
    }

    fn exit_element(&mut self, frame: TraversalFrame) {
        self.transform = frame.previous_transform;
        for (name, previous) in frame.restored_attrs.into_iter().rev() {
            if let Some(value) = previous {
                self.inherited_attrs.insert(name, value);
            } else {
                self.inherited_attrs.remove(&name);
            }
        }
    }

    fn effective_element<'a>(&self, element: &'a Element) -> Cow<'a, Element> {
        if self.inherited_attrs.is_empty() {
            return Cow::Borrowed(element);
        }

        let mut attributes = self.inherited_attrs.clone();
        attributes.extend(element.attributes.clone());
        Cow::Owned(Element {
            kind: element.kind.clone(),
            attributes,
        })
    }
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
/// `<g>` and root-level inherited presentation attributes are applied to
/// descendants, and `transform` attributes compose down the tree.
/// svg3-only 3D elements and 3D transform functions are enabled only when
/// the root `<svg>` has `extension="pupiltong"`; otherwise they are ignored
/// so the document renders as normal SVG.
pub fn build_scene(document: &Document, viewport: Viewport) -> Mesh {
    let markers = MarkerDefinitions::default();
    let paints = PaintDefinitions::default();
    let context = SceneContext {
        viewport,
        markers: &markers,
        paints: &paints,
        svg3_extension_enabled: document.svg3_extension_enabled(),
        twod_index: Cell::new(0),
    };
    let mut mesh = Mesh::default();
    let mut state = TraversalState::default();
    let root_frame = state.enter_element(
        document.element(document.root()),
        context.svg3_extension_enabled,
    );
    for child in document.node(document.root()).children.iter().copied() {
        append_subtree_mesh(document, child, &context, &mut state, true, &mut mesh);
    }
    state.exit_element(root_frame);
    mesh
}

/// Build headless render operations that preserve SVG painter's order while
/// isolating filtered subtrees into their own GPU post-process pass.
pub(crate) fn build_render_plan(document: &Document, viewport: Viewport) -> Vec<RenderOp> {
    let filters = FilterDefinitions::collect(document);
    let clips = ClipPathDefinitions::collect(document);
    let definitions = RenderDefinitions {
        filters: &filters,
        clips: &clips,
    };
    let markers = MarkerDefinitions::default();
    let paints = PaintDefinitions::default();
    let context = SceneContext {
        viewport,
        markers: &markers,
        paints: &paints,
        svg3_extension_enabled: document.svg3_extension_enabled(),
        twod_index: Cell::new(0),
    };
    let mut plan = Vec::new();
    let mut pending_mesh = Mesh::default();
    let mut state = TraversalState::default();
    let root_frame = state.enter_element(
        document.element(document.root()),
        context.svg3_extension_enabled,
    );
    for child in document.node(document.root()).children.iter().copied() {
        append_render_ops(
            document,
            child,
            definitions,
            &context,
            &mut state,
            &mut pending_mesh,
            &mut plan,
        );
    }
    state.exit_element(root_frame);
    flush_mesh(&mut pending_mesh, &mut plan);
    plan
}

fn append_render_ops(
    document: &Document,
    id: crate::dom::NodeId,
    definitions: RenderDefinitions<'_>,
    context: &SceneContext<'_>,
    state: &mut TraversalState,
    pending_mesh: &mut Mesh,
    plan: &mut Vec<RenderOp>,
) {
    let node = document.node(id);
    if is_definition_container(&node.element.kind) {
        return;
    }
    if is_svg3_3d_element(&node.element.kind) && !context.svg3_extension_enabled {
        return;
    }
    let frame = state.enter_element(&node.element, context.svg3_extension_enabled);

    // Resolve clip-path before filter (SVG 2 render order). When the
    // element references a clip-path, the filter's source texture is
    // clipped to the clip-path's UV rect before primitives run.
    // TODO: clip geometry is still resolved in viewport space; ancestor
    // transforms move the source mesh but not the clip region yet.
    let clip_uv = definitions
        .clips
        .resolve(&node.element)
        .map(|shape| shape.to_uv(context.viewport));

    match definitions.filters.resolve(&node.element) {
        Some(FilterResolution::Chain(chain)) => {
            let mut filtered_mesh = Mesh::default();
            append_entered_subtree_mesh(document, id, context, state, true, &mut filtered_mesh);
            // A primitive affects the chain output if any of:
            //  - its parameters are non-identity (`is_visible`)
            //  - its DAG wiring is non-default — e.g. `<feGaussianBlur
            //    in="SourceAlpha" stdDeviation="0"/>` is parameter-wise a
            //    no-op blur but still has to run to replace the RGB with
            //    SourceAlpha's `(0, 0, 0, src.a)`
            //  - it carries a `result` attribute that a later primitive
            //    might reference
            //  - it carries a primitive subregion (SVG 1.1 §15.5): even a
            //    parameter-wise no-op like `<feOffset dx="0" dy="0"
            //    x="20" y="20" width="10" height="10"/>` must run so the
            //    post-clip zeroes out pixels outside its rect.
            let affects_output = |primitive: &FilterPrimitive| {
                primitive.is_visible()
                    || !matches!(primitive.input, FilterInput::Default)
                    || !matches!(primitive.input2, FilterInput::Default)
                    || primitive.result.is_some()
                    || primitive.subregion.is_some()
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
                    // SVG `currentColor`: resolve the filtered element's
                    // effective `color` attribute and substitute it into any
                    // `flood-color="currentColor"` / `lighting-color="currentColor"`
                    // primitives in the chain.
                    let effective = state.effective_element(&node.element);
                    let current_color = resolve_current_color(effective.as_ref());
                    let mut owned_chain: Vec<FilterPrimitive> = chain.to_vec();
                    for primitive in owned_chain.iter_mut() {
                        primitive.substitute_current_color(current_color);
                    }
                    plan.push(RenderOp::Filter {
                        mesh: filtered_mesh,
                        primitives: owned_chain,
                        clip_uv,
                    });
                } else {
                    pending_mesh.append(filtered_mesh);
                }
            }
            state.exit_element(frame);
            return;
        }
        Some(FilterResolution::EmptyTransparent) => {
            // SVG 1.1 §15.4: an unresolved or empty filter still applies a
            // filter — the result is just transparent black. Drop the
            // element's geometry on the floor; the filter "replaces" the
            // source with nothing.
            state.exit_element(frame);
            return;
        }
        None => {}
    }

    append_element_mesh_biased(document, id, context, state, true, pending_mesh);
    if owns_children(&node.element.kind) {
        state.exit_element(frame);
        return;
    }
    for child in node.children.iter().copied() {
        append_render_ops(
            document,
            child,
            definitions,
            context,
            state,
            pending_mesh,
            plan,
        );
    }
    state.exit_element(frame);
}

/// Resolve `currentColor` from an element's `color` attribute. Without a
/// real style cascade (svg3::style is a skeleton) we look only at the
/// filtered element itself; ancestor inheritance is deferred to when the
/// cascade comes online. SVG 1.1: a missing `color` resolves to black.
fn resolve_current_color(element: &Element) -> [f32; 4] {
    element
        .attributes
        .get("color")
        .and_then(|s| crate::render::shapes::parse_color_value(s))
        .unwrap_or([0.0, 0.0, 0.0, 1.0])
}

fn append_subtree_mesh(
    document: &Document,
    id: NodeId,
    context: &SceneContext<'_>,
    state: &mut TraversalState,
    include_markers: bool,
    mesh: &mut Mesh,
) {
    let node = document.node(id);
    if is_definition_container(&node.element.kind) {
        return;
    }
    if is_svg3_3d_element(&node.element.kind) && !context.svg3_extension_enabled {
        return;
    }
    let frame = state.enter_element(&node.element, context.svg3_extension_enabled);
    append_entered_subtree_mesh(document, id, context, state, include_markers, mesh);
    state.exit_element(frame);
}

fn append_entered_subtree_mesh(
    document: &Document,
    id: NodeId,
    context: &SceneContext<'_>,
    state: &mut TraversalState,
    include_markers: bool,
    mesh: &mut Mesh,
) {
    let node = document.node(id);
    if is_svg3_3d_element(&node.element.kind) && !context.svg3_extension_enabled {
        return;
    }
    append_element_mesh_biased(document, id, context, state, include_markers, mesh);
    if owns_children(&node.element.kind) {
        return;
    }
    for child in node.children.iter().copied() {
        // TODO: Nested filters need their own render plan and offscreen pass.
        // This first filter milestone treats a filtered subtree as raw source
        // geometry for the outer filter.
        append_subtree_mesh(document, child, context, state, include_markers, mesh);
    }
}

/// Wraps [`append_element_mesh`] with the per-2D-shape painter-order Z
/// bias. Snapshots the mesh's vertex count, delegates to the tessellator,
/// then shifts the newly-appended vertices forward in Z if the element is
/// 2D — fill, stroke, and marker geometry all share the same bias slot so
/// the element composites cleanly internally. 3D elements keep their
/// authored world Z so spatial occlusion against the 2D plane and
/// other 3D content works per [SPEC.md](../../SPEC.md) §7.3.
fn append_element_mesh_biased(
    document: &Document,
    id: NodeId,
    context: &SceneContext<'_>,
    state: &TraversalState,
    include_markers: bool,
    mesh: &mut Mesh,
) {
    let source_element = &document.node(id).element;
    if !is_renderable_element(&source_element.kind) {
        return;
    }

    let start = mesh.vertices.len();
    let element = state.effective_element(source_element);
    append_element_mesh(
        document,
        id,
        element.as_ref(),
        context,
        include_markers,
        mesh,
    );
    if mesh.vertices.len() == start {
        return;
    }
    if state.transform != Mat4::identity() {
        apply_transform(mesh, start, state.transform);
    }
    if ElementDimension::of(&element.kind) == ElementDimension::TwoD {
        let index = context.twod_index.get();
        apply_painter_bias(mesh, start, (index as f32) * Z_PAINTER_STRIDE);
        context.twod_index.set(index + 1);
    }
}

fn append_element_mesh(
    document: &Document,
    id: NodeId,
    element: &Element,
    context: &SceneContext<'_>,
    include_markers: bool,
    mesh: &mut Mesh,
) {
    let viewport = context.viewport;
    let features = element_features(element, include_markers);
    match &element.kind {
        ElementKind::Rect => {
            if let Some(geo) = shapes::rect::resolve_rect(element, viewport) {
                let fill_bounds =
                    PaintBounds::new(geo.x, geo.y, geo.width, geo.height).expect("rect bounds");
                if let Some(paint) = context.paints.resolve_fill(
                    document,
                    element,
                    fill_bounds,
                    viewport,
                    features.has_opacity_attrs,
                ) {
                    append_painted_mesh(
                        shapes::rect::tessellate_rect(&geo, DUMMY_COLOR),
                        paint,
                        mesh,
                    );
                }
                if features.has_strokes {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    let stroke_mesh =
                        shapes::rect::tessellate_rect_stroke(&geo, &stroke_style, DUMMY_COLOR);
                    if let Some(paint) = context.paints.resolve_stroke(
                        document,
                        element,
                        fill_bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(stroke_mesh, paint, mesh);
                    }
                }
            }
        }
        ElementKind::Circle => {
            if let Some(geo) = shapes::circle::resolve_circle(element, viewport) {
                let fill_bounds =
                    PaintBounds::new(geo.cx - geo.r, geo.cy - geo.r, geo.r * 2.0, geo.r * 2.0)
                        .expect("circle bounds");
                if let Some(paint) = context.paints.resolve_fill(
                    document,
                    element,
                    fill_bounds,
                    viewport,
                    features.has_opacity_attrs,
                ) {
                    append_painted_mesh(
                        shapes::circle::tessellate_circle(&geo, DUMMY_COLOR),
                        paint,
                        mesh,
                    );
                }
                if features.has_strokes {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    let stroke_mesh =
                        shapes::circle::tessellate_circle_stroke(&geo, &stroke_style, DUMMY_COLOR);
                    if let Some(paint) = context.paints.resolve_stroke(
                        document,
                        element,
                        fill_bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(stroke_mesh, paint, mesh);
                    }
                }
            }
        }
        ElementKind::Ellipse => {
            if let Some(geo) = shapes::ellipse::resolve_ellipse(element, viewport) {
                let fill_bounds =
                    PaintBounds::new(geo.cx - geo.rx, geo.cy - geo.ry, geo.rx * 2.0, geo.ry * 2.0)
                        .expect("ellipse bounds");
                if let Some(paint) = context.paints.resolve_fill(
                    document,
                    element,
                    fill_bounds,
                    viewport,
                    features.has_opacity_attrs,
                ) {
                    append_painted_mesh(
                        shapes::ellipse::tessellate_ellipse(&geo, DUMMY_COLOR),
                        paint,
                        mesh,
                    );
                }
                if features.has_strokes {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    let stroke_mesh = shapes::ellipse::tessellate_ellipse_stroke(
                        &geo,
                        &stroke_style,
                        DUMMY_COLOR,
                    );
                    if let Some(paint) = context.paints.resolve_stroke(
                        document,
                        element,
                        fill_bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(stroke_mesh, paint, mesh);
                    }
                }
            }
        }
        ElementKind::Polygon => {
            if let Some(geo) = shapes::polygon::resolve_polygon(element) {
                if let Some(bounds) = PaintBounds::from_points(&geo.points) {
                    if let Some(paint) = context.paints.resolve_fill(
                        document,
                        element,
                        bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(
                            shapes::polygon::tessellate_polygon(&geo, DUMMY_COLOR),
                            paint,
                            mesh,
                        );
                    }
                }
                let has_markers = features.has_markers;
                let mut stroke_mesh = None;
                let stroke = if features.has_strokes {
                    let candidate_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    let candidate_mesh = shapes::polygon::tessellate_polygon_stroke(
                        &geo,
                        &candidate_style,
                        DUMMY_COLOR,
                    );
                    let paint = PaintBounds::from_points(&geo.points).and_then(|bounds| {
                        context.paints.resolve_stroke(
                            document,
                            element,
                            bounds,
                            viewport,
                            features.has_opacity_attrs,
                        )
                    });
                    stroke_mesh = Some((candidate_style, candidate_mesh));
                    paint
                } else {
                    None
                };
                let stroke_style = (stroke.is_some() || has_markers)
                    .then(|| shapes::stroke::resolve_stroke_style(element, viewport));
                if let Some(paint) = stroke {
                    let (_, stroke_mesh) =
                        stroke_mesh.expect("stroke mesh should exist when stroke paint exists");
                    append_painted_mesh(stroke_mesh, paint, mesh);
                }
                if has_markers {
                    append_marker_instances(
                        document,
                        context,
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
                let fill_mesh = shapes::polyline::tessellate_polyline(&geo, DUMMY_COLOR);
                if let Some(bounds) = (!fill_mesh.is_empty())
                    .then(|| PaintBounds::from_points_iter(geo.points()))
                    .flatten()
                {
                    if let Some(paint) = context.paints.resolve_fill(
                        document,
                        element,
                        bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(fill_mesh, paint, mesh);
                    }
                }
                let has_markers = features.has_markers;
                let mut stroke_mesh = None;
                let stroke = if features.has_strokes {
                    let candidate_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    let candidate_mesh = shapes::polyline::tessellate_polyline_stroke(
                        &geo,
                        &candidate_style,
                        DUMMY_COLOR,
                    );
                    let paint = PaintBounds::from_points_iter(geo.points()).and_then(|bounds| {
                        context.paints.resolve_stroke(
                            document,
                            element,
                            bounds,
                            viewport,
                            features.has_opacity_attrs,
                        )
                    });
                    stroke_mesh = Some((candidate_style, candidate_mesh));
                    paint
                } else {
                    None
                };
                let stroke_style = (stroke.is_some() || has_markers)
                    .then(|| shapes::stroke::resolve_stroke_style(element, viewport));
                if let Some(paint) = stroke {
                    let (_, stroke_mesh) =
                        stroke_mesh.expect("stroke mesh should exist when stroke paint exists");
                    append_painted_mesh(stroke_mesh, paint, mesh);
                }
                if has_markers {
                    append_marker_instances(
                        document,
                        context,
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
                    let stroke_mesh =
                        shapes::line::tessellate_segment(&geo, geo.stroke_width, DUMMY_COLOR);
                    if let Some(bounds) =
                        PaintBounds::from_points(&[(geo.x1, geo.y1), (geo.x2, geo.y2)])
                    {
                        if let Some(paint) = context
                            .paints
                            .resolve_stroke(document, element, bounds, viewport, false)
                        {
                            append_painted_mesh(stroke_mesh, paint, mesh);
                        }
                    }
                    return;
                }

                let has_markers = features.has_markers;
                let needs_general_stroke = features.has_general_line_strokes;
                let stroke_width = (!needs_general_stroke && (features.has_strokes || has_markers))
                    .then_some(geo.stroke_width);
                let stroke_style = (needs_general_stroke && (features.has_strokes || has_markers))
                    .then(|| shapes::stroke::resolve_stroke_style(element, viewport));
                if features.has_strokes {
                    let stroke_mesh = if let Some(stroke_width) = stroke_width {
                        shapes::line::tessellate_segment(&geo, stroke_width, DUMMY_COLOR)
                    } else {
                        let stroke_style = stroke_style
                            .as_ref()
                            .expect("stroke style should exist when stroke paint exists");
                        shapes::line::tessellate_line(&geo, stroke_style, DUMMY_COLOR)
                    };
                    if let Some(bounds) =
                        PaintBounds::from_points(&[(geo.x1, geo.y1), (geo.x2, geo.y2)])
                    {
                        if let Some(paint) = context.paints.resolve_stroke(
                            document,
                            element,
                            bounds,
                            viewport,
                            features.has_opacity_attrs,
                        ) {
                            append_painted_mesh(stroke_mesh, paint, mesh);
                        }
                    }
                }
                if has_markers {
                    let marker_stroke_width = stroke_width
                        .or_else(|| stroke_style.as_ref().map(|style| style.width))
                        .expect("stroke width should exist when markers exist");
                    append_marker_instances(
                        document,
                        context,
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
                let fill_mesh = shapes::path::tessellate_path_fill(&geo, DUMMY_COLOR);
                if let Some(bounds) = PaintBounds::from_mesh(&fill_mesh) {
                    if let Some(paint) = context.paints.resolve_fill(
                        document,
                        element,
                        bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(fill_mesh, paint, mesh);
                    }
                }
                if features.has_strokes {
                    let stroke_mesh = shapes::path::tessellate_path_stroke(&geo, DUMMY_COLOR);
                    if let Some(bounds) = PaintBounds::from_path(geo.path()) {
                        if let Some(paint) = context.paints.resolve_stroke(
                            document,
                            element,
                            bounds,
                            viewport,
                            features.has_opacity_attrs,
                        ) {
                            append_painted_mesh(stroke_mesh, paint, mesh);
                        }
                    }
                }
                if features.has_markers {
                    let stroke_style = shapes::stroke::resolve_stroke_style(element, viewport);
                    append_marker_instances(
                        document,
                        context,
                        element,
                        geo.path(),
                        stroke_style.width,
                        mesh,
                    );
                }
            }
        }
        ElementKind::Cube => {
            if let Some(geo) = shapes::cube::resolve_cube(element, viewport) {
                let cube_mesh = shapes::cube::tessellate_cube(&geo, DUMMY_COLOR);
                if let Some(bounds) = PaintBounds::from_mesh(&cube_mesh) {
                    if let Some(paint) = context.paints.resolve_fill(
                        document,
                        element,
                        bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(cube_mesh, paint, mesh);
                    }
                }
            }
        }
        ElementKind::Ellipsoid => {
            if let Some(geo) = shapes::ellipsoid::resolve_ellipsoid(element, viewport) {
                let ellipsoid_mesh = shapes::ellipsoid::tessellate_ellipsoid(&geo, DUMMY_COLOR);
                if let Some(bounds) = PaintBounds::from_mesh(&ellipsoid_mesh) {
                    if let Some(paint) = context.paints.resolve_fill(
                        document,
                        element,
                        bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(ellipsoid_mesh, paint, mesh);
                    }
                }
            }
        }
        ElementKind::Cylinder => {
            if let Some(geo) = shapes::cylinder::resolve_cylinder(element, viewport) {
                let cylinder_mesh = shapes::cylinder::tessellate_cylinder(&geo, DUMMY_COLOR);
                if let Some(bounds) = PaintBounds::from_mesh(&cylinder_mesh) {
                    if let Some(paint) = context.paints.resolve_fill(
                        document,
                        element,
                        bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(cylinder_mesh, paint, mesh);
                    }
                }
            }
        }
        ElementKind::Surface => {
            if let Some(geo) = shapes::surface::resolve_surface(document, id, viewport) {
                let surface_mesh = shapes::surface::tessellate_surface(&geo, DUMMY_COLOR);
                if let Some(bounds) = PaintBounds::from_mesh(&surface_mesh) {
                    if let Some(paint) = context.paints.resolve_fill(
                        document,
                        element,
                        bounds,
                        viewport,
                        features.has_opacity_attrs,
                    ) {
                        append_painted_mesh(surface_mesh, paint, mesh);
                    }
                }
            }
        }
        _ => {}
    }
}

fn apply_transform(mesh: &mut Mesh, start: usize, transform: Mat4) {
    for vertex in &mut mesh.vertices[start..] {
        vertex.position = transform.transform_point(vertex.position);
    }
}

fn append_painted_mesh(mut part: Mesh, paint: Paint, mesh: &mut Mesh) {
    paint.apply_to_mesh(&mut part);
    mesh.append(part);
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
    for (name, value) in &element.attributes {
        // Attributes are stored in a BTreeMap. Every feature flag watched here
        // sorts before `stroke-width`; update this guard when adding a watched
        // attribute that sorts later.
        if name.as_str() >= "stroke-width" {
            break;
        }
        match name.as_str() {
            "stroke" => {
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
    // `<foreignObject>` is *not* a definition — it can carry siblings — but
    // svg3 doesn't host HTML, so we treat its subtree the same way as the
    // definition containers: skip without rendering.
    matches!(
        kind,
        ElementKind::Defs
            | ElementKind::Filter
            | ElementKind::Marker
            | ElementKind::ClipPath
            | ElementKind::Mask
            | ElementKind::LinearGradient
            | ElementKind::RadialGradient
            | ElementKind::Pattern
            | ElementKind::ForeignObject
    )
}

fn inherits_to_children(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Svg | ElementKind::Group | ElementKind::Marker
    )
}

fn is_renderable_element(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Rect
            | ElementKind::Circle
            | ElementKind::Ellipse
            | ElementKind::Polygon
            | ElementKind::Polyline
            | ElementKind::Line
            | ElementKind::Path
            | ElementKind::Cube
            | ElementKind::Ellipsoid
            | ElementKind::Cylinder
            | ElementKind::Surface
    )
}

fn is_svg3_3d_element(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Cube | ElementKind::Ellipsoid | ElementKind::Cylinder | ElementKind::Surface
    )
}

/// Whether an element owns its children directly (consumes them as
/// geometric inputs rather than as nested scene content). `<surface>`
/// reads its `<path>` children to build a Bezier-patch surface; the
/// generic scene walker must not also dispatch those children as
/// standalone shapes, or they'd double-render at `z = 0`.
fn owns_children(kind: &ElementKind) -> bool {
    matches!(kind, ElementKind::Surface)
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
    use crate::render::shapes::KIND_ELLIPSE;

    /// A 100×100 viewport for `build_scene` tests, whose fixtures use
    /// absolute lengths (so the viewport value does not affect the result).
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    fn assert_xy(position: [f32; 3], expected: [f32; 2]) {
        assert!(
            (position[0] - expected[0]).abs() < 1e-6,
            "x mismatch: got {}, expected {}",
            position[0],
            expected[0]
        );
        assert!(
            (position[1] - expected[1]).abs() < 1e-6,
            "y mismatch: got {}, expected {}",
            position[1],
            expected[1]
        );
    }

    #[test]
    fn build_scene_tessellates_each_rect_in_document() {
        // Two rects: one renderable, one zero-width and skipped
        // (WPT `shapes/rect-05`).
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse("<svg><g/></svg>").unwrap();
        assert!(build_scene(&document, vp()).is_empty());
    }

    #[test]
    fn build_scene_skips_filter_definition_subtrees() {
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
    fn build_scene_skips_svg3_3d_elements_without_extension_attribute() {
        let document = crate::dom::parse(
            r##"<svg><rect width="10" height="10" fill="blue"/><cube cx="50" cy="50" size="20" fill="red"/></svg>"##,
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
    fn build_scene_skips_disabled_surface_children() {
        let document = crate::dom::parse(
            r##"<svg><surface d="M 0 L 1" fill="red"><path d="M 10 10 L 90 10"/><path d="M 10 90 L 90 90"/></surface></svg>"##,
        )
        .unwrap();

        assert!(build_scene(&document, vp()).is_empty());
    }

    #[test]
    fn build_scene_ignores_svg3_3d_transform_functions_without_extension_attribute() {
        let document = crate::dom::parse(
            r#"<svg><g transform="translate3d(10, 0, 20)"><rect width="10" height="10"/></g></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert_eq!(mesh.vertices.len(), 4);
        assert_xy(mesh.vertices[0].position, [0.0, 0.0]);
        assert_eq!(mesh.vertices[0].position[2], 0.0);
    }

    #[test]
    fn build_scene_applies_svg3_3d_transform_functions_with_extension_attribute() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong"><g transform="translate3d(10, 0, 20)"><rect width="10" height="10"/></g></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert_eq!(mesh.vertices.len(), 4);
        assert_xy(mesh.vertices[0].position, [10.0, 0.0]);
        assert_eq!(mesh.vertices[0].position[2], 20.0);
    }

    #[test]
    fn render_plan_isolates_filtered_subtree_in_painter_order() {
        let document = crate::dom::parse(
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
    fn render_plan_skips_disabled_svg3_3d_element_before_filter_resolution() {
        let document = crate::dom::parse(
            r##"<svg><filter id="paint"><feFlood flood-color="red"/></filter><cube cx="50" cy="50" size="20" filter="url(#paint)"/></svg>"##,
        )
        .unwrap();

        assert!(build_render_plan(&document, vp()).is_empty());
    }

    #[test]
    fn render_plan_isolates_fe_image_filter_in_painter_order() {
        let document = crate::dom::parse(
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
    fn build_scene_tessellates_ellipsoid() {
        // An `<ellipsoid>` is dispatched to the ellipsoid tessellator and
        // contributes its UV-parameterised surface mesh to the combined
        // mesh — KIND_SOLID triangles, vertices on the implicit surface,
        // 3D so left at its authored Z (no painter bias).
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong"><ellipsoid cx="50" cy="50" cz="0" r="20"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        assert!(!mesh.vertices.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!(mesh.vertices.iter().all(|v| v.kind == 0));
        // Every vertex sits on the implicit surface (within float precision).
        for v in &mesh.vertices {
            let nx = (v.position[0] - 50.0) / 20.0;
            let ny = (v.position[1] - 50.0) / 20.0;
            let nz = v.position[2] / 20.0;
            let r2 = nx * nx + ny * ny + nz * nz;
            assert!(
                (r2 - 1.0).abs() < 1e-4,
                "vertex {v:?} off the surface (r² = {r2})"
            );
        }
    }

    #[test]
    fn build_scene_skips_zero_radius_ellipsoid() {
        // SPEC §5.3: a zero radius on any axis disables rendering.
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong"><ellipsoid cx="50" cy="50" r="10" rz="0"/></svg>"#,
        )
        .unwrap();
        assert!(build_scene(&document, vp()).is_empty());
    }

    #[test]
    fn build_scene_tessellates_cylinder() {
        // A `<cylinder>` is dispatched to the cylinder tessellator and
        // contributes cap + side-wall triangles at its authored Z.
        let document =
            crate::dom::parse(r#"<svg extension="pupiltong"><cylinder cx="50" cy="50" cz="0" r="20" depth="30"/></svg>"#)
                .unwrap();
        let mesh = build_scene(&document, vp());
        assert!(!mesh.vertices.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!(mesh.vertices.iter().all(|v| v.kind == 0));
        let min_z = mesh
            .vertices
            .iter()
            .map(|v| v.position[2])
            .fold(f32::INFINITY, f32::min);
        let max_z = mesh
            .vertices
            .iter()
            .map(|v| v.position[2])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((min_z + 15.0).abs() < 1e-4);
        assert!((max_z - 15.0).abs() < 1e-4);
    }

    #[test]
    fn build_scene_skips_zero_depth_cylinder() {
        // SPEC §5.5: a zero depth disables rendering.
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong"><cylinder cx="50" cy="50" r="10" depth="0"/></svg>"#,
        )
        .unwrap();
        assert!(build_scene(&document, vp()).is_empty());
    }

    #[test]
    fn build_scene_tessellates_circle() {
        // A `<circle>` is dispatched to the circle tessellator and
        // contributes one SDF-covered bounding quad to the combined mesh.
        let document = crate::dom::parse(r#"<svg><circle cx="20" cy="20" r="10"/></svg>"#).unwrap();
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
            crate::dom::parse(r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/></svg>"#).unwrap();
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
            &crate::dom::parse(r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/></svg>"#).unwrap(),
            vp(),
        );
        let two = build_scene(
            &crate::dom::parse(
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
            &crate::dom::parse(
                r#"<svg><rect width="10" height="10"/><circle cx="40" cy="40" r="12"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        let full = build_scene(
            &crate::dom::parse(
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
        let document = crate::dom::parse(
            r#"<svg><g><g><ellipse cx="25" cy="35" rx="10" ry="6"/></g></g></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        // The ellipse, reached despite the `<g>` wrappers, is an SDF quad.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.vertices[0].kind, KIND_ELLIPSE);
    }

    #[test]
    fn build_scene_inherits_group_paint_for_multiple_descendants() {
        let document = crate::dom::parse(
            r##"<svg><g fill="blue"><rect width="10" height="10"/><circle cx="30" cy="30" r="6"/><rect x="50" width="10" height="10" fill="red"/></g></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        let blue = mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.color == [0.0, 0.0, 1.0, 1.0])
            .count();
        let red = mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.color == [1.0, 0.0, 0.0, 1.0])
            .count();
        assert_eq!(blue, 8, "the rect and circle should inherit group fill");
        assert_eq!(red, 4, "the explicit child fill should override the group");
    }

    #[test]
    fn build_scene_inherits_root_paint_to_direct_children() {
        let document =
            crate::dom::parse(r##"<svg fill="blue"><rect width="10" height="10"/></svg>"##)
                .unwrap();
        let mesh = build_scene(&document, vp());

        assert_eq!(mesh.vertices.len(), 4);
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn build_scene_inherits_group_stroke_for_multiple_lines() {
        let document = crate::dom::parse(
            r##"<svg><g stroke="red" stroke-width="4"><line x1="10" y1="20" x2="40" y2="20"/><line x1="10" y1="40" x2="40" y2="40"/></g></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert!(!mesh.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [1.0, 0.0, 0.0, 1.0]));
        assert!(mesh.vertices.iter().any(|vertex| vertex.position[1] < 20.0));
        assert!(mesh.vertices.iter().any(|vertex| vertex.position[1] > 40.0));
    }

    #[test]
    fn build_scene_inherits_group_fill_rule_for_paths() {
        let direct = crate::dom::parse(
            r##"<svg><path fill-rule="evenodd" d="M 10 10 H 90 V 90 H 10 Z M 30 30 H 70 V 70 H 30 Z"/></svg>"##,
        )
        .unwrap();
        let inherited = crate::dom::parse(
            r##"<svg><g fill-rule="evenodd"><path d="M 10 10 H 90 V 90 H 10 Z M 30 30 H 70 V 70 H 30 Z"/></g></svg>"##,
        )
        .unwrap();
        let nonzero = crate::dom::parse(
            r##"<svg><path d="M 10 10 H 90 V 90 H 10 Z M 30 30 H 70 V 70 H 30 Z"/></svg>"##,
        )
        .unwrap();

        let direct = build_scene(&direct, vp());
        let inherited = build_scene(&inherited, vp());
        let nonzero = build_scene(&nonzero, vp());
        assert_eq!(inherited.indices, direct.indices);
        assert_ne!(inherited.indices, nonzero.indices);
    }

    #[test]
    fn marker_children_inherit_marker_presentation_attributes() {
        let document = crate::dom::parse(
            r##"<svg><defs><marker id="arrow" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="8" refX="0" refY="0" fill="red"><path d="M0 0 L10 4 L0 8 Z"/></marker></defs><line x1="20" y1="20" x2="40" y2="20" stroke="none" marker-end="url(#arrow)"/></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert!(!mesh.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn build_scene_composes_nested_group_transforms() {
        let document = crate::dom::parse(
            r#"<svg><g transform="translate(10, 0)"><g transform="scale(2)"><rect x="5" y="6" width="4" height="3"/></g></g></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert_eq!(mesh.vertices.len(), 4);
        assert_xy(mesh.vertices[0].position, [20.0, 12.0]);
        assert_xy(mesh.vertices[1].position, [28.0, 12.0]);
        assert_xy(mesh.vertices[2].position, [28.0, 18.0]);
        assert_xy(mesh.vertices[3].position, [20.0, 18.0]);
    }

    #[test]
    fn render_plan_filters_group_source_with_inherited_paint_and_transform() {
        let document = crate::dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="1"/></filter><g fill="blue" transform="translate(10, 0)" filter="url(#soft)"><rect width="10" height="10"/></g></svg>"##,
        )
        .unwrap();
        let plan = build_render_plan(&document, vp());

        assert_eq!(plan.len(), 1);
        let RenderOp::Filter { mesh, .. } = &plan[0] else {
            panic!("expected the filtered group to be isolated");
        };
        assert_eq!(mesh.vertices.len(), 4);
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color == [0.0, 0.0, 1.0, 1.0]));
        let min_x = mesh
            .vertices
            .iter()
            .map(|vertex| vertex.position[0])
            .fold(f32::INFINITY, f32::min);
        assert_eq!(min_x, 10.0, "group transform should be applied once");
    }

    #[test]
    fn build_scene_skips_ellipse_with_fill_none() {
        // `fill="none"` resolves to no paint, so that ellipse contributes no
        // geometry — only the second, filled ellipse is tessellated.
        let document = crate::dom::parse(
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
        let document =
            crate::dom::parse(r#"<svg><polygon points="0,0 20,0 10,16"/></svg>"#).unwrap();
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
            let document = crate::dom::parse(source).unwrap();
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
    fn stroke_paint_object_bbox_uses_geometry_bounds() {
        let document = crate::dom::parse(
            r##"<svg><defs><linearGradient id="g"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><rect x="10" y="20" width="30" height="40" fill="none" stroke="url(#g)" stroke-width="10"/></svg>"##,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());

        assert_eq!(mesh.paint_servers().len(), 1);
        // The objectBoundingBox gradient resolves against the rect geometry
        // bounds, excluding the 10px stroke expansion and any tessellation
        // padding: x1/y1 at (10, 20), x2 at the right edge (40, 20).
        assert_eq!(mesh.paint_servers()[0].geometry, [10.0, 20.0, 40.0, 20.0]);
    }

    #[test]
    fn build_scene_tessellates_polyline_fill() {
        let document =
            crate::dom::parse(r#"<svg><polyline points="10,10 50,10 30,40"/></svg>"#).unwrap();
        let mesh = build_scene(&document, vp());
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
        assert_eq!(mesh.vertices[0].position, [10.0, 10.0, 0.0]);
        assert_eq!(mesh.vertices[1].position, [50.0, 10.0, 0.0]);
        assert_eq!(mesh.vertices[2].position, [30.0, 40.0, 0.0]);
    }

    #[test]
    fn build_scene_skips_polyline_without_fill_geometry() {
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
            &crate::dom::parse(
                r#"<svg><line x1="10" y1="50" x2="90" y2="50" stroke="blue" stroke-width="6"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        let dashed = build_scene(
            &crate::dom::parse(
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
    fn build_scene_tessellates_path_fill() {
        let document =
            crate::dom::parse(r#"<svg><path d="M 10 10 L 50 10 L 30 40 Z" fill="blue"/></svg>"#)
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(r#"<svg width="300" height="200"/>"#).unwrap();
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
        let document = crate::dom::parse(r#"<svg width="50%" height="25%"/>"#).unwrap();
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
        let document = crate::dom::parse(
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
