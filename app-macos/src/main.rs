//! `svg3-macos` — native macOS demo window for svg3.
//!
//! Opens a Cocoa NSWindow via winit, accepts an SVG string through a macOS
//! dialog, and draws the currently implemented 2D shape geometry — including
//! referenced `<feGaussianBlur>` and PNG data-URL `<feImage>` filters — into
//! a Metal-backed wgpu surface through [`Renderer::encode_document`], the
//! same GPU path the headless renderer uses. The document is viewed through
//! an orbit camera the user can move: drag or the arrow keys to orbit, scroll
//! to zoom, `R` to reset.

mod camera;

use std::borrow::Cow;
use std::process::Command;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use svg3_dom::Document;
use svg3_render::{clear_target, document_viewport, RenderConfig, Renderer, Viewport};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use camera::OrbitCamera;

const APP_TITLE: &str = "svg3";
const EDIT_HINT: &str = "drag/arrows orbit · scroll zoom · R reset · Cmd+O edit";
const INITIAL_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(800.0, 600.0);
const MIN_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(320.0, 240.0);

/// Orbit sensitivity for a mouse drag, in radians per pixel.
const DRAG_ORBIT_SPEED: f32 = 0.005;
/// Orbit step applied per arrow-key press, in radians.
const KEY_ORBIT_STEP: f32 = 0.08;
/// Eye-distance multiplier applied per mouse-wheel line of scroll.
const WHEEL_ZOOM_STEP: f32 = 0.88;

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.96,
    g: 0.97,
    b: 0.98,
    a: 1.0,
};

const DEFAULT_SVG: &str = r##"<svg><rect x="40" y="40" width="220" height="130" fill="#2563eb"/><circle cx="245" cy="155" r="64" fill="#f97316"/><ellipse cx="395" cy="130" rx="62" ry="38" fill="#a855f7"/><polygon points="505,58 565,170 448,170" fill="#facc15"/><polyline points="400,70 575,220 355,220" fill="#22c55e"/><line x1="48" y1="220" x2="330" y2="70" stroke="#111827" stroke-width="10"/><path d="M 620 58 C 675 58 690 148 635 178 Q 595 148 620 58 Z" fill="#14b8a6" stroke="#0f172a" stroke-width="6"/><cube cx="490" cy="270" cz="0" size="90" fill="#ef4444"/></svg>"##;

/// GPU state, created once the window exists.
///
/// `Arc<Window>` + `Surface<'static>`: wgpu borrows the window handle for the
/// surface's lifetime and also retains the handle source internally. The `Arc`
/// keeps the window alive as long as the surface; no `unsafe` is required.
struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    /// Shared renderer: owns the wgpu device, queue and render pipelines.
    /// Drives every frame through [`Renderer::encode_document`] — the same
    /// GPU path the headless renderer uses.
    renderer: Renderer,
    config: wgpu::SurfaceConfiguration,
    /// The parsed document paired with the document viewport for percentage
    /// length resolution. `None` until the first successful parse.
    document: Option<(Document, Viewport)>,
    svg_source: String,
    /// The orbit camera the document is viewed through.
    camera: OrbitCamera,
    /// Last status message, kept so the window title can be rebuilt whenever
    /// the camera moves.
    status: String,
}

impl Gfx {
    fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .context("failed to create wgpu surface")?;

