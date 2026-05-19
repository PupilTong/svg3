//! `svg3-style` — computed-style resolution for SVG3 documents.
//!
//! Styling follows the web platform: element styles are resolved with
//! [Stylo](https://crates.io/crates/stylo), Servo's CSS engine.
//!
//! Status: scaffolding. The Stylo dependency is wired so it builds with this
//! workspace's toolchain, but the cascade is not implemented yet. The planned
//! integration implements Stylo's `TElement` / `TNode` / `TDocument` traits
//! over [`svg3_dom`]; **Blitz (`blitz-dom`) is the reference implementation**
//! for driving Stylo over a custom (non-browser) DOM:
//! <https://github.com/DioxusLabs/blitz>.

use svg3_dom::Document;
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
    /// Not implemented yet — returns [`StyleError::NotImplemented`].
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
}
