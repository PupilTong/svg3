//! `<clipPath>` definition collection and reference resolution.
//!
//! Splits out of the parent [`super`] module because it's an SVG 2 concept
//! that runs *before* the filter chain, not inside it: the document walker
//! resolves an element's `clip-path="url(#id)"` reference up front and the
//! GPU layer applies the resulting [`ClipShape`] to the filter source
//! texture ahead of the first primitive pass.
//!
//! svg3 currently supports a minimal subset — a `<clipPath>` containing a
//! single `<rect>` child resolves to a rectangular clip; everything else
//! parses without crashing but resolves to the default (no clip).

use std::collections::BTreeMap;

use crate::dom::{Document, Element, ElementKind, Node};
use crate::render::shapes::Length;
use crate::render::Viewport;

use super::filter_reference_id;

/// Resolved `<clipPath>` definitions keyed by `id`.
///
/// svg3 supports a minimal clip-path: a `<clipPath>` containing one `<rect>`
/// child is exposed as a rectangular clip. Other clip-path shapes parse
/// without crashing but resolve to the default (no clip). The clip-path
/// applies before any filter, matching SVG 2 render order.
#[derive(Debug, Default)]
pub(crate) struct ClipPathDefinitions {
    clips: BTreeMap<String, ClipShape>,
}

/// One resolved clip-path shape. svg3 currently supports a single
/// rectangular clip; arbitrary path-based clipping is a follow-up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ClipShape {
    /// Rectangular clip in user-space lengths.
    Rect {
        x: Length,
        y: Length,
        width: Length,
        height: Length,
    },
}

impl ClipPathDefinitions {
    pub(crate) fn collect(document: &Document) -> Self {
        let mut defs = Self::default();
        let mut stack = vec![document.root()];
        while let Some(id) = stack.pop() {
            let node = document.node(id);
            if node.element.kind == ElementKind::ClipPath {
                if let Some(clip_id) = node.element.attributes.get("id") {
                    if let Some(shape) = first_clip_shape(document, node) {
                        defs.clips.entry(clip_id.to_owned()).or_insert(shape);
                    }
                }
                continue;
            }
            stack.extend(node.children.iter().rev().copied());
        }
        defs
    }

    /// Resolve an element's `clip-path="url(#id)"` reference.
    pub(crate) fn resolve(&self, element: &Element) -> Option<ClipShape> {
        let raw = element.attributes.get("clip-path")?;
        let id = filter_reference_id(raw)?;
        self.clips.get(id).copied()
    }
}

fn first_clip_shape(document: &Document, node: &Node) -> Option<ClipShape> {
    for child_id in &node.children {
        let child = document.element(*child_id);
        if child.kind == ElementKind::Rect {
            let x = Length::parse(child.attributes.get("x").map_or("0", String::as_str))
                .unwrap_or(Length::Px(0.0));
            let y = Length::parse(child.attributes.get("y").map_or("0", String::as_str))
                .unwrap_or(Length::Px(0.0));
            let width = Length::parse(child.attributes.get("width").map_or("0", String::as_str))?;
            let height = Length::parse(child.attributes.get("height").map_or("0", String::as_str))?;
            return Some(ClipShape::Rect {
                x,
                y,
                width,
                height,
            });
        }
    }
    None
}

impl ClipShape {
    /// Resolve to UV-space (0..1 over the SVG viewport).
    pub(crate) fn to_uv(self, viewport: Viewport) -> [f32; 4] {
        match self {
            ClipShape::Rect {
                x,
                y,
                width,
                height,
            } => [
                x.resolve(viewport.width) / viewport.width,
                y.resolve(viewport.height) / viewport.height,
                width.resolve(viewport.width) / viewport.width,
                height.resolve(viewport.height) / viewport.height,
            ],
        }
    }
}
