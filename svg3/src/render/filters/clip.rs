//! `<clipPath>` definition collection and reference resolution.
//!
//! Splits out of the parent [`super`] module because clip-path runs before
//! the filter chain in the renderer. The document walker resolves an
//! element's `clip-path="url(#id)"` reference up front and then renders the
//! referenced definition's supported child geometry into an alpha mask.

use std::collections::BTreeMap;

use crate::dom::{Document, Element, ElementKind, NodeId};

use super::filter_reference_id;

/// Resolved `<clipPath>` definitions keyed by `id`.
#[derive(Debug, Default)]
pub(crate) struct ClipPathDefinitions {
    clips: BTreeMap<String, NodeId>,
}

impl ClipPathDefinitions {
    pub(crate) fn collect(document: &Document) -> Self {
        let mut defs = Self::default();
        let mut stack = vec![document.root()];
        while let Some(id) = stack.pop() {
            let node = document.node(id);
            if node.element.kind == ElementKind::ClipPath {
                if let Some(clip_id) = node.element.attributes.get("id") {
                    defs.clips.entry(clip_id.to_owned()).or_insert(id);
                }
                continue;
            }
            stack.extend(node.children.iter().rev().copied());
        }
        defs
    }

    /// Resolve an element's `clip-path="url(#id)"` reference.
    pub(crate) fn resolve(&self, element: &Element) -> Option<NodeId> {
        let raw = element.attributes.get("clip-path")?;
        let id = filter_reference_id(raw)?;
        self.clips.get(id).copied()
    }
}
