//! Render-target configuration ([`RenderConfig`]) and the movable 3D
//! [`Camera`].
//!
//! Both feed [`RenderConfig::view_projection`], the matrix that maps scene
//! geometry to clip space: with no camera the scene is drawn flat through an
//! orthographic projection; with one it is viewed through a perspective
//! camera that can be placed anywhere in 3D space.

use glam::{Mat4, Vec3};

/// Target-surface configuration for a render pass.
#[derive(Debug, Clone, Copy)]
pub struct RenderConfig {
    /// Swapchain / texture format to render into. Reserved for the future
    /// windowed path; [`render_to_image`](crate::Renderer::render_to_image)
    /// always targets an sRGB `RGBA8` texture.
    pub format: wgpu::TextureFormat,
    /// Target width, in physical pixels.
    pub width: u32,
    /// Target height, in physical pixels.
    pub height: u32,
    /// Optional 3D camera. `None` draws content flat through the default
    /// orthographic [`projection`](RenderConfig::projection); `Some` views
    /// the scene — including 2D content in the plane `z = 0` — through a
    /// movable perspective [`Camera`].
    pub camera: Option<Camera>,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 1,
            height: 1,
            camera: None,
        }
    }
}

impl RenderConfig {
    /// The default 2D camera: an orthographic projection mapping SVG user
    /// space — origin top-left, y-down, spanning `0..width × 0..height` — to
    /// wgpu normalized device coordinates. `width`/`height` are clamped to
    /// at least 1, matching the render target, so the matrix stays valid
    /// (finite) for a zero-sized config.
    ///
    /// Assumes 1 user unit = 1 device pixel. The outer `<svg>`'s
    /// `width`/`height` drive the document viewport used for percentage
    /// lengths, but this target projection stays tied to output pixels;
    /// `viewBox` and `preserveAspectRatio` are not consulted yet.
    pub fn projection(&self) -> Mat4 {
        Mat4::orthographic_rh(
            0.0,
            self.width.max(1) as f32,
            self.height.max(1) as f32,
            0.0,
            -1.0,
            1.0,
        )
    }

    /// The matrix that maps scene geometry to clip space: the
    /// [`camera`](RenderConfig::camera)'s view-projection when one is set,
    /// otherwise the default orthographic [`projection`](RenderConfig::projection).
    pub fn view_projection(&self) -> Mat4 {
        match self.camera {
            Some(camera) => {
                let aspect = self.width.max(1) as f32 / self.height.max(1) as f32;
                camera.view_proj(aspect)
            }
            None => self.projection(),
        }
    }
}

