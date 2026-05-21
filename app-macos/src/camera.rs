//! Orbit-style camera controller for the demo window.
//!
//! The eye orbits a target — the loaded document's centre — at a distance
//! the user can change. Yaw and pitch are spherical angles; at
//! `yaw = pitch = 0` the eye sits straight in front of the document on the
//! `+Z` (viewer) side, matching [`svg3_render::Camera::facing`].

use glam::Vec3;
use svg3_render::{Camera, Viewport};

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
    /// A camera framing a `viewport`-sized document head-on.
    pub fn framing(viewport: Viewport) -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            distance: frame_distance(viewport),
        }
    }

    /// Reframe head-on for a (possibly new) document size.
    pub fn reset(&mut self, viewport: Viewport) {
        *self = Self::framing(viewport);
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

/// The eye distance at which a `FOV_Y` vertical field of view spans the
/// document height exactly — the head-on framing distance.
fn frame_distance(viewport: Viewport) -> f32 {
    (viewport.height.max(1.0) / 2.0) / (FOV_Y / 2.0).tan()
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

    #[test]
    fn framing_is_head_on() {
        // yaw = pitch = 0 -> eye straight in front, level with the centre.
        let camera = OrbitCamera::framing(vp()).to_camera(vp());
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
        let mut camera = OrbitCamera::framing(vp());
        camera.orbit(0.0, 100.0);
        assert!(camera.pitch() <= MAX_PITCH + 1e-6);
        camera.orbit(0.0, -100.0);
        assert!(camera.pitch() >= -MAX_PITCH - 1e-6);
    }

    #[test]
    fn zoom_clamps_distance() {
        let mut camera = OrbitCamera::framing(vp());
        camera.zoom(1.0e9);
        assert!(camera.distance() <= MAX_DISTANCE);
        camera.zoom(1.0e-12);
        assert!(camera.distance() >= MIN_DISTANCE);
    }

    #[test]
    fn orbit_swings_the_eye_but_keeps_the_distance() {
        let viewport = vp();
        let mut camera = OrbitCamera::framing(viewport);
        let target = camera.to_camera(viewport).target;
        let front = camera.to_camera(viewport).eye;
        camera.orbit(0.7, 0.0);
        let orbited = camera.to_camera(viewport).eye;
        assert!(
            (front.x - orbited.x).abs() > 1.0,
            "a yaw orbit should swing the eye sideways"
        );
        let before = (front - target).length();
        let after = (orbited - target).length();
        assert!(
            (before - after).abs() < 1e-3,
            "orbit must preserve distance"
        );
    }

    #[test]
    fn reset_restores_the_head_on_view() {
        let viewport = vp();
        let mut camera = OrbitCamera::framing(viewport);
        camera.orbit(1.0, 0.5);
        camera.zoom(4.0);
        camera.reset(viewport);
        assert_eq!(camera.yaw(), 0.0);
        assert_eq!(camera.pitch(), 0.0);
        assert_eq!(camera.distance(), frame_distance(viewport));
    }
}
