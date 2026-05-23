//! The document walk.
//!
//! [`build_scene`] turns a parsed [`Document`] into one combined [`Mesh`] by
//! dispatching every supported SVG element to its [`crate::shapes`]
//! tessellator, and [`document_viewport`] resolves the root `<svg>` sizing
//! that percentage lengths resolve against.

use svg3_dom::{Document, ElementKind};

use crate::filters::{FilterDefinitions, FilterPrimitive, FilterPrimitiveKind};
use crate::shapes;
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
    let mut mesh = Mesh::default();
    for child in document.node(document.root()).children.iter().copied() {
        append_subtree_mesh(document, child, viewport, &mut mesh);
    }
    mesh
}

/// Build headless render operations that preserve SVG painter's order while
/// isolating filtered subtrees into their own GPU post-process pass.
pub(crate) fn build_render_plan(document: &Document, viewport: Viewport) -> Vec<RenderOp> {
    let filters = FilterDefinitions::collect(document);
    let mut plan = Vec::new();
    let mut pending_mesh = Mesh::default();
    for child in document.node(document.root()).children.iter().copied() {
        append_render_ops(
            document,
            child,
            viewport,
            &filters,
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
    viewport: Viewport,
    filters: &FilterDefinitions,
    pending_mesh: &mut Mesh,
    plan: &mut Vec<RenderOp>,
) {
    let node = document.node(id);
    if node.element.kind == ElementKind::Filter {
        return;
    }

    if let Some(chain) = filters.resolve(&node.element) {
        let mut filtered_mesh = Mesh::default();
        append_subtree_mesh(document, id, viewport, &mut filtered_mesh);
        let visible = chain.iter().any(FilterPrimitive::is_visible);
        let generator = chain.iter().any(|primitive| {
            matches!(
                primitive.kind,
                FilterPrimitiveKind::Flood(_) | FilterPrimitiveKind::Turbulence(_)
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

    append_element_mesh(&node.element, viewport, pending_mesh);
    for child in node.children.iter().copied() {
        append_render_ops(document, child, viewport, filters, pending_mesh, plan);
    }
}

fn append_subtree_mesh(
    document: &Document,
    id: svg3_dom::NodeId,
    viewport: Viewport,
    mesh: &mut Mesh,
) {
    let node = document.node(id);
    if node.element.kind == ElementKind::Filter {
        return;
    }
    append_element_mesh(&node.element, viewport, mesh);
    for child in node.children.iter().copied() {
        // TODO: Nested filters need their own render plan and offscreen pass.
        // This first filter milestone treats a filtered subtree as raw source
        // geometry for the outer filter.
        append_subtree_mesh(document, child, viewport, mesh);
    }
}

fn append_element_mesh(element: &svg3_dom::Element, viewport: Viewport, mesh: &mut Mesh) {
    match &element.kind {
        ElementKind::Rect => {
            if let (Some(geo), Some(color)) = (
                shapes::rect::resolve_rect(element, viewport),
                shapes::resolve_fill(element),
            ) {
                mesh.append(shapes::rect::tessellate_rect(&geo, color));
            }
        }
        ElementKind::Circle => {
            if let (Some(geo), Some(color)) = (
                shapes::circle::resolve_circle(element, viewport),
                shapes::resolve_fill(element),
            ) {
                mesh.append(shapes::circle::tessellate_circle(&geo, color));
            }
        }
        ElementKind::Ellipse => {
            if let (Some(geo), Some(color)) = (
                shapes::ellipse::resolve_ellipse(element, viewport),
                shapes::resolve_fill(element),
            ) {
                mesh.append(shapes::ellipse::tessellate_ellipse(&geo, color));
            }
        }
        ElementKind::Polygon => {
            if let (Some(geo), Some(color)) = (
                shapes::polygon::resolve_polygon(element),
                shapes::resolve_fill(element),
            ) {
                mesh.append(shapes::polygon::tessellate_polygon(&geo, color));
            }
        }
        ElementKind::Polyline => {
            if let (Some(geo), Some(color)) = (
                shapes::polyline::resolve_polyline(element),
                shapes::resolve_fill(element),
            ) {
                mesh.append(shapes::polyline::tessellate_polyline(&geo, color));
            }
        }
        ElementKind::Line => {
            if let (Some(geo), Some(color)) = (
                shapes::line::resolve_line(element, viewport),
                shapes::resolve_stroke(element),
            ) {
                mesh.append(shapes::line::tessellate_line(&geo, color));
            }
        }
        ElementKind::Path => {
            if let Some(geo) = shapes::path::resolve_path(element, viewport) {
                if let Some(color) = shapes::resolve_fill(element) {
                    mesh.append(shapes::path::tessellate_path_fill(&geo, color));
                }
                if let Some(color) = shapes::resolve_stroke(element) {
                    mesh.append(shapes::path::tessellate_path_stroke(&geo, color));
                }
            }
        }
        _ => {}
    }
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
    use crate::shapes::{KIND_ELLIPSE, KIND_SEGMENT};

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
        // Exactly the one stroked line, as a four-vertex SDF box quad whose
        // `params` carries the box half-length then the stroke half-width.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.vertices[0].kind, KIND_SEGMENT);
        assert_eq!(mesh.vertices[0].params, [20.0, 2.0, 0.0, 0.0]);
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
