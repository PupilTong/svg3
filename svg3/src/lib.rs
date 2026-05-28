//! `svg3` — an extended SVG renderer for 3D models.
//!
//! The crate is split into two layers:
//!
//! - [`dom`] — parse SVG3 XML into an element tree.
//! - [`render`] — paint the parsed scene with wgpu.
//!
//! Styling currently reads SVG 1.1 presentation attributes and inline
//! `style="..."` declarations directly inside [`render`]; a full CSS cascade
//! (the historical Stylo direction) is a paused roadmap item.

pub mod dom;
pub mod render;

use thiserror::Error;

/// A top-level error from the parse → render pipeline.
#[derive(Debug, Error)]
pub enum Error {
    /// Document parsing failed.
    #[error(transparent)]
    Parse(#[from] dom::ParseError),
    /// Rendering failed.
    #[error(transparent)]
    Render(#[from] render::RenderError),
}

/// Parse and render an SVG3 document from XML text.
///
/// This is the intended public entry point: it parses `input`, then runs the
/// headless renderer and returns the resulting image.
pub fn render_str(input: &str, config: render::RenderConfig) -> Result<render::Image, Error> {
    let document = dom::parse(input)?;
    let image = render::Renderer::headless()?.render_to_image(&document, config)?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_str_renders_trivial_document() {
        // The full parse → render pipeline must succeed end-to-end on a
        // minimal valid document; this guards against the entry point
        // regressing back to "always errors before reaching the renderer".
        let image = render_str(
            "<svg width=\"4\" height=\"4\"/>",
            render::RenderConfig::default(),
        )
        .expect("render_str should succeed on a trivial <svg/> document");
        assert!(image.width > 0 && image.height > 0);
    }

    #[test]
    fn build_scene_renders_rect_across_layers() {
        // The render stage works through the re-exported render API too:
        // parse a `<rect>` and tessellate it, exercising the dom -> render
        // path end to end.
        let document = dom::parse(r#"<svg><rect width="20" height="10"/></svg>"#).unwrap();
        let mesh = render::build_scene(
            &document,
            render::Viewport {
                width: 20.0,
                height: 10.0,
            },
        );
        assert!(!mesh.is_empty());
    }

    #[test]
    fn build_scene_renders_line_across_layers() {
        let document = dom::parse(r#"<svg><line x2="20" y2="10" stroke="blue"/></svg>"#).unwrap();
        let mesh = render::build_scene(
            &document,
            render::Viewport {
                width: 20.0,
                height: 10.0,
            },
        );
        assert!(!mesh.is_empty());
    }

    #[test]
    fn build_scene_renders_path_across_layers() {
        let document =
            dom::parse(r#"<svg><path d="M 0 0 L 20 0 L 10 10 Z" fill="blue"/></svg>"#).unwrap();
        let mesh = render::build_scene(
            &document,
            render::Viewport {
                width: 20.0,
                height: 10.0,
            },
        );
        assert!(!mesh.is_empty());
    }
}
