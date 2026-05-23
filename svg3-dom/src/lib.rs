//! `svg3-dom` — the SVG3 document model and runtime XML parser.
//!
//! A [`Document`] is a flat arena of [`Node`]s addressed by [`NodeId`].
//! Each `Node` carries [`Element`] data (tag kind + raw attributes) plus
//! its children's ids. Storing nodes in a `Vec` gives stable identifiers
//! and decouples tree mutation from the borrow checker — both useful for
//! the planned Stylo cascade (Blitz / `blitz-dom`, the canonical Stylo-over-
//! custom-DOM reference, uses the same shape).
//!
//! The parser is structural only at this milestone: attribute *values* are
//! preserved as raw strings (so the planned Stylo cascade can consume
//! `class`, `id`, `style`, …) but no attribute values are interpreted into
//! typed representations. Text content inside elements is ignored.
//! Namespaces are not handled.

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use thiserror::Error;

/// The kind of an svg3 element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementKind {
    /// Document root — the SVG 1.1 `<svg>` element. svg3 documents share
    /// the SVG 1.1 root per [SPEC.md](../SPEC.md) §1.1.
    Svg,
    /// Grouping / transform container (SVG 1.1 `<g>`).
    Group,
    /// Axis-aligned rectangle (SVG 1.1 `<rect>` basic shape).
    Rect,
    /// Circle (SVG 1.1 `<circle>` basic shape).
    Circle,
    /// Ellipse (SVG 1.1 `<ellipse>` basic shape).
    Ellipse,
    /// Polygon (SVG 1.1 `<polygon>` basic shape).
    Polygon,
    /// Connected line segments (SVG 1.1 `<polyline>` basic shape).
    Polyline,
    /// Line segment (SVG 1.1 `<line>` basic shape).
    Line,
    /// General path (SVG 1.1 `<path>`).
    Path,
    /// Filter definition container (SVG 1.1 `<filter>`).
    Filter,
    /// Gaussian blur filter primitive (SVG 1.1 `<feGaussianBlur>`).
    FeGaussianBlur,
    /// Colour matrix filter primitive (SVG 1.1 `<feColorMatrix>`).
    FeColorMatrix,
    /// Turbulence / fractal-noise filter primitive (SVG 1.1 `<feTurbulence>`).
    FeTurbulence,
    /// Specular lighting filter primitive (SVG 1.1 `<feSpecularLighting>`).
    FeSpecularLighting,
    /// Diffuse lighting filter primitive (SVG 1.1 `<feDiffuseLighting>`).
    FeDiffuseLighting,
    /// Morphology (dilate / erode) filter primitive (SVG 1.1 `<feMorphology>`).
    FeMorphology,
    /// Flood fill filter primitive (SVG 1.1 `<feFlood>`).
    FeFlood,
    /// Drop shadow filter primitive (SVG Filter Effects 1 `<feDropShadow>`).
    FeDropShadow,
    /// Displacement map filter primitive (SVG 1.1 `<feDisplacementMap>`).
    FeDisplacementMap,
    /// Convolution matrix filter primitive (SVG 1.1 `<feConvolveMatrix>`).
    FeConvolveMatrix,
    /// Component transfer filter primitive (SVG 1.1 `<feComponentTransfer>`).
    FeComponentTransfer,
    /// Per-channel transfer function for the R channel of `<feComponentTransfer>`.
    FeFuncR,
    /// Per-channel transfer function for the G channel of `<feComponentTransfer>`.
    FeFuncG,
    /// Per-channel transfer function for the B channel of `<feComponentTransfer>`.
    FeFuncB,
    /// Per-channel transfer function for the A channel of `<feComponentTransfer>`.
    FeFuncA,
    /// Directional light source for the lighting filter primitives.
    FeDistantLight,
    /// Point light source for the lighting filter primitives.
    FePointLight,
    /// Spot light source for the lighting filter primitives.
    FeSpotLight,
    /// Axis-aligned box primitive.
    Cube,
    /// Ellipsoid primitive.
    Ellipsoid,
    /// An element not recognised yet (tag name kept verbatim).
    Unknown(String),
}

