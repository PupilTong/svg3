//! SVG filter definition resolution for the renderer.
//!
//! This module intentionally covers only the first GPU-backed filter
//! primitive implemented by the crate: `<feGaussianBlur>` inside a referenced
//! `<filter>`. The DOM keeps all attributes raw, so this layer resolves the
//! `filter="url(#id)"` paint-time reference and parses `stdDeviation`.

use std::collections::BTreeMap;

use svg3_dom::{Document, Element, ElementKind};

/// A resolved `<feGaussianBlur>` primitive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GaussianBlur {
    /// Horizontal standard deviation in filter/render pixels.
    pub(crate) std_deviation_x: f32,
    /// Vertical standard deviation in filter/render pixels.
    pub(crate) std_deviation_y: f32,
}

impl GaussianBlur {
    /// Whether this blur changes the source image.
    pub(crate) fn is_visible(self) -> bool {
        self.std_deviation_x > 0.0 || self.std_deviation_y > 0.0
    }
}

/// Filter definitions keyed by their XML `id` attribute.
#[derive(Debug, Default)]
pub(crate) struct FilterDefinitions {
    filters: BTreeMap<String, GaussianBlur>,
}

impl FilterDefinitions {
    /// Collect supported filter definitions from the whole document.
    ///
    /// If duplicate `id` values appear, the first supported definition wins,
    /// matching SVG's first-element lookup behavior for fragment references.
    pub(crate) fn collect(document: &Document) -> Self {
        let mut definitions = Self::default();
        let mut stack = vec![document.root()];
        while let Some(id) = stack.pop() {
            let node = document.node(id);
            if node.element.kind == ElementKind::Filter {
                if let (Some(filter_id), Some(blur)) = (
                    node.element.attributes.get("id"),
                    resolve_gaussian_blur(document, node.children.iter().copied()),
                ) {
                    definitions
                        .filters
                        .entry(filter_id.to_owned())
                        .or_insert(blur);
                }
                continue;
            }
            stack.extend(node.children.iter().rev().copied());
        }
        definitions
    }

    /// Resolve an element's `filter="url(#id)"` presentation attribute.
    ///
    /// Missing, unsupported, malformed, or unknown filter references resolve
    /// to `None`, so the caller renders the element as if no filter were set.
    pub(crate) fn resolve(&self, element: &Element) -> Option<GaussianBlur> {
        let id = element
            .attributes
            .get("filter")
            .and_then(|value| filter_reference_id(value))?;
        self.filters.get(id).copied()
    }
}

fn resolve_gaussian_blur(
    document: &Document,
    children: impl Iterator<Item = svg3_dom::NodeId>,
) -> Option<GaussianBlur> {
    for child_id in children {
        let child = document.element(child_id);
        if child.kind == ElementKind::FeGaussianBlur {
            return Some(
                child
                    .attributes
                    .get("stdDeviation")
                    .and_then(|value| parse_std_deviation(value))
                    .unwrap_or(GaussianBlur {
                        std_deviation_x: 0.0,
                        std_deviation_y: 0.0,
                    }),
            );
        }
    }
    None
}

fn filter_reference_id(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return None;
    }
    let inner = value.strip_prefix("url(")?.strip_suffix(')')?.trim();
    let inner = inner
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .or_else(|| {
            inner
                .strip_prefix('\'')
                .and_then(|quoted| quoted.strip_suffix('\''))
        })
        .unwrap_or(inner)
        .trim();
    let id = inner.strip_prefix('#')?.trim();
    (!id.is_empty()).then_some(id)
}

fn parse_std_deviation(value: &str) -> Option<GaussianBlur> {
    let values: Vec<f32> = value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>)
        .collect::<Result<_, _>>()
        .ok()?;
    let (x, y) = match values.as_slice() {
        [one] => (*one, *one),
        [x, y, ..] => (*x, *y),
        [] => return None,
    };
    (x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0).then_some(GaussianBlur {
        std_deviation_x: x,
        std_deviation_y: y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_resolves_url_referenced_gaussian_blur() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4 2"/></filter><rect filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];

        assert_eq!(
            definitions.resolve(document.element(rect_id)),
            Some(GaussianBlur {
                std_deviation_x: 4.0,
                std_deviation_y: 2.0,
            })
        );
    }

    #[test]
    fn filter_reference_accepts_quoted_fragment_urls() {
        assert_eq!(filter_reference_id("url(\"#soft\")"), Some("soft"));
        assert_eq!(filter_reference_id("url('#soft')"), Some("soft"));
        assert_eq!(filter_reference_id("none"), None);
    }

    #[test]
    fn duplicate_filter_ids_keep_the_first_definition() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="2"/></filter><filter id="soft"><feGaussianBlur stdDeviation="8"/></filter><rect filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[2];

        assert_eq!(
            definitions.resolve(document.element(rect_id)),
            Some(GaussianBlur {
                std_deviation_x: 2.0,
                std_deviation_y: 2.0,
            })
        );
    }

    #[test]
    fn std_deviation_uses_single_value_for_both_axes() {
        assert_eq!(
            parse_std_deviation("3"),
            Some(GaussianBlur {
                std_deviation_x: 3.0,
                std_deviation_y: 3.0,
            })
        );
    }

    #[test]
    fn std_deviation_rejects_negative_values() {
        assert_eq!(parse_std_deviation("-1"), None);
    }
}