        let (device, queue, config) = pollster::block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    force_fallback_adapter: false,
                    compatible_surface: Some(&surface),
                })
                .await
                .context("no suitable GPU adapter")?;

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("svg3 device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    ..Default::default()
                })
                .await
                .context("failed to request wgpu device")?;

            let config = surface
                .get_default_config(&adapter, width, height)
                .context("surface not supported by adapter")?;

            anyhow::Ok((device, queue, config))
        })?;

        surface.configure(&device, &config);
        let renderer = Renderer::with_device(device, queue, config.format);

        let mut gfx = Self {
            window,
            surface,
            renderer,
            config,
            document: None,
            svg_source: String::new(),
            camera: OrbitCamera::framing(Viewport {
                width: width as f32,
                height: height as f32,
            }),
            status: String::new(),
        };
        gfx.set_svg_source(DEFAULT_SVG.to_owned());
        Ok(gfx)
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        let width = size.width.max(1);
        let height = size.height.max(1);
        if width == self.config.width && height == self.config.height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(self.renderer.device(), &self.config);
        // A resize changes the surface dimensions percentage lengths resolve
        // against. The parsed document stays valid; only the viewport needs
        // refreshing so the per-frame walk sees the new size.
        self.refresh_viewport();
        self.window.request_redraw();
    }

    fn render(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.surface.configure(self.renderer.device(), &self.config);
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(self.renderer.device(), &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => {
                log::debug!("surface frame skipped (transient state)");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.renderer
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("svg3 frame encoder"),
                });
        clear_target(&mut encoder, &view, CLEAR_COLOR, "svg3 frame clear pass");
        if let Some((document, viewport)) = &self.document {
            self.renderer.encode_document(
                document,
                *viewport,
                self.view_projection(*viewport),
                &view,
                self.target_extent(),
                &mut encoder,
            );
        }
        self.renderer
            .queue()
            .submit(std::iter::once(encoder.finish()));
        frame.present();
    }

    fn set_svg_source(&mut self, source: String) {
        self.svg_source = source;
        match svg3_dom::parse(&self.svg_source) {
            Ok(document) => {
                let viewport = document_viewport(&document, self.target_viewport());
                // A freshly loaded document gets a head-on framing.
                self.camera.reset(viewport);
                self.document = Some((document, viewport));
                self.set_status("rendered");
            }
            Err(e) => {
                self.document = None;
                self.set_status(&format!("parse error: {e}"));
                log::error!("failed to parse SVG input: {e}");
            }
        }
        self.window.request_redraw();
    }

    /// Recompute the document viewport from the current target size.
    ///
    /// Called when the surface resizes — the parsed document is unchanged
    /// but percentage lengths track the new viewport.
    fn refresh_viewport(&mut self) {
        let Some((document, viewport)) = self.document.as_mut() else {
            return;
        };
        *viewport = document_viewport(
            document,
            Viewport {
                width: self.config.width.max(1) as f32,
                height: self.config.height.max(1) as f32,
            },
        );
    }

    /// The render target viewed as a [`Viewport`], used as the fallback for
    /// `<svg>` root sizing.
    fn target_viewport(&self) -> Viewport {
        Viewport {
            width: self.config.width.max(1) as f32,
            height: self.config.height.max(1) as f32,
        }
    }

    /// The render target's wgpu extent, used to size offscreen filter
    /// textures inside [`Renderer::encode_document`].
    fn target_extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.config.width.max(1),
            height: self.config.height.max(1),
            depth_or_array_layers: 1,
        }
    }

    /// The view-projection matrix for the current camera framing `viewport`.
    fn view_projection(&self, viewport: Viewport) -> glam::Mat4 {
        RenderConfig {
            format: self.config.format,
            width: self.config.width.max(1),
            height: self.config.height.max(1),
            camera: Some(self.camera.to_camera(viewport)),
        }
        .view_projection()
    }

    /// Orbit the camera by `dyaw` / `dpitch` radians and redraw.
    fn orbit_camera(&mut self, dyaw: f32, dpitch: f32) {
        self.camera.orbit(dyaw, dpitch);
        self.window.request_redraw();
        self.refresh_title();
    }

    /// Multiply the camera's eye distance by `factor` and redraw.
    fn zoom_camera(&mut self, factor: f32) {
        self.camera.zoom(factor);
        self.window.request_redraw();
        self.refresh_title();
    }

    /// Reframe the camera head-on for the current document.
    fn reset_camera(&mut self) {
        let Some(viewport) = self.document.as_ref().map(|(_, viewport)| *viewport) else {
            return;
        };
        self.camera.reset(viewport);
        self.window.request_redraw();
        self.refresh_title();
    }

    fn set_status(&mut self, status: &str) {
        self.status = compact_status(status, 80).into_owned();
        self.refresh_title();
    }

    /// Rebuild the window title from the current status and camera state.
    fn refresh_title(&self) {
        let camera = format!(
            "cam {:.0}°/{:.0}° d{:.0}",
            self.camera.yaw().to_degrees(),
            self.camera.pitch().to_degrees(),
            self.camera.distance(),
        );
        self.window.set_title(&format!(
            "{APP_TITLE} - {} - {camera} ({EDIT_HINT})",
            self.status
        ));
    }
}

struct App {
    gfx: Option<Gfx>,
    modifiers: ModifiersState,
    /// Last cursor position, for computing orbit-drag deltas.
    cursor: Option<(f64, f64)>,
    /// Whether the left button is held — an orbit drag is in progress.
    orbiting: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            gfx: None,
            modifiers: ModifiersState::empty(),
            cursor: None,
            orbiting: false,
        }
    }
}

