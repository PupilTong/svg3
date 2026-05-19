//! `svg3-render` — GPU rendering for styled SVG3 scenes.
//!
//! Turns a styled [`svg3_dom`] document into GPU draw calls with
//! [wgpu](https://crates.io/crates/wgpu).
//!
//! Status: scaffolding. wgpu is wired as a dependency and the public types are
//! in place, but no device/surface is created and nothing is drawn yet.

use glam::{Mat4, Vec3};
use svg3_dom::Document;
use svg3_style::StyleEngine;
use thiserror::Error;

/// A single GPU vertex: position + linear RGBA colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Linear RGBA colour in `[0, 1]`.
    pub color: [f32; 4],
}

/// Target-surface configuration for a render pass.
#[derive(Debug, Clone, Copy)]
pub struct RenderConfig {
    /// Swapchain / texture format to render into.
    pub format: wgpu::TextureFormat,
    /// Target width, in physical pixels.
    pub width: u32,
    /// Target height, in physical pixels.
    pub height: u32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 1,
            height: 1,
        }
    }
}

/// A right-handed perspective camera.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// Eye position in world space.
    pub eye: Vec3,
    /// Look-at target in world space.
    pub target: Vec3,
    /// Vertical field of view, in radians.
    pub fov_y: f32,
}

impl Camera {
    /// Combined view-projection matrix for the given `aspect` (width / height).
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye, self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fov_y, aspect, 0.1, 100.0);
        proj * view
    }
}

/// Renders styled SVG3 documents onto a GPU surface.
///
/// Status: scaffolding — holds no GPU state yet.
#[derive(Debug, Default)]
pub struct Renderer;

impl Renderer {
    /// Create a new renderer.
    pub fn new() -> Self {
        Self
    }

    /// Render `document` (styled by `styles`) into the configured target.
    ///
    /// Not implemented yet — returns [`RenderError::NotImplemented`].
    pub fn render(
        &self,
        _document: &Document,
        _styles: &StyleEngine,
        _config: RenderConfig,
    ) -> Result<(), RenderError> {
        log::debug!("svg3-render: render() called on the scaffold (not implemented)");
        Err(RenderError::NotImplemented)
    }
}

/// Errors that can occur during rendering.
#[derive(Debug, Error)]
pub enum RenderError {
    /// The wgpu rendering path is not implemented yet (scaffolding).
    #[error("rendering is not implemented yet")]
    NotImplemented,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_layout_is_tightly_packed() {
        assert_eq!(
            std::mem::size_of::<Vertex>(),
            7 * std::mem::size_of::<f32>()
        );
    }

    #[test]
    fn camera_view_proj_is_finite() {
        let cam = Camera {
            eye: Vec3::new(0.0, 0.0, 5.0),
            target: Vec3::ZERO,
            fov_y: 1.0,
        };
        let m = cam.view_proj(16.0 / 9.0);
        assert!(m.to_cols_array().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn render_is_not_implemented_yet() {
        let r = Renderer::new();
        let doc = Document::new();
        let styles = StyleEngine::new();
        assert!(matches!(
            r.render(&doc, &styles, RenderConfig::default()),
            Err(RenderError::NotImplemented)
        ));
    }
}
