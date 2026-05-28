//! `<mask>` definition collection and reference resolution.
//!
//! The renderer resolves an element's `mask="url(#id)"` reference during the
//! scene walk, renders the referenced mask definition's supported child
//! geometry into an offscreen texture, and multiplies the source graphic by
//! that texture.

use std::collections::BTreeMap;

use crate::dom::{Document, Element, ElementKind, NodeId};

use super::filter_reference_id;

/// Resolved `<mask>` definitions keyed by `id`.
#[derive(Debug, Default)]
pub(crate) struct MaskDefinitions {
    masks: BTreeMap<String, MaskDefinition>,
}

/// One referenced mask definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaskDefinition {
    /// The DOM node for the `<mask>` element.
    pub(crate) node: NodeId,
    /// How the rendered mask texture is converted into a scalar coverage.
    pub(crate) mode: MaskMode,
}

/// SVG mask interpretation. SVG masks default to luminance; `mask-type`
/// can request alpha semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaskMode {
    /// Use rendered mask alpha directly.
    Alpha,
    /// Use rendered mask luminance multiplied by alpha.
    Luminance,
}

impl MaskDefinitions {
    pub(crate) fn collect(document: &Document) -> Self {
        let mut defs = Self::default();
        let mut stack = vec![document.root()];
        while let Some(id) = stack.pop() {
            let node = document.node(id);
            if node.element.kind == ElementKind::Mask {
                if let Some(mask_id) = node.element.attributes.get("id") {
                    defs.masks
                        .entry(mask_id.to_owned())
                        .or_insert(MaskDefinition {
                            node: id,
                            mode: mask_mode(&node.element),
                        });
                }
                continue;
            }
            stack.extend(node.children.iter().rev().copied());
        }
        defs
    }

    /// Resolve an element's `mask="url(#id)"` reference.
    pub(crate) fn resolve(&self, element: &Element) -> Option<MaskDefinition> {
        let raw = element.attributes.get("mask")?;
        let id = filter_reference_id(raw)?;
        self.masks.get(id).copied()
    }
}

fn mask_mode(element: &Element) -> MaskMode {
    let value = element
        .attributes
        .get("mask-type")
        .map(|value| value.trim());
    match value {
        Some(value) if value.eq_ignore_ascii_case("alpha") => MaskMode::Alpha,
        _ => MaskMode::Luminance,
    }
}
