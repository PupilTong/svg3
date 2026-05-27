//! Orbit-style camera controller for the demo window.
//!
//! The eye orbits a target — the loaded document's centre — at a distance
//! the user can change. Yaw and pitch are spherical angles; at
//! `yaw = pitch = 0` the eye sits straight in front of the document on the
//! `+Z` (viewer) side, matching [`svg3::render::Camera::facing`].

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
    /// A camera framing a `viewport`-sized document head-on inside a window
    /// whose width/height ratio is `target_aspect`. The framing distance
    /// fits both the document's width and height in the visible region, so
    /// wide-but-short documents like a 440×140 banner are not horizontally
    /// cropped in a square window.
    pub fn framing(viewport: Viewport, target_aspect: f32) -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            distance: frame_distance(viewport, target_aspect),
        }
    }

    /// Reframe head-on for a (possibly new) document size and window aspect.
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
/// window of the given aspect ratio. Picks the larger of the height-fit and
/// width-fit distances so a wide-but-short document (e.g. 440×140) is not
/// horizontally cropped in a square or tall window.
fn frame_distance(viewport: Viewport, target_aspect: f32) -> f32 {
    let half_tan_y = (FOV_Y / 2.0).tan();
    let d_for_height = (viewport.height.max(1.0) / 2.0) / half_tan_y;
    // Horizontal half-FOV depends on the window aspect: tan(FOV_X/2) =
    // tan(FOV_Y/2) * (window_width / window_height). The width-fit distance
    // is the eye distance at which the document width spans the horizontal
    // FOV exactly. A zero or negative `target_aspect` is clamped so a
    // degenerate window doesn't divide by zero.
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

    /// Window aspect that's wide enough not to widen the framing — the
    /// document's 200×100 fits inside a 2:1 (or wider) window with the
    /// height-fit distance alone.
    const WINDOW_ASPECT: f32 = 2.0;

    #[test]
    fn framing_is_head_on() {
        // yaw = pitch = 0 -> eye straight in front, level with the centre.
        let camera = OrbitCamera::framing(vp(), WINDOW_ASPECT).to_camera(vp());
        assert_eq!(camera.target, Vec3::new(100.0, 50.0, 0.0));
        assert!(camera.eye.z > 0.0, "eye should be on the +Z viewer side");
        assert!(
            (camera.eye.x - 100.0).abs() < 1e-4 && (camera.eye.y - 50.0).abs() < 1e-4,
            "head-on eye should be level with the document centre: {:?}",
            camera.eye,
        );
    }

    #[test]
    fn framing_fits_width_in_narrow_window() {
        // A wide 400×100 document in a square (1:1) window: the height-fit
        // distance crops the width, so the width-fit distance must win.
        let banner = Viewport {
            width: 400.0,
            height: 100.0,
        };
        let camera = OrbitCamera::framing(banner, 1.0);
        let height_fit = (banner.height / 2.0) / (FOV_Y / 2.0).tan();
        assert!(
            camera.distance() > height_fit * 1.5,
            "wide doc in square window should zoom out past the height-fit distance: \
             d={}, height-fit={}",
            camera.distance(),
            height_fit,
        );
        // The width should fit exactly at the chosen distance (with a 1:1
        // window aspect, FOV_X == FOV_Y).
        let visible_width = 2.0 * camera.distance() * (FOV_Y / 2.0).tan();
        assert!(
            (visible_width - banner.width).abs() < 1e-2,
            "visible width {visible_width} should match document width {}",
            banner.width,
        );
    }

    #[test]
    fn orbit_clamps_pitch_near_the_poles() {
        let mut camera = OrbitCamera::framing(vp(), WINDOW_ASPECT);
        camera.orbit(0.0, 100.0);
        assert!(camera.pitch() <= MAX_PITCH + 1e-6);
        camera.orbit(0.0, -100.0);
        assert!(camera.pitch() >= -MAX_PITCH - 1e-6);
    }

    #[test]
    fn zoom_clamps_distance() {
        let mut camera = OrbitCamera::framing(vp(), WINDOW_ASPECT);
        camera.zoom(1.0e9);
        assert!(camera.distance() <= MAX_DISTANCE);
        camera.zoom(1.0e-12);
        assert!(camera.distance() >= MIN_DISTANCE);
    }

    #[test]
    fn orbit_swings_the_eye_but_keeps_the_distance() {
        let viewport = vp();
        let mut camera = OrbitCamera::framing(viewport, WINDOW_ASPECT);
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
        let mut camera = OrbitCamera::framing(viewport, WINDOW_ASPECT);
        camera.orbit(1.0, 0.5);
        camera.zoom(4.0);
        camera.reset(viewport, WINDOW_ASPECT);
        assert_eq!(camera.yaw(), 0.0);
        assert_eq!(camera.pitch(), 0.0);
        assert_eq!(camera.distance(), frame_distance(viewport, WINDOW_ASPECT));
    }
}
