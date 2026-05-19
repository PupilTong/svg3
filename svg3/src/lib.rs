//! `svg3` — an extended SVG renderer for 3D models.
//!
//! Umbrella crate tying the layers together:
//!
//! - [`dom`] — parse SVG3 XML into an element tree.
//! - [`style`] — resolve computed styles via Stylo.
//! - [`render`] — paint the styled scene with wgpu.
//!
//! Status: scaffolding. [`render_str`] wires the layers, but each layer is a
//! skeleton, so it currently surfaces the first stage's "not implemented"
//! error.

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
/// This is the intended public entry point. The pipeline is fully wired, but
/// each stage is still a scaffold, so this currently returns the first stage's
/// "not implemented" error.
pub fn render_str(input: &str, config: render::RenderConfig) -> Result<(), Error> {
    let document = dom::parse(input)?;
    let styles = style::StyleEngine::new();
    styles.resolve(&document)?;
    render::Renderer::new().render(&document, &styles, config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_surfaces_parse_not_implemented() {
        let err = render_str("<scene/>", render::RenderConfig::default()).unwrap_err();
        assert!(matches!(err, Error::Parse(dom::ParseError::NotImplemented)));
    }
}
