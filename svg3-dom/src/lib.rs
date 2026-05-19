//! `svg3-dom` — the SVG3 document model.
//!
//! Parses SVG3 XML (an SVG/XML dialect extended with 3D elements such as
//! `<scene>`, `<cube>` and `<ellipsoid>`) into a mutable element tree.
//!
//! Status: scaffolding. The tree model is usable; the XML parser is not
//! implemented yet ([`parse`] returns [`ParseError::NotImplemented`]).

use glam::{Mat4, Vec3};
use thiserror::Error;

/// The kind of an SVG3 element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementKind {
    /// Root 3D scene container.
    Scene,
    /// Grouping / transform container.
    Group,
    /// Axis-aligned box primitive.
    Cube,
    /// Ellipsoid primitive.
    Ellipsoid,
    /// An element not recognised yet (tag name kept verbatim).
    Unknown(String),
}

impl ElementKind {
    /// Map an XML tag name to an [`ElementKind`].
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "scene" => Self::Scene,
            "g" | "group" => Self::Group,
            "cube" => Self::Cube,
            "ellipsoid" => Self::Ellipsoid,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

/// A single node in the document tree.
#[derive(Debug, Clone)]
pub struct Node {
    /// What kind of element this is.
    pub kind: ElementKind,
    /// Local transform applied to this node and its subtree.
    pub transform: Mat4,
    /// Child nodes, in document order.
    pub children: Vec<Node>,
}

impl Node {
    /// Create a leaf node of `kind` with an identity transform.
    pub fn new(kind: ElementKind) -> Self {
        Self {
            kind,
            transform: Mat4::IDENTITY,
            children: Vec::new(),
        }
    }

    /// Append `child` and return `self`, for ergonomic tree building.
    pub fn with_child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    /// Pre-multiply this node's local transform by a translation of `offset`.
    pub fn translate(&mut self, offset: Vec3) {
        self.transform = Mat4::from_translation(offset) * self.transform;
    }
}

/// A parsed SVG3 document.
#[derive(Debug, Clone)]
pub struct Document {
    /// The root node (conventionally an [`ElementKind::Scene`]).
    pub root: Node,
}

impl Document {
    /// Create an empty document with a `<scene>` root.
    pub fn new() -> Self {
        Self {
            root: Node::new(ElementKind::Scene),
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur while parsing an SVG3 document.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The XML parser is not implemented yet (scaffolding).
    #[error("SVG3 parsing is not implemented yet")]
    NotImplemented,
    /// Underlying XML reader error (surfaced once parsing lands).
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),
}

/// Parse an SVG3 document from XML text.
///
/// Not implemented yet — returns [`ParseError::NotImplemented`]. The signature
/// is stable so downstream crates can build against it now.
pub fn parse(_input: &str) -> Result<Document, ParseError> {
    log::debug!("svg3-dom: parse() called on the scaffold (not implemented)");
    Err(ParseError::NotImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_mapping_recognises_3d_elements() {
        assert_eq!(ElementKind::from_tag("cube"), ElementKind::Cube);
        assert_eq!(ElementKind::from_tag("ellipsoid"), ElementKind::Ellipsoid);
        assert_eq!(ElementKind::from_tag("scene"), ElementKind::Scene);
        assert_eq!(
            ElementKind::from_tag("widget"),
            ElementKind::Unknown("widget".to_owned())
        );
    }

    #[test]
    fn document_tree_can_be_built_and_traversed() {
        let mut cube = Node::new(ElementKind::Cube);
        cube.translate(Vec3::new(1.0, 0.0, 0.0));

        let mut doc = Document::new();
        doc.root = doc.root.with_child(cube);

        assert_eq!(doc.root.kind, ElementKind::Scene);
        assert_eq!(doc.root.children.len(), 1);
        assert_eq!(doc.root.children[0].kind, ElementKind::Cube);
    }

    #[test]
    fn parse_is_not_implemented_yet() {
        assert!(matches!(parse("<scene/>"), Err(ParseError::NotImplemented)));
    }
}
