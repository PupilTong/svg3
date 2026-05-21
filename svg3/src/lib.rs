//! `svg3` — an extended SVG renderer for 3D models.
//!
//! Umbrella crate tying the layers together:
//!
//! - [`dom`] — parse SVG3 XML into an element tree.
//! - [`style`] — resolve computed styles via Stylo.
//! - [`render`] — paint the styled scene with wgpu.
//!
//! Status: early scaffolding. [`render_str`] wires the layers. [`dom`]
//! parsing and [`render`]'s `<rect>`/`<circle>` path are implemented, but
//! [`style`] is still a skeleton, so `render_str` currently surfaces the
//! style stage's "not implemented" error.

pub use svg3_dom as dom;
pub use svg3_render as render;
pub use svg3_style as style;

use thiserror::Error;

/// A top-level error from the parse → style → render pipeline.
#[derive(Debug, Error)]
pub enum Error {
    /// Document parsing failed.
    #[error(transparent)]
    Parse(#[from] dom::ParseError),
    /// Style resolution failed.
    #[error(transparent)]
    Style(#[from] style::StyleError),
    /// Rendering failed.
    #[error(transparent)]
    Render(#[from] render::RenderError),
}

/// Parse, style and render an SVG3 document from XML text.
///
/// This is the intended public entry point. The parse and render stages are
/// implemented, but style resolution is still a scaffold, so the full
/// pipeline currently returns [`style::StyleError::NotImplemented`]. To
/// render `<rect>`/`<circle>` geometry today, use [`render::build_scene`] /
/// [`render::Renderer::render_to_image`] directly.
pub fn render_str(input: &str, config: render::RenderConfig) -> Result<render::Image, Error> {
    let document = dom::parse(input)?;
    let styles = style::StyleEngine::new();
    styles.resolve(&document)?;
    let image = render::Renderer::new().render_to_image(&document, config)?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_surfaces_style_not_implemented() {
        // Parsing now succeeds for valid svg3 XML (svg3 inherits SVG 1.1's
        // `<svg>` root); the pipeline fails at the next stage that is still
        // a skeleton (style resolution).
        let err = render_str("<svg/>", render::RenderConfig::default()).unwrap_err();
        assert!(matches!(
            err,
            Error::Style(style::StyleError::NotImplemented)
        ));
    }

    #[test]
    fn build_scene_renders_rect_across_crates() {
        // The render stage works without the (still-skeleton) style stage:
        // parse a `<rect>` and tessellate it through the re-exported render
        // API, exercising the dom -> render path end to end.
        let document = dom::parse(r#"<svg><rect width="20" height="10"/></svg>"#).unwrap();
        let mesh = render::build_scene(&document);
        assert!(!mesh.is_empty());
    }
}
