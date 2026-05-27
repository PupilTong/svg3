//! Runtime svg3 XML parser.
//!
//! Structural-only at this milestone: attribute *values* are preserved as raw
//! strings (so the planned Stylo cascade can consume `class`, `id`, `style`,
//! …) but no attribute values are interpreted into typed representations.
//! Text content inside elements is ignored. Namespaces are not handled.

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use thiserror::Error;

use super::{Document, Element, ElementKind, Node, NodeId};

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
/// see [SPEC.md](../../../SPEC.md)). Tag names are mapped via
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
        let tag = arena[unclosed_id.index()].element.kind.as_tag().to_owned();
        return Err(ParseError::UnclosedElement { tag });
    }

    let root = root.ok_or(ParseError::EmptyDocument)?;
    let root_kind = &arena[root.index()].element.kind;
    if *root_kind != ElementKind::Svg {
        return Err(ParseError::UnexpectedRoot {
            found: root_kind.as_tag().to_owned(),
        });
    }

    Ok(Document::from_arena(arena, root))
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
    let id = NodeId::from_index(arena.len());
    arena.push(Node {
        element,
        children: Vec::new(),
    });
    id
}

fn attach(arena: &mut [Node], parents: &[NodeId], root: &mut Option<NodeId>, id: NodeId) {
    if let Some(parent) = parents.last() {
        arena[parent.index()].children.push(id);
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
    fn parse_filter_preserves_fe_image_attributes() {
        let xml = r#"<svg><filter id="tex"><feImage href="data:image/png;base64,abc" x="4" y="6" width="12" height="14"/></filter></svg>"#;
        let doc = parse(xml).unwrap();
        let filter_id = doc.node(doc.root()).children[0];
        let filter = doc.node(filter_id);
        assert_eq!(filter.element.kind, ElementKind::Filter);
        assert_eq!(filter.children.len(), 1);

        let image = doc.element(filter.children[0]);
        assert_eq!(image.kind, ElementKind::FeImage);
        assert_eq!(
            image.attributes.get("href").map(String::as_str),
            Some("data:image/png;base64,abc")
        );
        assert_eq!(image.attributes.get("x").map(String::as_str), Some("4"));
        assert_eq!(image.attributes.get("y").map(String::as_str), Some("6"));
        assert_eq!(
            image.attributes.get("width").map(String::as_str),
            Some("12")
        );
        assert_eq!(
            image.attributes.get("height").map(String::as_str),
            Some("14")
        );
    }

    #[test]
    fn parse_defs_and_marker_elements() {
        let xml = r#"<svg><defs><marker id="arrow" markerWidth="10" markerHeight="8" refX="10" refY="4" orient="auto"><path d="M0 0 L10 4 L0 8 Z"/></marker></defs></svg>"#;
        let doc = parse(xml).unwrap();
        let defs_id = doc.node(doc.root()).children[0];
        let defs = doc.node(defs_id);
        assert_eq!(defs.element.kind, ElementKind::Defs);
        assert_eq!(defs.children.len(), 1);

        let marker = doc.node(defs.children[0]);
        assert_eq!(marker.element.kind, ElementKind::Marker);
        assert_eq!(
            marker.element.attributes.get("id").map(String::as_str),
            Some("arrow")
        );
        assert_eq!(marker.children.len(), 1);
        assert_eq!(doc.element(marker.children[0]).kind, ElementKind::Path);
    }

    #[test]
    fn parse_nested_groups() {
        let xml = "<svg><g><cube/><ellipsoid/><cylinder/></g></svg>";
        let doc = parse(xml).unwrap();
        let group_id = doc.node(doc.root()).children[0];
        let group = doc.node(group_id);
        assert_eq!(group.element.kind, ElementKind::Group);
        assert_eq!(group.children.len(), 3);
        assert_eq!(doc.element(group.children[0]).kind, ElementKind::Cube);
        assert_eq!(doc.element(group.children[1]).kind, ElementKind::Ellipsoid);
        assert_eq!(doc.element(group.children[2]).kind, ElementKind::Cylinder);
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
