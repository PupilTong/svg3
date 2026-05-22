//! `svg3-style` — computed-style resolution for SVG3 documents.
//!
//! Styling follows the web platform: element styles are resolved with
//! [Stylo](https://crates.io/crates/stylo), Servo's CSS engine.
//!
//! Status: in progress. [`collect_css`] gathers the CSS from a document's
//! `<style>` elements — the input the cascade parses. The Stylo
//! selector-matching cascade (implementing Stylo's `TElement` over
//! [`svg3_dom`]) is being built incrementally on top of it;
//! [`StyleEngine::resolve`] is not wired to it yet.

use svg3_dom::{Document, ElementKind};
use thiserror::Error;

/// A resolved style for a single element.
///
/// Placeholder shape — real computed values (colour, opacity, transforms,
/// material properties) come from the Stylo cascade once it is wired.
#[derive(Debug, Clone, Copy)]
pub struct ComputedStyle {
    /// sRGB base colour, RGBA in `[0, 1]`.
    pub color: [f32; 4],
    /// Element opacity in `[0, 1]`.
    pub opacity: f32,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            color: [1.0, 1.0, 1.0, 1.0],
            opacity: 1.0,
        }
    }
}

/// Collect the CSS source from every `<style>` element in `document`.
///
/// `<style>` elements are visited in document order (a pre-order walk) and
/// their captured text joined — separated by newlines — into a single
/// stylesheet source, the input the cascade parses. svg3 captures `<style>`
/// text content while parsing the document (see [`svg3_dom`]); text on any
/// other element is not part of the model and is not collected.
///
/// Returns an empty string when the document has no `<style>` element.
pub fn collect_css(document: &Document) -> String {
    let mut css = String::new();
    // Pre-order DFS; children pushed in reverse so they pop in document
    // order, so stacked `<style>` rules cascade in source order.
    let mut stack = vec![document.root()];
    while let Some(id) = stack.pop() {
        let node = document.node(id);
        if node.element.kind == ElementKind::Style {
            if !css.is_empty() {
                css.push('\n');
            }
            css.push_str(&node.element.text);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    css
}

/// Owns the Stylo context and resolves styles for a [`Document`].
///
/// Status: scaffolding — holds no Stylo state yet.
#[derive(Debug, Default, Clone, Copy)]
pub struct StyleEngine;

impl StyleEngine {
    /// Create a new style engine.
    pub fn new() -> Self {
        Self
    }

    /// Resolve computed styles for every element in `document`.
    ///
    /// Not implemented yet — the Stylo `TElement` cascade is still being
    /// built. Returns [`StyleError::NotImplemented`].
    pub fn resolve(&self, _document: &Document) -> Result<(), StyleError> {
        log::debug!("svg3-style: resolve() called on the scaffold (not implemented)");
        Err(StyleError::NotImplemented)
    }
}

/// Errors that can occur during style resolution.
#[derive(Debug, Error)]
pub enum StyleError {
    /// The Stylo cascade is not implemented yet (scaffolding).
    #[error("style resolution is not implemented yet")]
    NotImplemented,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_is_opaque_and_in_range() {
        let s = ComputedStyle::default();
        assert!(s.opacity.is_finite() && (0.0..=1.0).contains(&s.opacity));
        assert!(s.color.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn resolve_is_not_implemented_yet() {
        let engine = StyleEngine::new();
        let doc = Document::new();
        assert!(matches!(
            engine.resolve(&doc),
            Err(StyleError::NotImplemented)
        ));
    }

    #[test]
    fn collect_css_gathers_style_element_text() {
        let doc = svg3_dom::parse(r#"<svg><style>rect { fill: red; }</style></svg>"#).unwrap();
        assert_eq!(collect_css(&doc), "rect { fill: red; }");
    }

    #[test]
    fn collect_css_joins_multiple_style_elements_in_document_order() {
        let doc = svg3_dom::parse(
            r#"<svg><style>rect { fill: red; }</style><g><style>circle { fill: blue; }</style></g></svg>"#,
        )
        .unwrap();
        assert_eq!(
            collect_css(&doc),
            "rect { fill: red; }\ncircle { fill: blue; }"
        );
    }

    #[test]
    fn collect_css_is_empty_without_style_elements() {
        let doc = svg3_dom::parse(r#"<svg><rect width="10" height="10"/></svg>"#).unwrap();
        assert!(collect_css(&doc).is_empty());
    }
}