impl ElementKind {
    /// Map an XML tag name to an [`ElementKind`]. Unrecognised tags are
    /// preserved verbatim so SVG 1.1 markup that svg3 has not implemented
    /// yet (text, gradients, masks, …) still round-trips through the
    /// DOM; the planned Stylo cascade can match selectors on the raw tag
    /// name regardless.
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "svg" => Self::Svg,
            "g" => Self::Group,
            "rect" => Self::Rect,
            "circle" => Self::Circle,
            "ellipse" => Self::Ellipse,
            "polygon" => Self::Polygon,
            "polyline" => Self::Polyline,
            "line" => Self::Line,
            "path" => Self::Path,
            "filter" => Self::Filter,
            "feGaussianBlur" => Self::FeGaussianBlur,
            "feColorMatrix" => Self::FeColorMatrix,
            "feTurbulence" => Self::FeTurbulence,
            "feSpecularLighting" => Self::FeSpecularLighting,
            "feDiffuseLighting" => Self::FeDiffuseLighting,
            "feMorphology" => Self::FeMorphology,
            "feFlood" => Self::FeFlood,
            "feDropShadow" => Self::FeDropShadow,
            "feDisplacementMap" => Self::FeDisplacementMap,
            "feConvolveMatrix" => Self::FeConvolveMatrix,
            "feComponentTransfer" => Self::FeComponentTransfer,
            "feFuncR" => Self::FeFuncR,
            "feFuncG" => Self::FeFuncG,
            "feFuncB" => Self::FeFuncB,
            "feFuncA" => Self::FeFuncA,
            "feDistantLight" => Self::FeDistantLight,
            "fePointLight" => Self::FePointLight,
            "feSpotLight" => Self::FeSpotLight,
            "cube" => Self::Cube,
            "ellipsoid" => Self::Ellipsoid,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// The tag name as it appears in source XML.
    pub fn as_tag(&self) -> &str {
        match self {
            Self::Svg => "svg",
            Self::Group => "g",
            Self::Rect => "rect",
            Self::Circle => "circle",
            Self::Ellipse => "ellipse",
            Self::Polygon => "polygon",
            Self::Polyline => "polyline",
            Self::Line => "line",
            Self::Path => "path",
            Self::Filter => "filter",
            Self::FeGaussianBlur => "feGaussianBlur",
            Self::FeColorMatrix => "feColorMatrix",
            Self::FeTurbulence => "feTurbulence",
            Self::FeSpecularLighting => "feSpecularLighting",
            Self::FeDiffuseLighting => "feDiffuseLighting",
            Self::FeMorphology => "feMorphology",
            Self::FeFlood => "feFlood",
            Self::FeDropShadow => "feDropShadow",
            Self::FeDisplacementMap => "feDisplacementMap",
            Self::FeConvolveMatrix => "feConvolveMatrix",
            Self::FeComponentTransfer => "feComponentTransfer",
            Self::FeFuncR => "feFuncR",
            Self::FeFuncG => "feFuncG",
            Self::FeFuncB => "feFuncB",
            Self::FeFuncA => "feFuncA",
            Self::FeDistantLight => "feDistantLight",
            Self::FePointLight => "fePointLight",
            Self::FeSpotLight => "feSpotLight",
            Self::Cube => "cube",
            Self::Ellipsoid => "ellipsoid",
            Self::Unknown(t) => t.as_str(),
        }
    }
}

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
/// an `<svg>` element (svg3 inherits SVG 1.1's root; see [SPEC.md](../SPEC.md)).
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

    /// The id of the root `<svg>` node.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Borrow a node by id. Panics if `id` does not belong to this
    /// document.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    /// Mutably borrow a node by id. Panics if `id` does not belong to this
    /// document.
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0 as usize]
    }

    /// Convenience shorthand for `&doc.node(id).element`.
    pub fn element(&self, id: NodeId) -> &Element {
        &self.node(id).element
    }

    /// Append a new child element under `parent` and return its id.
    pub fn append_child(&mut self, parent: NodeId, kind: ElementKind) -> NodeId {
        let id = self.alloc(Element::new(kind));
        self.nodes[parent.0 as usize].children.push(id);
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
        let id = NodeId(self.nodes.len() as u32);
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

/// Errors that can occur while parsing an svg3 document.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The document contained no element.
    #[error("the document is empty")]
    EmptyDocument,
    /// The root element was not `<svg>`.
    #[error("expected `<svg>` root element, found `<{found}>`")]
    UnexpectedRoot {
        /// The tag name found at the document root.
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

/// Parse an svg3 document from XML text.
///
/// The root element must be `<svg>` (svg3 is an extension of SVG 1.1;
/// see [SPEC.md](../SPEC.md)). Tag names are mapped via
/// [`ElementKind::from_tag`]; unknown tags become [`ElementKind::Unknown`]
/// so SVG 1.1 markup not yet specialised by svg3 (paths, basic shapes, …)
/// still round-trips. Attribute values are XML-unescape-normalised and
/// stored verbatim on the owning [`Element`].
pub fn parse(input: &str) -> Result<Document, ParseError> {
    let mut reader = Reader::from_str(input);
    let mut arena: Vec<Node> = Vec::new();
    let mut parents: Vec<NodeId> = Vec::new();
    let mut root: Option<NodeId> = None;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let element = build_element(&e)?;
                let id = alloc(&mut arena, element);
                attach(&mut arena, &parents, &mut root, id);
                parents.push(id);
            }
            Event::Empty(e) => {
                let element = build_element(&e)?;
                let id = alloc(&mut arena, element);
                attach(&mut arena, &parents, &mut root, id);
            }
            Event::End(_) => {
                parents.pop().ok_or(ParseError::EmptyDocument)?;
            }
            Event::Eof => break,
            // Text, comments, CDATA, processing instructions, XML
            // declarations and DOCTYPEs are ignored at this milestone.
            _ => {}
        }
        buf.clear();
    }

    if let Some(unclosed_id) = parents.last() {
        let tag = arena[unclosed_id.0 as usize]
            .element
            .kind
            .as_tag()
            .to_owned();
        return Err(ParseError::UnclosedElement { tag });
    }

    let root = root.ok_or(ParseError::EmptyDocument)?;
    let root_kind = &arena[root.0 as usize].element.kind;
    if *root_kind != ElementKind::Svg {
        return Err(ParseError::UnexpectedRoot {
            found: root_kind.as_tag().to_owned(),
        });
    }

    Ok(Document { nodes: arena, root })
}

