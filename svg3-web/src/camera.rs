//! Orbit-style camera controller for the browser app.
//!
//! A direct port of `app-macos/src/camera.rs`: the eye orbits a target — the
//! loaded document's centre — at a distance the user can change. Yaw and pitch
//! are spherical angles; at `yaw = pitch = 0` the eye sits straight in front of
//! the document on the `+Z` (viewer) side, matching
//! [`svg3::render::Camera::facing`]. It depends only on `glam` and
//! `svg3::render`, so it carries no platform or winit coupling.

use glam::Vec3;
use svg3::render::{Camera, Viewport};

/// Vertical field of view, in radians (matches `Camera::facing`).
const FOV_Y: f32 = std::f32::consts::FRAC_PI_4;
/// Pitch is clamped short of the poles so the look-at up vector never lines
/// up with the view direction (which would make the view basis degenerate).
const MAX_PITCH: f32 = 1.2;
/// Eye-distance clamp, in user units — kept well inside the renderer
/// camera's `near`/`far` range.
const MIN_DISTANCE: f32 = 1.0;
const MAX_DISTANCE: f32 = 50_000.0;

/// An orbit camera: yaw/pitch/distance around a document-centred target.
#[derive(Debug, Clone, Copy)]
pub struct OrbitCamera {
    /// Yaw around the world Y axis, in radians.
    yaw: f32,
    /// Pitch away from the document plane, in radians (clamped near the poles).
    pitch: f32,
    /// Eye distance from the target, in user units.
    distance: f32,
}

impl OrbitCamera {
    /// A camera framing a `viewport`-sized document head-on inside a viewport
    /// whose width/height ratio is `target_aspect`. The framing distance fits
    /// both the document's width and height in the visible region, so
    /// wide-but-short documents like a 440×140 banner are not horizontally
    /// cropped in a square canvas.
    pub fn framing(viewport: Viewport, target_aspect: f32) -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            distance: frame_distance(viewport, target_aspect),
        }
    }

    /// Reframe head-on for a (possibly new) document size and canvas aspect.
    pub fn reset(&mut self, viewport: Viewport, target_aspect: f32) {
        *self = Self::framing(viewport, target_aspect);
    }

    /// Orbit by `dyaw` / `dpitch` radians; pitch is clamped near the poles.
    pub fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Multiply the eye distance by `factor`, clamped to a sane range.
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Yaw, in radians — for the status read-out.
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Pitch, in radians — for the status read-out.
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Eye distance, in user units — for the status read-out.
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// Convert to a renderer [`Camera`] aimed at the document centre.
    pub fn to_camera(self, viewport: Viewport) -> Camera {
        let target = Vec3::new(viewport.width / 2.0, viewport.height / 2.0, 0.0);
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        // At yaw = pitch = 0 this is +Z, so the eye sits in front of the
        // document on the viewer side, matching `Camera::facing`. World Y
        // points down, so a positive pitch lifts the eye (toward -Y).
        let direction = Vec3::new(sin_yaw * cos_pitch, -sin_pitch, cos_yaw * cos_pitch);
        Camera {
            eye: target + direction * self.distance,
            target,
            fov_y: FOV_Y,
        }
    }
}

/// The eye distance at which the camera frames the whole document inside a
/// canvas of the given aspect ratio. Picks the larger of the height-fit and
/// width-fit distances so a wide-but-short document (e.g. 440×140) is not
/// horizontally cropped in a square or tall canvas.
fn frame_distance(viewport: Viewport, target_aspect: f32) -> f32 {
    let half_tan_y = (FOV_Y / 2.0).tan();
    let d_for_height = (viewport.height.max(1.0) / 2.0) / half_tan_y;
    // Horizontal half-FOV depends on the canvas aspect: tan(FOV_X/2) =
    // tan(FOV_Y/2) * (canvas_width / canvas_height). The width-fit distance is
    // the eye distance at which the document width spans the horizontal FOV
    // exactly. A zero or negative `target_aspect` is clamped so a degenerate
    // canvas doesn't divide by zero.
    let aspect = target_aspect.max(1e-3);
    let d_for_width = (viewport.width.max(1.0) / 2.0) / (half_tan_y * aspect);
    d_for_height.max(d_for_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport {
            width: 200.0,
            height: 100.0,
        }
    }

    /// Canvas aspect that's wide enough not to widen the framing — the
    /// document's 200×100 fits inside a 2:1 (or wider) canvas with the
    /// height-fit distance alone.
    const CANVAS_ASPECT: f32 = 2.0;

    #[test]
    fn framing_is_head_on() {
        let camera = OrbitCamera::framing(vp(), CANVAS_ASPECT).to_camera(vp());
        assert_eq!(camera.target, Vec3::new(100.0, 50.0, 0.0));
        assert!(camera.eye.z > 0.0, "eye should be on the +Z viewer side");
        assert!(
            (camera.eye.x - 100.0).abs() < 1e-4 && (camera.eye.y - 50.0).abs() < 1e-4,
            "head-on eye should be level with the document centre: {:?}",
            camera.eye,
        );
    }

    #[test]
    fn orbit_clamps_pitch_near_the_poles() {
        let mut camera = OrbitCamera::framing(vp(), CANVAS_ASPECT);
        camera.orbit(0.0, 100.0);
        assert!(camera.pitch() <= MAX_PITCH + 1e-6);
        camera.orbit(0.0, -100.0);
        assert!(camera.pitch() >= -MAX_PITCH - 1e-6);
    }

    #[test]
    fn zoom_clamps_distance() {
        let mut camera = OrbitCamera::framing(vp(), CANVAS_ASPECT);
        camera.zoom(1.0e9);
        assert!(camera.distance() <= MAX_DISTANCE);
        camera.zoom(1.0e-12);
        assert!(camera.distance() >= MIN_DISTANCE);
    }

    #[test]
    fn reset_restores_the_head_on_view() {
        let viewport = vp();
        let mut camera = OrbitCamera::framing(viewport, CANVAS_ASPECT);
        camera.orbit(1.0, 0.5);
        camera.zoom(4.0);
        camera.reset(viewport, CANVAS_ASPECT);
        assert_eq!(camera.yaw(), 0.0);
        assert_eq!(camera.pitch(), 0.0);
        assert_eq!(camera.distance(), frame_distance(viewport, CANVAS_ASPECT));
    }
}