impl App {
    fn open_svg_dialog(&mut self) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        match prompt_for_svg(&gfx.svg_source) {
            Ok(Some(source)) => gfx.set_svg_source(source),
            Ok(None) => {}
            Err(e) => {
                log::error!("{e:#}");
                gfx.set_status("input dialog failed");
                gfx.window.request_redraw();
            }
        }
    }

    /// Apply a camera control key (arrow keys orbit, `R` resets).
    fn handle_camera_key(&mut self, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::ArrowLeft) => gfx.orbit_camera(-KEY_ORBIT_STEP, 0.0),
            Key::Named(NamedKey::ArrowRight) => gfx.orbit_camera(KEY_ORBIT_STEP, 0.0),
            Key::Named(NamedKey::ArrowUp) => gfx.orbit_camera(0.0, -KEY_ORBIT_STEP),
            Key::Named(NamedKey::ArrowDown) => gfx.orbit_camera(0.0, KEY_ORBIT_STEP),
            Key::Character(key) if key.eq_ignore_ascii_case("r") && !event.repeat => {
                gfx.reset_camera();
            }
            _ => {}
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("svg3")
            .with_inner_size(INITIAL_WINDOW_SIZE)
            .with_min_inner_size(MIN_WINDOW_SIZE);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };
        match Gfx::new(window) {
            Ok(gfx) => {
                self.gfx = Some(gfx);
                self.open_svg_dialog();
                if let Some(gfx) = &self.gfx {
                    gfx.window.request_redraw();
                }
            }
            Err(e) => {
                log::error!("failed to initialize graphics: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.resize(size);
                    gfx.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.render();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. }
                if is_svg_input_shortcut(&event, self.modifiers) =>
            {
                self.open_svg_dialog();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_camera_key(&event);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                // A left drag orbits the camera; the SVG dialog opens via
                // Command+O / Command+I instead.
                self.orbiting = state == ElementState::Pressed;
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.orbiting = false;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let next = (position.x, position.y);
                if let (true, Some((px, py)), Some(gfx)) =
                    (self.orbiting, self.cursor, self.gfx.as_mut())
                {
                    let dyaw = (next.0 - px) as f32 * DRAG_ORBIT_SPEED;
                    let dpitch = (next.1 - py) as f32 * DRAG_ORBIT_SPEED;
                    gfx.orbit_camera(dyaw, dpitch);
                }
                self.cursor = Some(next);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 60.0,
                };
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.zoom_camera(WHEEL_ZOOM_STEP.powf(lines));
                }
            }
            _ => {}
        }
    }
}

fn is_svg_input_shortcut(event: &winit::event::KeyEvent, modifiers: ModifiersState) -> bool {
    if event.state != ElementState::Pressed || event.repeat || !modifiers.super_key() {
        return false;
    }
    match event.logical_key.as_ref() {
        Key::Character(key) => key.eq_ignore_ascii_case("o") || key.eq_ignore_ascii_case("i"),
        _ => false,
    }
}

fn prompt_for_svg(default_source: &str) -> Result<Option<String>> {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(format!(
            "set promptText to {}",
            apple_script_string(
                "Paste an SVG string. This demo currently renders <rect>, <circle>, <ellipse>, <polygon>, filled <polyline>, <line>, and <path> elements, including referenced <filter> definitions with <feGaussianBlur/> and PNG data-URL <feImage/>."
            )
        ))
        .arg("-e")
        .arg(format!(
            "set defaultSvg to {}",
            apple_script_string(default_source)
        ))
        .arg("-e")
        .arg("set dialogResult to display dialog promptText default answer defaultSvg buttons {\"Cancel\", \"Render\"} default button \"Render\" cancel button \"Cancel\" with title \"svg3\"")
        .arg("-e")
        .arg("text returned of dialogResult")
        .output()
        .context("failed to run macOS SVG input dialog")?;

    if output.status.success() {
        let source =
            String::from_utf8(output.stdout).context("SVG input dialog returned non-UTF-8 text")?;
        return Ok(Some(source.trim_end_matches(['\n', '\r']).to_owned()));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("User canceled") || stderr.contains("(-128)") {
        return Ok(None);
    }
    Err(anyhow!("SVG input dialog failed: {}", stderr.trim()))
}

fn apple_script_string(value: &str) -> String {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        match ch {
            '"' => current.push_str("\\\""),
            '\\' => current.push_str("\\\\"),
            '\n' => {
                push_apple_script_literal_part(&mut parts, &mut current);
                parts.push("linefeed".to_owned());
            }
            '\r' => {
                push_apple_script_literal_part(&mut parts, &mut current);
                parts.push("return".to_owned());
            }
            _ => current.push(ch),
        }
    }
    push_apple_script_literal_part(&mut parts, &mut current);
    if parts.is_empty() {
        "\"\"".to_owned()
    } else {
        parts.join(" & ")
    }
}

fn push_apple_script_literal_part(parts: &mut Vec<String>, current: &mut String) {
    if current.is_empty() {
        return;
    }
    parts.push(format!("\"{current}\""));
    current.clear();
}

fn compact_status(status: &str, max_chars: usize) -> Cow<'_, str> {
    if status.chars().count() <= max_chars {
        return Cow::Borrowed(status);
    }
    let mut compact = status
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    compact.push('…');
    Cow::Owned(compact)
}

fn main() -> Result<()> {
    env_logger::init();
    println!(
        "svg3-macos camera controls: drag or arrow keys orbit · scroll \
         zooms · R resets · Command+O / Command+I edits the SVG"
    );
    let event_loop = EventLoop::new().context("failed to create event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::default();
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_script_string_escapes_svg_text() {
        let source = "<svg>\n<rect fill=\"#fff\\000\"/>\r</svg>";
        assert_eq!(
            apple_script_string(source),
            "\"<svg>\" & linefeed & \"<rect fill=\\\"#fff\\\\000\\\"/>\" & return & \"</svg>\""
        );
    }

    #[test]
    fn compact_status_keeps_short_strings_borrowed() {
        assert!(matches!(compact_status("rendered", 100), Cow::Borrowed(_)));
    }

    #[test]
    fn compact_status_truncates_long_strings() {
        assert_eq!(
            compact_status("abcdef", 4),
            Cow::<str>::Owned("abc…".to_owned())
        );
    }
}
