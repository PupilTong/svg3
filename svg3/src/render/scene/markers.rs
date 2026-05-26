//! `<marker>` resolution and instancing for the document walk.
//!
//! Splits cleanly out of the parent [`super`] module because it owns its
//! own type cluster — [`MarkerDefinitions`] (the lazy id → definition map),
//! the per-definition [`MarkerDefinition`] with its `marker-units` /
//! `orient` / `viewBox` decoders, [`MarkerRefs`] (the resolved
//! start/mid/end triple for one element), and the [`append_marker_instances`]
//! entry the document walker calls on every stroked path.
//!
//! Marker definitions are collected lazily the first time an element
//! references one; documents with no markers therefore don't pay the
//! definition walk. Definitions own their `NodeId`, so instancing
//! re-enters the document walker with marker expansion disabled — which
//! prevents authored marker references inside a marker from recursively
//! instancing other marker definitions.
//!
//! The `placement → (x, y, angle)` computation comes from
//! [`crate::render::shapes::stroke::marker_placements`]; this module just
//! transforms the marker's tessellated mesh into the placement frame and
//! appends it to the parent mesh.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::dom::{Document, Element, ElementKind, NodeId};
use crate::render::paint::PaintDefinitions;
use crate::render::shapes::{self, stroke::MarkerKind};
use crate::render::{Mesh, Viewport};

use super::{append_subtree_mesh, url_reference_id, SceneContext};

#[derive(Debug, Default)]
pub(super) struct MarkerDefinitions {
    // Collected on the first actual marker reference. No-marker documents are
    // common and should not pay a separate definition walk.
    markers: OnceLock<BTreeMap<String, MarkerDefinition>>,
}

impl MarkerDefinitions {
    pub(super) fn marker_refs(&self, document: &Document, element: &Element) -> MarkerRefs {
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
pub(super) struct MarkerDefinition {
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
pub(super) struct MarkerRefs {
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

    pub(super) fn is_empty(&self) -> bool {
        self.start.is_none() && self.mid.is_none() && self.end.is_none()
    }
}

pub(super) fn append_marker_instances(
    document: &Document,
    markers: &MarkerDefinitions,
    paints: &PaintDefinitions,
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
        // Markers render with a fresh painter-bias counter so the marker's
        // internal 2D content layers within itself, independent of the
        // surrounding scene's bias slot the marker reference consumed.
        let marker_context = SceneContext {
            viewport: marker_viewport,
            markers,
            paints,
            twod_index: std::cell::Cell::new(0),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