fn build_element(e: &BytesStart<'_>) -> Result<Element, ParseError> {
    let tag = std::str::from_utf8(e.name().into_inner())?;
    let mut element = Element::new(ElementKind::from_tag(tag));
    for attr in e.attributes() {
        let attr = attr?;
        let key = std::str::from_utf8(attr.key.into_inner())?.to_owned();
        let value = attr.normalized_value(XmlVersion::Implicit1_0)?.into_owned();
        element.attributes.insert(key, value);
    }
    Ok(element)
}

fn alloc(arena: &mut Vec<Node>, element: Element) -> NodeId {
    let id = NodeId(arena.len() as u32);
    arena.push(Node {
        element,
        children: Vec::new(),
    });
    id
}

fn attach(arena: &mut [Node], parents: &[NodeId], root: &mut Option<NodeId>, id: NodeId) {
    if let Some(parent) = parents.last() {
        arena[parent.0 as usize].children.push(id);
    } else {
        // First top-level allocation = root. quick-xml enforces XML's
        // single-root rule, so a sibling at the document level would
        // already have surfaced as an XML error before we got here.
        *root = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_mapping_recognises_svg3_elements() {
        assert_eq!(ElementKind::from_tag("svg"), ElementKind::Svg);
        assert_eq!(ElementKind::from_tag("g"), ElementKind::Group);
        assert_eq!(ElementKind::from_tag("rect"), ElementKind::Rect);
        assert_eq!(ElementKind::from_tag("circle"), ElementKind::Circle);
        assert_eq!(ElementKind::from_tag("ellipse"), ElementKind::Ellipse);
        assert_eq!(ElementKind::from_tag("polygon"), ElementKind::Polygon);
        assert_eq!(ElementKind::from_tag("polyline"), ElementKind::Polyline);
        assert_eq!(ElementKind::from_tag("line"), ElementKind::Line);
        assert_eq!(ElementKind::from_tag("path"), ElementKind::Path);
        assert_eq!(ElementKind::from_tag("filter"), ElementKind::Filter);
        assert_eq!(
            ElementKind::from_tag("feGaussianBlur"),
            ElementKind::FeGaussianBlur
        );
        assert_eq!(
            ElementKind::from_tag("feColorMatrix"),
            ElementKind::FeColorMatrix
        );
        assert_eq!(
            ElementKind::from_tag("feTurbulence"),
            ElementKind::FeTurbulence
        );
        assert_eq!(
            ElementKind::from_tag("feSpecularLighting"),
            ElementKind::FeSpecularLighting
        );
        assert_eq!(
            ElementKind::from_tag("feDiffuseLighting"),
            ElementKind::FeDiffuseLighting
        );
        assert_eq!(
            ElementKind::from_tag("feMorphology"),
            ElementKind::FeMorphology
        );
        assert_eq!(ElementKind::from_tag("feFlood"), ElementKind::FeFlood);
        assert_eq!(
            ElementKind::from_tag("feDropShadow"),
            ElementKind::FeDropShadow
        );
        assert_eq!(
            ElementKind::from_tag("feDisplacementMap"),
            ElementKind::FeDisplacementMap
        );
        assert_eq!(
            ElementKind::from_tag("feConvolveMatrix"),
            ElementKind::FeConvolveMatrix
        );
        assert_eq!(
            ElementKind::from_tag("feComponentTransfer"),
            ElementKind::FeComponentTransfer
        );
        assert_eq!(ElementKind::from_tag("feFuncR"), ElementKind::FeFuncR);
        assert_eq!(ElementKind::from_tag("feFuncA"), ElementKind::FeFuncA);
        assert_eq!(
            ElementKind::from_tag("feDistantLight"),
            ElementKind::FeDistantLight
        );
        assert_eq!(
            ElementKind::from_tag("fePointLight"),
            ElementKind::FePointLight
        );
        assert_eq!(
            ElementKind::from_tag("feSpotLight"),
            ElementKind::FeSpotLight
        );
        assert_eq!(ElementKind::from_tag("cube"), ElementKind::Cube);
        assert_eq!(ElementKind::from_tag("ellipsoid"), ElementKind::Ellipsoid);
        // `as_tag` round-trips a recognised kind back to its source name.
        assert_eq!(ElementKind::Rect.as_tag(), "rect");
        assert_eq!(ElementKind::Ellipse.as_tag(), "ellipse");
        assert_eq!(ElementKind::Polygon.as_tag(), "polygon");
        assert_eq!(ElementKind::Polyline.as_tag(), "polyline");
        assert_eq!(ElementKind::Line.as_tag(), "line");
        assert_eq!(ElementKind::Path.as_tag(), "path");
        assert_eq!(ElementKind::Filter.as_tag(), "filter");
        assert_eq!(ElementKind::FeGaussianBlur.as_tag(), "feGaussianBlur");
        assert_eq!(ElementKind::FeColorMatrix.as_tag(), "feColorMatrix");
        assert_eq!(ElementKind::FeFlood.as_tag(), "feFlood");
        assert_eq!(ElementKind::FeDropShadow.as_tag(), "feDropShadow");
        assert_eq!(
            ElementKind::FeComponentTransfer.as_tag(),
            "feComponentTransfer"
        );
        assert_eq!(ElementKind::FeFuncR.as_tag(), "feFuncR");
        assert_eq!(ElementKind::FeDistantLight.as_tag(), "feDistantLight");
        // `<group>` is not in SPEC.md; only `<g>` from SVG 1.1 is the
        // canonical grouping element.
        assert_eq!(
            ElementKind::from_tag("group"),
            ElementKind::Unknown("group".to_owned())
        );
        // Similarly, `<scene>` (an earlier draft name) is no longer
        // recognised — it round-trips as Unknown.
        assert_eq!(
            ElementKind::from_tag("scene"),
            ElementKind::Unknown("scene".to_owned())
        );
        assert_eq!(
            ElementKind::from_tag("widget"),
            ElementKind::Unknown("widget".to_owned())
        );
    }

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
    fn parse_empty_svg() {
        let doc = parse("<svg/>").unwrap();
        let root = doc.element(doc.root());
        assert_eq!(root.kind, ElementKind::Svg);
        assert!(root.attributes.is_empty());
        assert!(doc.node(doc.root()).children.is_empty());
        assert_eq!(doc.len(), 1);
    }

    #[test]
    fn parse_svg_with_explicit_close() {
        let doc = parse("<svg></svg>").unwrap();
        assert_eq!(doc.element(doc.root()).kind, ElementKind::Svg);
        assert!(doc.node(doc.root()).children.is_empty());
    }

    #[test]
    fn parse_single_cube() {
        let doc = parse("<svg><cube/></svg>").unwrap();
        assert_eq!(doc.element(doc.root()).kind, ElementKind::Svg);
        let children = &doc.node(doc.root()).children;
        assert_eq!(children.len(), 1);
        assert_eq!(doc.element(children[0]).kind, ElementKind::Cube);
        assert_eq!(doc.len(), 2);
    }

    #[test]
    fn parse_rect_preserves_geometry_attributes() {
        let xml = r#"<svg><rect x="10" y="20" width="50" height="40" rx="8" fill="blue"/></svg>"#;
        let doc = parse(xml).unwrap();
        let rect_id = doc.node(doc.root()).children[0];
        let rect = doc.element(rect_id);
        assert_eq!(rect.kind, ElementKind::Rect);
        assert_eq!(rect.attributes.get("x").map(String::as_str), Some("10"));
        assert_eq!(rect.attributes.get("y").map(String::as_str), Some("20"));
        assert_eq!(rect.attributes.get("width").map(String::as_str), Some("50"));
        assert_eq!(
            rect.attributes.get("height").map(String::as_str),
            Some("40")
        );
        assert_eq!(rect.attributes.get("rx").map(String::as_str), Some("8"));
        assert_eq!(
            rect.attributes.get("fill").map(String::as_str),
            Some("blue")
        );
    }

    #[test]
    fn parse_circle_preserves_geometry_attributes() {
        let xml = r#"<svg><circle cx="30" cy="40" r="25" fill="red"/></svg>"#;
        let doc = parse(xml).unwrap();
        let circle_id = doc.node(doc.root()).children[0];
        let circle = doc.element(circle_id);
        assert_eq!(circle.kind, ElementKind::Circle);
        assert_eq!(circle.attributes.get("cx").map(String::as_str), Some("30"));
        assert_eq!(circle.attributes.get("cy").map(String::as_str), Some("40"));
        assert_eq!(circle.attributes.get("r").map(String::as_str), Some("25"));
        assert_eq!(
            circle.attributes.get("fill").map(String::as_str),
            Some("red")
        );
    }

    #[test]
    fn parse_ellipse_preserves_geometry_attributes() {
        let xml = r#"<svg><ellipse cx="60" cy="40" rx="50" ry="30" fill="green"/></svg>"#;
        let doc = parse(xml).unwrap();
        let ellipse_id = doc.node(doc.root()).children[0];
        let ellipse = doc.element(ellipse_id);
        assert_eq!(ellipse.kind, ElementKind::Ellipse);
        assert_eq!(ellipse.attributes.get("cx").map(String::as_str), Some("60"));
        assert_eq!(ellipse.attributes.get("cy").map(String::as_str), Some("40"));
        assert_eq!(ellipse.attributes.get("rx").map(String::as_str), Some("50"));
        assert_eq!(ellipse.attributes.get("ry").map(String::as_str), Some("30"));
        assert_eq!(
            ellipse.attributes.get("fill").map(String::as_str),
            Some("green")
        );
    }

    #[test]
    fn parse_polygon_preserves_points_attribute() {
        let xml = r#"<svg><polygon points="0,0 20,0 10,16" fill="green"/></svg>"#;
        let doc = parse(xml).unwrap();
        let polygon_id = doc.node(doc.root()).children[0];
        let polygon = doc.element(polygon_id);
        assert_eq!(polygon.kind, ElementKind::Polygon);
        assert_eq!(
            polygon.attributes.get("points").map(String::as_str),
            Some("0,0 20,0 10,16")
        );
        assert_eq!(
            polygon.attributes.get("fill").map(String::as_str),
            Some("green")
        );
    }

    #[test]
    fn parse_polyline_preserves_geometry_attributes() {
        let xml = r#"<svg><polyline points="10,20 30,40 50,20" fill="green"/></svg>"#;
        let doc = parse(xml).unwrap();
        let polyline_id = doc.node(doc.root()).children[0];
        let polyline = doc.element(polyline_id);
        assert_eq!(polyline.kind, ElementKind::Polyline);
        assert_eq!(
            polyline.attributes.get("points").map(String::as_str),
            Some("10,20 30,40 50,20")
        );
        assert_eq!(
            polyline.attributes.get("fill").map(String::as_str),
            Some("green")
        );
    }

    #[test]
    fn parse_line_preserves_geometry_attributes() {
        let xml =
            r#"<svg><line x1="10" y1="20" x2="50" y2="40" stroke="blue" stroke-width="3"/></svg>"#;
        let doc = parse(xml).unwrap();
        let line_id = doc.node(doc.root()).children[0];
        let line = doc.element(line_id);
        assert_eq!(line.kind, ElementKind::Line);
        assert_eq!(line.attributes.get("x1").map(String::as_str), Some("10"));
        assert_eq!(line.attributes.get("y1").map(String::as_str), Some("20"));
        assert_eq!(line.attributes.get("x2").map(String::as_str), Some("50"));
        assert_eq!(line.attributes.get("y2").map(String::as_str), Some("40"));
        assert_eq!(
            line.attributes.get("stroke").map(String::as_str),
            Some("blue")
        );
        assert_eq!(
            line.attributes.get("stroke-width").map(String::as_str),
            Some("3")
        );
    }

    #[test]
    fn parse_path_preserves_geometry_attributes() {
        let xml =
            r#"<svg><path d="M10 20 L50 20 Z" fill="blue" stroke="red" stroke-width="3"/></svg>"#;
        let doc = parse(xml).unwrap();
        let path_id = doc.node(doc.root()).children[0];
        let path = doc.element(path_id);
        assert_eq!(path.kind, ElementKind::Path);
        assert_eq!(
            path.attributes.get("d").map(String::as_str),
            Some("M10 20 L50 20 Z")
        );
        assert_eq!(
            path.attributes.get("fill").map(String::as_str),
            Some("blue")
        );
        assert_eq!(
            path.attributes.get("stroke").map(String::as_str),
            Some("red")
        );
        assert_eq!(
            path.attributes.get("stroke-width").map(String::as_str),
            Some("3")
        );
    }

    #[test]
    fn parse_filter_preserves_gaussian_blur_attributes() {
        let xml = r#"<svg><filter id="soft"><feGaussianBlur stdDeviation="4 2"/></filter></svg>"#;
        let doc = parse(xml).unwrap();
        let filter_id = doc.node(doc.root()).children[0];
        let filter = doc.node(filter_id);
        assert_eq!(filter.element.kind, ElementKind::Filter);
        assert_eq!(
            filter.element.attributes.get("id").map(String::as_str),
            Some("soft")
        );
        assert_eq!(filter.children.len(), 1);

        let blur = doc.element(filter.children[0]);
        assert_eq!(blur.kind, ElementKind::FeGaussianBlur);
        assert_eq!(
            blur.attributes.get("stdDeviation").map(String::as_str),
            Some("4 2")
        );
    }

    #[test]
    fn parse_nested_groups() {
        let xml = "<svg><g><cube/><ellipsoid/></g></svg>";
        let doc = parse(xml).unwrap();
        let group_id = doc.node(doc.root()).children[0];
        let group = doc.node(group_id);
        assert_eq!(group.element.kind, ElementKind::Group);
        assert_eq!(group.children.len(), 2);
        assert_eq!(doc.element(group.children[0]).kind, ElementKind::Cube);
        assert_eq!(doc.element(group.children[1]).kind, ElementKind::Ellipsoid);
    }

    #[test]
    fn parse_preserves_attributes_and_unescapes_values() {
        let xml = r#"<svg><cube id="a" class="b" size="2" label="a&amp;b"/></svg>"#;
        let doc = parse(xml).unwrap();
        let cube_id = doc.node(doc.root()).children[0];
        let cube = doc.element(cube_id);
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
        let doc = parse("<svg><widget/></svg>").unwrap();
        let widget_id = doc.node(doc.root()).children[0];
        assert_eq!(
            doc.element(widget_id).kind,
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
    fn parse_scene_rooted_document_errors() {
        // `<scene>` was an earlier draft root; it is no longer recognised
        // and must surface as UnexpectedRoot.
        let err = parse("<scene/>").unwrap_err();
        match err {
            ParseError::UnexpectedRoot { found } => assert_eq!(found, "scene"),
            other => panic!("expected UnexpectedRoot, got {other:?}"),
        }
    }

    #[test]
    fn parse_unclosed_element_errors() {
        let err = parse("<svg>").unwrap_err();
        match err {
            ParseError::UnclosedElement { tag } => assert_eq!(tag, "svg"),
            other => panic!("expected UnclosedElement, got {other:?}"),
        }
    }
}
