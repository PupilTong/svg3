//! The SVG3 document model and runtime XML parser.
//!
//! A [`Document`] is a flat arena of [`Node`]s addressed by [`NodeId`].
//! Each `Node` carries [`Element`] data (tag kind + raw attributes) plus
//! its children's ids. Storing nodes in a `Vec` gives stable identifiers
//! and decouples tree mutation from the borrow checker — both useful for
//! the planned Stylo cascade (Blitz / `blitz-dom`, the canonical Stylo-over-
//! custom-DOM reference, uses the same shape).

use std::collections::BTreeMap;

mod element;
mod parser;

pub use element::ElementKind;
pub use parser::{parse, ParseError};

/// Root `<svg>` attribute that opts a document into svg3's 3D extension.
pub const SVG3_EXTENSION_ATTRIBUTE: &str = "extension";
/// Required value for [`SVG3_EXTENSION_ATTRIBUTE`].
pub const SVG3_EXTENSION_VALUE: &str = "pupiltong";

/// Element data living on a [`Node`]: a tag [`ElementKind`] and the raw
/// attributes parsed from XML.
///
/// Values are XML-unescape-normalised `String`s; the parser does not
/// interpret them into typed forms (e.g. `transform="translate(...)"` stays
/// a string). Higher layers — style resolution, rendering — decide how to
/// consume `class`, `id`, geometry attributes, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Tag kind.
    pub kind: ElementKind,
    /// Raw attributes, keyed by name.
    pub attributes: BTreeMap<String, String>,
}

impl Element {
    /// Construct an element with the given kind and no attributes.
    pub fn new(kind: ElementKind) -> Self {
        Self {
            kind,
            attributes: BTreeMap::new(),
        }
    }
}

/// Opaque stable identifier for a [`Node`] inside a [`Document`].
///
/// `NodeId`s are dense indices and are only meaningful inside the
/// [`Document`] that produced them — do not mix ids across documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

impl NodeId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index as u32)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// One node in a [`Document`]'s arena. Children are ids into the same
/// arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Element data on this node.
    pub element: Element,
    /// Children in document order.
    pub children: Vec<NodeId>,
}

/// A parsed svg3 document.
///
/// Nodes live in a flat arena, addressed by [`NodeId`]. The root is always
/// an `<svg>` element (svg3 inherits SVG 1.1's root; see [SPEC.md](../../SPEC.md)).
#[derive(Debug, Clone)]
pub struct Document {
    nodes: Vec<Node>,
    root: NodeId,
}

impl Document {
    /// Create an empty document containing just an `<svg>` root.
    pub fn new() -> Self {
        let svg = Node {
            element: Element::new(ElementKind::Svg),
            children: Vec::new(),
        };
        Self {
            nodes: vec![svg],
            root: NodeId(0),
        }
    }

    /// Construct a document from a pre-built arena and root id (used by
    /// the parser).
    pub(crate) fn from_arena(nodes: Vec<Node>, root: NodeId) -> Self {
        Self { nodes, root }
    }

    /// The id of the root `<svg>` node.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Borrow a node by id. Panics if `id` does not belong to this
    /// document.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Mutably borrow a node by id. Panics if `id` does not belong to this
    /// document.
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    /// Convenience shorthand for `&doc.node(id).element`.
    pub fn element(&self, id: NodeId) -> &Element {
        &self.node(id).element
    }

    /// Whether the root `<svg>` opts into svg3's 3D extension features.
    pub fn svg3_extension_enabled(&self) -> bool {
        self.element(self.root())
            .attributes
            .get(SVG3_EXTENSION_ATTRIBUTE)
            .is_some_and(|value| value.trim() == SVG3_EXTENSION_VALUE)
    }

    /// Append a new child element under `parent` and return its id.
    pub fn append_child(&mut self, parent: NodeId, kind: ElementKind) -> NodeId {
        let id = self.alloc(Element::new(kind));
        self.nodes[parent.index()].children.push(id);
        id
    }

    /// Number of nodes in the arena (always at least 1 — the root).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Always `false` for a constructed document; provided to satisfy
    /// clippy's `len_without_is_empty`.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn alloc(&mut self, element: Element) -> NodeId {
        let id = NodeId::from_index(self.nodes.len());
        self.nodes.push(Node {
            element,
            children: Vec::new(),
        });
        id
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_can_be_built_programmatically() {
        let mut doc = Document::new();
        let cube_id = doc.append_child(doc.root(), ElementKind::Cube);

        assert_eq!(doc.element(doc.root()).kind, ElementKind::Svg);
        assert_eq!(doc.node(doc.root()).children, vec![cube_id]);
        assert_eq!(doc.element(cube_id).kind, ElementKind::Cube);
        assert_eq!(doc.len(), 2);
    }

    #[test]
    fn svg3_extension_requires_root_pupiltong_attribute() {
        let plain = crate::dom::parse(r#"<svg><cube/></svg>"#).unwrap();
        assert!(!plain.svg3_extension_enabled());

        let enabled = crate::dom::parse(r#"<svg extension="pupiltong"><cube/></svg>"#).unwrap();
        assert!(enabled.svg3_extension_enabled());

        let other = crate::dom::parse(r#"<svg extension="other"><cube/></svg>"#).unwrap();
        assert!(!other.svg3_extension_enabled());
    }
}
