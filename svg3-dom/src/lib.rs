//! `svg3-dom` — the SVG3 document model and runtime XML parser.
//!
//! Parses SVG3 XML (an SVG/XML dialect extended with 3D elements such as
//! `<scene>`, `<cube>` and `<ellipsoid>`) into a mutable element tree.
//!
//! The parser is structural only at this milestone: attribute *values* are
//! preserved as raw strings (so the planned Stylo cascade can consume `class`,
//! `id`, `style`, …), but no attribute values are interpreted into typed
//! representations (e.g. `transform="translate(...)"` is not parsed into a
//! matrix). Text content inside elements is ignored. Namespaces are not
//! handled.

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
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
    /// Map an XML tag name to an [`ElementKind`]. Unknown tags are preserved
    /// so the document can still be inspected even if it uses non-svg3
    /// elements; the planned Stylo cascade will match selectors on the raw
    /// tag name regardless.
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "scene" => Self::Scene,
            "g" | "group" => Self::Group,
            "cube" => Self::Cube,
            "ellipsoid" => Self::Ellipsoid,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// The tag name as it appears in source XML.
    pub fn as_tag(&self) -> &str {
        match self {
            Self::Scene => "scene",
            Self::Group => "group",
            Self::Cube => "cube",
            Self::Ellipsoid => "ellipsoid",
            Self::Unknown(t) => t.as_str(),
        }
    }
}

/// A single node in the document tree.
///
/// Attribute values are stored as raw `String`s in document order's
/// lexicographic projection (a `BTreeMap`); the parser does not interpret
/// them. Higher layers (style, render) decide how to consume `class`, `id`,
/// `transform`, geometry attributes, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// What kind of element this is.
    pub kind: ElementKind,
    /// Raw attributes, keyed by name. Values are XML-unescaped strings.
    pub attributes: BTreeMap<String, String>,
    /// Child nodes, in document order.
    pub children: Vec<Node>,
}

impl Node {
    /// Create a leaf node of `kind` with no attributes and no children.
    pub fn new(kind: ElementKind) -> Self {
        Self {
            kind,
            attributes: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    /// Append `child` and return `self`, for ergonomic tree building.
    pub fn with_child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }
}

/// A parsed SVG3 document. The root is required to be a `<scene>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The root node (an [`ElementKind::Scene`]).
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
    /// The document contained no element.
    #[error("the document is empty")]
    EmptyDocument,
    /// The root element was not `<scene>`.
    #[error("expected `<scene>` root element, found `<{found}>`")]
    UnexpectedRoot {
        /// The tag name that was found instead.
        found: String,
    },
    /// An element was left open at end of document.
    #[error("unclosed `<{tag}>` element at end of document")]
    UnclosedElement {
        /// The tag name that was never closed.
        tag: String,
    },
    /// Underlying XML reader error (malformed XML).
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),
    /// Underlying XML attribute parser error.
    #[error("XML attribute error: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    /// Tag name or attribute key was not valid UTF-8.
    #[error("invalid UTF-8 in document: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

/// Parse an SVG3 document from XML text.
///
/// The root element must be `<scene>`. Tag names are mapped via
/// [`ElementKind::from_tag`]; unknown tags become [`ElementKind::Unknown`] so
/// the document round-trips even when it contains non-svg3 elements.
/// Attribute values are XML-unescaped and stored verbatim on the [`Node`].
pub fn parse(input: &str) -> Result<Document, ParseError> {
    let mut reader = Reader::from_str(input);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                stack.push(build_node(&e)?);
            }
            Event::Empty(e) => {
                let node = build_node(&e)?;
                attach(&mut stack, &mut root, node);
            }
            Event::End(_) => {
                let node = stack.pop().ok_or(ParseError::EmptyDocument)?;
                attach(&mut stack, &mut root, node);
            }
            Event::Eof => break,
            // Text, comments, CDATA, processing instructions, XML
            // declarations and DOCTYPEs are ignored at this milestone.
            _ => {}
        }
        buf.clear();
    }

    if let Some(unclosed) = stack.pop() {
        return Err(ParseError::UnclosedElement {
            tag: unclosed.kind.as_tag().to_owned(),
        });
    }

    let root = root.ok_or(ParseError::EmptyDocument)?;
    if root.kind != ElementKind::Scene {
        return Err(ParseError::UnexpectedRoot {
            found: root.kind.as_tag().to_owned(),
        });
    }
    Ok(Document { root })
}

