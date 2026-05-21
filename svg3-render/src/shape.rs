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

/// Parse an absolute SVG length into user units. Accepts a plain number or a
/// `px`-suffixed number (1px = 1 user unit). Percentages are handled by
/// [`Length`]; other units are not handled yet and yield `None`.
pub(crate) fn parse_length(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let number = trimmed.strip_suffix("px").unwrap_or(trimmed).trim();
    let parsed: f32 = number.parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

/// An SVG 1.1 `<length>`: an absolute value in user units, or a percentage
/// resolved against a viewport extent at use time ([SVG11] §7.10).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Length {
    /// An absolute length, already in user units.
    Px(f32),
    /// A percentage — `Percent(50.0)` is `"50%"` — resolved by [`Length::resolve`].
    Percent(f32),
}

impl Length {
    /// Parse an SVG length. A trailing `%` yields [`Length::Percent`];
    /// otherwise the absolute grammar of [`parse_length`] applies, yielding
    /// [`Length::Px`]. Returns `None` for an unparseable value.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if let Some(percent) = trimmed.strip_suffix('%') {
            let parsed: f32 = percent.trim().parse().ok()?;
            return parsed.is_finite().then_some(Self::Percent(parsed));
        }
        parse_length(trimmed).map(Self::Px)
    }

    /// Resolve the length to user units. A percentage is taken relative to
    /// `basis` (the relevant viewport extent); an absolute length ignores it.
    pub(crate) fn resolve(self, basis: f32) -> f32 {
        match self {
            Self::Px(value) => value,
            Self::Percent(percent) => percent / 100.0 * basis,
        }
    }

    /// Whether the length's numeric value is below zero. A `<rect>`'s
    /// `rx`/`ry` treats a negative value as "not properly specified", i.e.
    /// auto ([SVG11] §9.2).
    pub(crate) fn is_negative(self) -> bool {
        match self {
            Self::Px(value) | Self::Percent(value) => value < 0.0,
        }
    }
}

/// The SVG viewport that percentage lengths resolve against.
///
/// In the current model the viewport is the root `<svg>`'s `width`/`height`
/// resolved against the render target fallback — 1 user unit = 1 device
/// pixel. `viewBox` is not consulted yet.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    /// Viewport width in user units.
    pub width: f32,
    /// Viewport height in user units.
    pub height: f32,
}

impl Viewport {
    /// The percentage basis for a length that is neither purely horizontal
    /// nor vertical — e.g. a `<circle>`'s `r` ([SVG11] §7.10): the viewport
    /// diagonal divided by `√2`.
    pub(crate) fn diagonal(&self) -> f32 {
        self.width.hypot(self.height) * std::f32::consts::FRAC_1_SQRT_2
    }
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

    #[test]
    fn length_parse_distinguishes_absolute_and_percentage() {
        assert_eq!(Length::parse("50"), Some(Length::Px(50.0)));
        assert_eq!(Length::parse("50px"), Some(Length::Px(50.0)));
        assert_eq!(Length::parse("1e2"), Some(Length::Px(100.0)));
        assert_eq!(Length::parse("100%"), Some(Length::Percent(100.0)));
        assert_eq!(Length::parse("12.5%"), Some(Length::Percent(12.5)));
        // Unparseable values — including a bare `%` — yield `None`.
        assert_eq!(Length::parse("abc"), None);
        assert_eq!(Length::parse("%"), None);
    }

    #[test]
    fn length_resolve_applies_percentage_basis() {
        // An absolute length ignores the basis.
        assert_eq!(Length::Px(40.0).resolve(1000.0), 40.0);
        // A percentage scales the basis.
        assert_eq!(Length::Percent(100.0).resolve(300.0), 300.0);
        assert_eq!(Length::Percent(50.0).resolve(200.0), 100.0);
        assert_eq!(Length::Percent(0.0).resolve(200.0), 0.0);
    }

    #[test]
    fn viewport_diagonal_is_the_normalized_diagonal() {
        // For a square viewport the normalized diagonal equals the side.
        let vp = Viewport {
            width: 100.0,
            height: 100.0,
        };
        assert!((vp.diagonal() - 100.0).abs() < 1e-3);
    }
}
