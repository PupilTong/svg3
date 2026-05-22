//! `svg3-macos` — native macOS demo window for svg3.
//!
//! Opens a Cocoa NSWindow via winit, accepts an SVG string through a macOS
//! dialog, and draws the currently implemented 2D shape geometry
//! into a Metal-backed wgpu surface. The document is viewed through an orbit
//! camera the user can move: drag or the arrow keys to orbit, scroll to
//! zoom, `R` to reset.

mod camera;

use std::borrow::Cow;
use std::process::Command;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use svg3_render::{build_scene, document_viewport, RenderConfig, Vertex, Viewport};
use wgpu::util::DeviceExt;
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

const DEFAULT_SVG: &str = r##"<svg><rect x="40" y="40" width="220" height="130" fill="#2563eb"/><circle cx="245" cy="155" r="64" fill="#f97316"/><ellipse cx="395" cy="130" rx="62" ry="38" fill="#a855f7"/><polygon points="505,58 565,170 448,170" fill="#facc15"/><polyline points="400,70 575,220 355,220" fill="#22c55e"/></svg>"##;

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

/// A built scene: object-space geometry buffers plus the camera transform
/// uniform. The vertex and index buffers are stable until the SVG itself
/// changes; camera moves only rewrite the uniform buffer.
struct DrawScene {
    /// The document viewport the camera frames and targets.
    viewport: Viewport,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    transform_buffer: wgpu::Buffer,
    transform_bind_group: wgpu::BindGroup,
    index_count: u32,
}

/// GPU state, created once the window exists.
///
/// `Arc<Window>` + `Surface<'static>`: wgpu borrows the window handle for the
/// surface's lifetime and also retains the handle source internally. The `Arc`
/// keeps the window alive as long as the surface; no `unsafe` is required.
struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    scene: Option<DrawScene>,
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
        let pipeline = build_pipeline(&device, config.format);

        let mut gfx = Self {
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            scene: None,
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
        self.surface.configure(&self.device, &self.config);
        self.rebuild_scene();
    }

    fn render(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.surface.configure(&self.device, &self.config);
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        if let Some(scene) = &self.scene {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shape pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &scene.transform_bind_group, &[]);
            pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
            pass.set_index_buffer(scene.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..scene.index_count, 0, 0..1);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }

    fn set_svg_source(&mut self, source: String) {
        self.svg_source = source;
        match svg3_dom::parse(&self.svg_source) {
            Ok(document) => {
                self.scene = self.build_draw_scene(&document);
                match self.scene.as_ref().map(|scene| scene.viewport) {
                    Some(viewport) => {
                        // A freshly loaded document gets a head-on framing.
                        self.camera.reset(viewport);
                        self.reproject();
                        self.set_status("rendered");
                    }
                    None => self.set_status("no supported shapes"),
                }
            }
            Err(e) => {
                self.scene = None;
                self.set_status(&format!("parse error: {e}"));
                log::error!("failed to parse SVG input: {e}");
            }
        }
        self.window.request_redraw();
    }

    fn rebuild_scene(&mut self) {
        // A resize re-tessellates (percentage lengths track the viewport)
        // but keeps the camera where the user left it.
        self.scene = match svg3_dom::parse(&self.svg_source) {
            Ok(document) => self.build_draw_scene(&document),
            Err(_) => None,
        };
        self.reproject();
        self.window.request_redraw();
    }

    /// Tessellate `document` into an object-space [`DrawScene`].
    fn build_draw_scene(&self, document: &svg3_dom::Document) -> Option<DrawScene> {
        let width = self.config.width.max(1);
        let height = self.config.height.max(1);
        let target_viewport = Viewport {
            width: width as f32,
            height: height as f32,
        };
        let viewport = document_viewport(document, target_viewport);
        let mesh = build_scene(document, viewport);
        if mesh.is_empty() {
            return None;
        }

        let render_config = RenderConfig {
            format: self.config.format,
            width,
            height,
            camera: Some(self.camera.to_camera(viewport)),
        };
        let (transform_buffer, transform_bind_group) =
            build_transform_bind_group(&self.device, &self.pipeline, render_config);
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 app vertex buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 app index buffer"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let index_count = mesh.indices.len() as u32;
        Some(DrawScene {
            viewport,
            vertex_buffer,
            index_buffer,
            transform_buffer,
            transform_bind_group,
            index_count,
        })
    }

    /// Upload the current camera transform. Cheap enough to call on every
    /// camera or surface change.
    fn reproject(&self) {
        let Some(scene) = &self.scene else {
            return;
        };
        let view_projection = RenderConfig {
            format: self.config.format,
            width: self.config.width.max(1),
            height: self.config.height.max(1),
            camera: Some(self.camera.to_camera(scene.viewport)),
        }
        .view_projection();
        let transform = view_projection.to_cols_array_2d();
        self.queue
            .write_buffer(&scene.transform_buffer, 0, bytemuck::cast_slice(&transform));
        self.window.request_redraw();
    }

    /// Orbit the camera by `dyaw` / `dpitch` radians and re-project.
    fn orbit_camera(&mut self, dyaw: f32, dpitch: f32) {
        self.camera.orbit(dyaw, dpitch);
        self.reproject();
        self.refresh_title();
    }

    /// Multiply the camera's eye distance by `factor` and re-project.
    fn zoom_camera(&mut self, factor: f32) {
        self.camera.zoom(factor);
        self.reproject();
        self.refresh_title();
    }

    /// Reframe the camera head-on for the current document.
    fn reset_camera(&mut self) {
        let Some(viewport) = self.scene.as_ref().map(|scene| scene.viewport) else {
            return;
        };
        self.camera.reset(viewport);
        self.reproject();
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

fn build_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("svg3 app shader"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
            "../../svg3-render/src/shader.wgsl"
        ))),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("svg3 app shape pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &VERTEX_ATTRIBUTES,
            }],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn build_transform_bind_group(
    device: &wgpu::Device,
    pipeline: &wgpu::RenderPipeline,
    config: RenderConfig,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let view_projection = config.view_projection().to_cols_array_2d();
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("svg3 app transform uniform"),
        contents: bytemuck::cast_slice(&view_projection),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("svg3 app transform bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (buffer, bind_group)
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
                "Paste an SVG string. This demo currently renders <rect>, <circle>, <ellipse>, <polygon>, and filled <polyline> elements."
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