/// A movable perspective camera.
///
/// Set one on [`RenderConfig::camera`] to view the scene — including 2D
/// content in the world plane `z = 0` — from any position in 3D space.
/// World space is left-handed, per SPEC §3.1: +X right, +Y down, +Z toward
/// the viewer.
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
    /// A straight-on reference camera framing a `width`×`height` document.
    ///
    /// The eye sits on the `+Z` (viewer) side, level with the document
    /// centre and far enough back that the document height fills the frame
    /// exactly. This is the natural starting point a caller perturbs to move
    /// the camera through 3D space. `width`/`height` are clamped to at least
    /// 1, matching the render target.
    pub fn facing(width: u32, height: u32) -> Self {
        let w = width.max(1) as f32;
        let h = height.max(1) as f32;
        let fov_y = std::f32::consts::FRAC_PI_4;
        // The eye distance at which a vertical field of view of `fov_y`
        // spans exactly `h` user units.
        let distance = (h / 2.0) / (fov_y / 2.0).tan();
        Self {
            eye: Vec3::new(w / 2.0, h / 2.0, distance),
            target: Vec3::new(w / 2.0, h / 2.0, 0.0),
            fov_y,
        }
    }

    /// Combined view-projection matrix for the given `aspect` (width / height).
    ///
    /// Left-handed, matching the SPEC §3.1 world (+X right, +Y down, +Z
    /// toward the viewer). The `-Y` up vector keeps y-down content upright:
    /// unlike [`RenderConfig::projection`], whose y-flip is baked into its
    /// orthographic bounds, a perspective matrix carries no flip of its own.
    ///
    /// `near`/`far` are a generous fixed range; geometry nearer than `0.1`
    /// or farther than `100_000` user units from the eye is clipped.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_lh(self.eye, self.target, Vec3::NEG_Y);
        let proj = Mat4::perspective_lh(self.fov_y, aspect, 0.1, 100_000.0);
        proj * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn camera_facing_frames_document() {
        // The straight-on reference camera frames a square document: its
        // centre lands on the NDC origin and every corner stays inside the
        // clip box, including the depth range, so nothing is clipped away.
        let camera = Camera::facing(100, 100);
        let view_proj = camera.view_proj(1.0);
        let at = |x, y| view_proj.project_point3(Vec3::new(x, y, 0.0));

        let centre = at(50.0, 50.0);
        assert!(
            centre.x.abs() < 1e-4 && centre.y.abs() < 1e-4,
            "document centre should project to the NDC origin: {centre:?}"
        );
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (100.0, 100.0)] {
            let ndc = at(x, y);
            assert!(
                ndc.x.abs() <= 1.0 + 1e-3 && ndc.y.abs() <= 1.0 + 1e-3,
                "corner ({x}, {y}) should be within the clip box: {ndc:?}"
            );
            assert!(
                (0.0..=1.0).contains(&ndc.z),
                "corner ({x}, {y}) should be within the depth range: {ndc:?}"
            );
        }
    }

    #[test]
    fn camera_facing_is_upright_not_mirrored() {
        // The straight-on camera renders y-down content the same way up as
        // the orthographic default: SVG-up (smaller y) maps to NDC +y, and
        // SVG-left (smaller x) maps to NDC -x. This pins the camera's
        // handedness and up vector.
        let view_proj = Camera::facing(100, 100).view_proj(1.0);
        let at = |x, y| view_proj.project_point3(Vec3::new(x, y, 0.0));

        let above = at(50.0, 20.0);
        let left = at(20.0, 50.0);
        assert!(
            above.y > 0.0,
            "a point above centre should map to +y NDC: {above:?}"
        );
        assert!(
            left.x < 0.0,
            "a point left of centre should map to -x NDC: {left:?}"
        );
    }

    #[test]
    fn view_projection_defaults_to_orthographic() {
        // With no camera set, `view_projection` is exactly the orthographic
        // `projection` — so 2D rendering is unchanged when a caller opts out.
        let config = RenderConfig {
            width: 200,
            height: 100,
            ..RenderConfig::default()
        };
        assert!(config.camera.is_none());
        assert_eq!(
            config.view_projection().to_cols_array(),
            config.projection().to_cols_array(),
        );
    }

    #[test]
    fn projection_maps_surface_corners_to_ndc() {
        let config = RenderConfig {
            width: 200,
            height: 100,
            ..RenderConfig::default()
        };
        let proj = config.projection();
        let at = |x, y| proj.project_point3(Vec3::new(x, y, 0.0));
        // SVG origin (top-left) -> NDC top-left; far corner -> bottom-right;
        // the surface centre -> the NDC origin.
        let tl = at(0.0, 0.0);
        let br = at(200.0, 100.0);
        let mid = at(100.0, 50.0);
        assert!((tl.x + 1.0).abs() < 1e-5 && (tl.y - 1.0).abs() < 1e-5);
        assert!((br.x - 1.0).abs() < 1e-5 && (br.y + 1.0).abs() < 1e-5);
        assert!(mid.x.abs() < 1e-5 && mid.y.abs() < 1e-5);
    }

    #[test]
    fn projection_is_finite_for_zero_sized_config() {
        // A zero-dimension config is clamped like the render target, so the
        // projection stays finite instead of dividing by a zero extent.
        let config = RenderConfig {
            width: 0,
            height: 0,
            ..RenderConfig::default()
        };
        let proj = config.projection();
        assert!(proj.to_cols_array().iter().all(|v| v.is_finite()));
    }
}
