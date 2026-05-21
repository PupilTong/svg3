//! Shared support for the SVG 1.1 basic-shape modules ([`crate::rect`],
//! [`crate::circle`]).
//!
//! Each basic-shape module resolves its own geometry, but they all share
//! the SVG length grammar, `fill` paint resolution, and the mesh [`Vertex`]
//! constructor — collected here so a new shape reuses the SVG 1.1 attribute
//! rules instead of reimplementing them.
//!
//! Stroke, `fill-opacity`, CSS / `style=""`-set properties, and the Stylo
//! cascade are not consulted yet — see the crate roadmap.

use svg3_dom::Element;

use crate::Vertex;

/// The SVG 1.1 initial `fill` value — opaque black — in linear RGBA.
const DEFAULT_FILL: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Resolve a shape's solid fill as linear RGBA in `[0, 1]`.
///
/// Reads the `fill` presentation attribute only — the CSS `style=""` form
/// and the Stylo cascade are not consulted yet. `fill="none"` yields
/// `None` (no fill geometry). A missing or unparseable value falls back to
/// the SVG 1.1 initial value, opaque black.
pub(crate) fn resolve_fill(element: &Element) -> Option<[f32; 4]> {
    let Some(value) = element.attributes.get("fill") else {
        return Some(DEFAULT_FILL);
    };
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return None;
    }
    Some(parse_color(value).unwrap_or(DEFAULT_FILL))
}

/// A mesh [`Vertex`] at `(x, y)` in SVG user space. Basic shapes are
/// two-dimensional, so the position lies in the plane `z = 0`
/// ([SPEC.md](../../SPEC.md) §3.1).
pub(crate) fn vertex(x: f32, y: f32, color: [f32; 4]) -> Vertex {
    Vertex {
        position: [x, y, 0.0],
        color,
    }
}

/// Parse an SVG length value into user units. Accepts a plain number or a
/// `px`-suffixed number (1px = 1 user unit). Percentages and other units
/// are not handled yet and yield `None`.
pub(crate) fn parse_length(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let number = trimmed.strip_suffix("px").unwrap_or(trimmed).trim();
    let parsed: f32 = number.parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

/// Parse an sRGB colour — `#rgb`, `#rrggbb`, or a named colour — into
/// linear RGBA. Returns `None` for an unrecognised value.
fn parse_color(value: &str) -> Option<[f32; 4]> {
    match value.strip_prefix('#') {
        Some(hex) => parse_hex(hex),
        None => parse_named(value),
    }
}

fn parse_hex(hex: &str) -> Option<[f32; 4]> {
    if !hex.is_ascii() {
        return None;
    }
    let (r, g, b) = match hex.as_bytes() {
        // `#rgb` shorthand: each nibble is doubled (`0xN` -> `0xNN`).
        [r, g, b] => (nibble(*r)? * 17, nibble(*g)? * 17, nibble(*b)? * 17),
        [r0, r1, g0, g1, b0, b1] => (byte(*r0, *r1)?, byte(*g0, *g1)?, byte(*b0, *b1)?),
        _ => return None,
    };
    Some([
        srgb_to_linear(r as f32 / 255.0),
        srgb_to_linear(g as f32 / 255.0),
        srgb_to_linear(b as f32 / 255.0),
        1.0,
    ])
}

fn nibble(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

fn byte(hi: u8, lo: u8) -> Option<u8> {
    Some(nibble(hi)? * 16 + nibble(lo)?)
}

/// A small subset of the SVG named colours — enough for the WPT basic-shape
/// tests and common authoring.
fn parse_named(name: &str) -> Option<[f32; 4]> {
    let hex = match name.to_ascii_lowercase().as_str() {
        "black" => "000000",
        "white" => "ffffff",
        "red" => "ff0000",
        "green" => "008000",
        "blue" => "0000ff",
        "yellow" => "ffff00",
        "cyan" | "aqua" => "00ffff",
        "magenta" | "fuchsia" => "ff00ff",
        "gray" | "grey" => "808080",
        "silver" => "c0c0c0",
        "maroon" => "800000",
        "navy" => "000080",
        "orange" => "ffa500",
        "purple" => "800080",
        "lime" => "00ff00",
        "teal" => "008080",
        _ => return None,
    };
    parse_hex(hex)
}

/// Convert one sRGB channel in `[0, 1]` to linear-light.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svg3_dom::ElementKind;

    /// Build an element carrying the given raw attributes. The kind is
    /// irrelevant — `resolve_fill` reads attributes regardless of tag.
    fn element(attrs: &[(&str, &str)]) -> Element {
        let mut element = Element::new(ElementKind::Rect);
        for (key, value) in attrs {
            element
                .attributes
                .insert((*key).to_owned(), (*value).to_owned());
        }
        element
    }

    #[test]
    fn parse_length_accepts_numbers_and_px() {
        assert_eq!(parse_length("50"), Some(50.0));
        assert_eq!(parse_length("50px"), Some(50.0));
        assert_eq!(parse_length("  12  "), Some(12.0));
        assert_eq!(parse_length("1e2"), Some(100.0));
        assert_eq!(parse_length("-3"), Some(-3.0));
    }

    #[test]
    fn parse_length_rejects_unsupported_values() {
        assert_eq!(parse_length(""), None);
        assert_eq!(parse_length("abc"), None);
        assert_eq!(parse_length("50%"), None);
        assert_eq!(parse_length("inf"), None);
    }

    #[test]
    fn resolve_fill_reads_presentation_attribute() {
        // Missing `fill` => SVG 1.1 initial value, opaque black.
        assert_eq!(resolve_fill(&element(&[])), Some([0.0, 0.0, 0.0, 1.0]));
        // `none` => no fill geometry.
        assert_eq!(resolve_fill(&element(&[("fill", "none")])), None);
        // Named + hex forms of pure blue resolve identically.
        assert_eq!(
            resolve_fill(&element(&[("fill", "blue")])),
            Some([0.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            resolve_fill(&element(&[("fill", "#0000ff")])),
            Some([0.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            resolve_fill(&element(&[("fill", "#00f")])),
            Some([0.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            resolve_fill(&element(&[("fill", "#ff0000")])),
            Some([1.0, 0.0, 0.0, 1.0])
        );
        // An unparseable colour falls back to black.
        assert_eq!(
            resolve_fill(&element(&[("fill", "bogus")])),
            Some([0.0, 0.0, 0.0, 1.0])
        );
    }
}