fn build_node(e: &BytesStart<'_>) -> Result<Node, ParseError> {
    let tag = std::str::from_utf8(e.name().into_inner())?;
    let mut node = Node::new(ElementKind::from_tag(tag));
    for attr in e.attributes() {
        let attr = attr?;
        let key = std::str::from_utf8(attr.key.into_inner())?.to_owned();
        let value = attr.normalized_value(XmlVersion::Implicit1_0)?.into_owned();
        node.attributes.insert(key, value);
    }
    Ok(node)
}

fn attach(stack: &mut [Node], root: &mut Option<Node>, node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        // `attach` is only called from `Empty` / `End` arms; quick-xml
        // guarantees a balanced or error-surfaced document, so on a
        // top-level Empty/End we are establishing the single root.
        // (Multiple top-level siblings would already have failed at the
        // XML layer.)
        *root = Some(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_mapping_recognises_3d_elements() {
        assert_eq!(ElementKind::from_tag("cube"), ElementKind::Cube);
        assert_eq!(ElementKind::from_tag("ellipsoid"), ElementKind::Ellipsoid);
        assert_eq!(ElementKind::from_tag("scene"), ElementKind::Scene);
        assert_eq!(ElementKind::from_tag("g"), ElementKind::Group);
        assert_eq!(ElementKind::from_tag("group"), ElementKind::Group);
        assert_eq!(
            ElementKind::from_tag("widget"),
            ElementKind::Unknown("widget".to_owned())
        );
    }

    #[test]
    fn document_tree_can_be_built_programmatically() {
        let cube = Node::new(ElementKind::Cube);
        let mut doc = Document::new();
        doc.root = doc.root.with_child(cube);

        assert_eq!(doc.root.kind, ElementKind::Scene);
        assert_eq!(doc.root.children.len(), 1);
        assert_eq!(doc.root.children[0].kind, ElementKind::Cube);
    }

    #[test]
    fn parse_empty_scene() {
        let doc = parse("<scene/>").unwrap();
        assert_eq!(doc.root.kind, ElementKind::Scene);
        assert!(doc.root.attributes.is_empty());
        assert!(doc.root.children.is_empty());
    }

    #[test]
    fn parse_scene_with_explicit_close() {
        let doc = parse("<scene></scene>").unwrap();
        assert_eq!(doc.root.kind, ElementKind::Scene);
        assert!(doc.root.children.is_empty());
    }

    #[test]
    fn parse_single_cube() {
        let doc = parse("<scene><cube/></scene>").unwrap();
        assert_eq!(doc.root.kind, ElementKind::Scene);
        assert_eq!(doc.root.children.len(), 1);
        assert_eq!(doc.root.children[0].kind, ElementKind::Cube);
    }

    #[test]
    fn parse_nested_groups() {
        let xml = "<scene><group><cube/><ellipsoid/></group></scene>";
        let doc = parse(xml).unwrap();
        let group = &doc.root.children[0];
        assert_eq!(group.kind, ElementKind::Group);
        assert_eq!(group.children.len(), 2);
        assert_eq!(group.children[0].kind, ElementKind::Cube);
        assert_eq!(group.children[1].kind, ElementKind::Ellipsoid);
    }

    #[test]
    fn parse_preserves_attributes_and_unescapes_values() {
        let xml = r#"<scene><cube id="a" class="b" size="2" label="a&amp;b"/></scene>"#;
        let doc = parse(xml).unwrap();
        let cube = &doc.root.children[0];
        assert_eq!(cube.kind, ElementKind::Cube);
        assert_eq!(cube.attributes.get("id").map(String::as_str), Some("a"));
        assert_eq!(cube.attributes.get("class").map(String::as_str), Some("b"));
        assert_eq!(cube.attributes.get("size").map(String::as_str), Some("2"));
        assert_eq!(
            cube.attributes.get("label").map(String::as_str),
            Some("a&b")
        );
    }

    #[test]
    fn parse_unknown_elements_kept_verbatim() {
        let doc = parse("<scene><widget/></scene>").unwrap();
        assert_eq!(
            doc.root.children[0].kind,
            ElementKind::Unknown("widget".to_owned())
        );
    }

    #[test]
    fn parse_empty_input_errors() {
        assert!(matches!(parse(""), Err(ParseError::EmptyDocument)));
    }

    #[test]
    fn parse_wrong_root_errors() {
        let err = parse("<cube/>").unwrap_err();
        match err {
            ParseError::UnexpectedRoot { found } => assert_eq!(found, "cube"),
            other => panic!("expected UnexpectedRoot, got {other:?}"),
        }
    }

    #[test]
    fn parse_unclosed_element_errors() {
        let err = parse("<scene>").unwrap_err();
        match err {
            ParseError::UnclosedElement { tag } => assert_eq!(tag, "scene"),
            other => panic!("expected UnclosedElement, got {other:?}"),
        }
    }
}
