//! `svg3-web` — the browser rendering engine behind the GitHub Pages app.
//!
//! Compiles the cross-platform [`svg3`] renderer to WebAssembly and drives it
//! through the same GPU path the native `app-macos` demo uses
//! ([`Renderer::with_device`] + [`Renderer::encode_document`]). Rather than
//! present a wgpu surface — WebGPU surface presentation does not composite for a
//! worker-owned `OffscreenCanvas` — it renders each frame into an offscreen
//! sRGB texture and hands the pixels back to the worker, which blits them onto
//! the `OffscreenCanvas` through a 2D context. Both the GPU render and the
//! canvas draw happen on the worker thread.
//!
//! [`WebRenderer`] is the browser analogue of `app-macos`'s `Gfx`: it owns the
//! device/queue/pipelines, the parsed document and the orbit camera, and
//! exposes an imperative API the worker calls in response to UI messages.
//!
//! The whole crate is gated to `wasm32` so the workspace's host jobs compile it
//! to an empty library instead of pulling `wasm-bindgen` into a native build.
#![cfg(target_arch = "wasm32")]

mod camera;

use camera::OrbitCamera;
use svg3::dom::{self, Document};
use svg3::render::{clear_target, document_viewport, RenderConfig, Renderer, Viewport};
use wasm_bindgen::prelude::*;

// `getrandom` is enabled transitively (wgpu → naga/ahash); referencing the
// feature-bearing dependencies as `_` switches on the browser backend without
// tripping unused-dependency lints. See ../.cargo/config.toml.
use getrandom_v02 as _;
use getrandom_v03 as _;

/// Render-target format. sRGB `RGBA8` so the read-back bytes are exactly what a
/// 2D canvas `ImageData` expects — the worker can `putImageData` them directly.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Canvas background, matching `app-macos`'s near-white clear colour.
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.96,
    g: 0.97,
    b: 0.98,
    a: 1.0,
};

/// Module init (runs when the worker awaits `init()`): install a panic hook
/// that logs to the browser console, and route `log` records there too.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Warn);
}

/// The browser render engine: owns the GPU, the parsed document and the orbit
/// camera. Lives entirely on the worker thread. Mutating calls update state;
/// [`render_frame`](WebRenderer::render_frame) renders the current state into a
/// fresh RGBA8 buffer the worker paints onto the canvas.
#[wasm_bindgen]
pub struct WebRenderer {
    /// Shared renderer: owns the wgpu device, queue and pipelines, and drives
    /// every frame through [`Renderer::encode_document`].
    renderer: Renderer,
    /// Current target size, in physical (device) pixels.
    width: u32,
    height: u32,
    /// Parsed document paired with its percentage-resolution viewport. `None`
    /// until the first successful parse (and after a parse error).
    document: Option<(Document, Viewport)>,
    /// The orbit camera 3D documents are viewed through.
    camera: OrbitCamera,
    /// Last status line ("rendered" or "parse error: …"), relayed to the UI.
    status: String,
}

#[wasm_bindgen]
impl WebRenderer {
    /// Bring up a headless wgpu device for `width`×`height` (physical pixels).
    /// Async because adapter/device acquisition is async on the web (no
    /// `pollster` — that path is only for the native headless renderer).
    ///
    /// Returns a rejected promise (a `JsValue` error string) when WebGPU is
    /// unavailable or no adapter/device can be acquired; the worker relays that
    /// to the UI as a fatal error.
    pub async fn create(width: u32, height: u32) -> Result<WebRenderer, JsValue> {
        let width = width.max(1);
        let height = height.max(1);

        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|e| JsValue::from_str(&format!("no suitable GPU adapter: {e}")))?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("svg3 web device"),
                required_features: wgpu::Features::empty(),
                // Request exactly what the adapter exposes so the device
                // request never fails on a limit mismatch across browsers.
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .map_err(|e| JsValue::from_str(&format!("failed to request wgpu device: {e}")))?;

        // Surface any pipeline/shader validation error as a fatal init error
        // rather than letting it silently invalidate the first frame. (Browser
        // WGSL validation is stricter than native naga, so this is where a
        // shader incompatibility would show up.)
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let renderer = Renderer::with_device(device, queue, TARGET_FORMAT);
        if let Some(error) = scope.pop().await {
            return Err(JsValue::from_str(&format!(
                "renderer pipeline build failed: {error}"
            )));
        }
        let camera = OrbitCamera::framing(
            Viewport {
                width: width as f32,
                height: height as f32,
            },
            width as f32 / height as f32,
        );

        Ok(WebRenderer {
            renderer,
            width,
            height,
            document: None,
            camera,
            status: String::new(),
        })
    }

    /// Parse and display `source`, returning a status line for the UI. A parse
    /// error clears the document and reports the message rather than throwing.
    /// Does not render — the worker calls [`render_frame`](Self::render_frame).
    pub fn set_source(&mut self, source: &str) -> String {
        match dom::parse(source) {
            Ok(document) => {
                let viewport = document_viewport(&document, self.target_viewport());
                // A freshly loaded document gets a head-on framing.
                self.camera.reset(viewport, self.target_aspect());
                self.document = Some((document, viewport));
                self.status = "rendered".to_owned();
            }
            Err(e) => {
                self.document = None;
                self.status = format!("parse error: {e}");
                log::warn!("svg3-web: {}", self.status);
            }
        }
        self.status.clone()
    }

    /// Orbit the camera by `dyaw` / `dpitch` radians. (No visible effect on flat
    /// 2D documents, which render through the orthographic projection.)
    pub fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.camera.orbit(dyaw, dpitch);
    }

    /// Multiply the camera's eye distance by `factor`.
    pub fn zoom(&mut self, factor: f32) {
        self.camera.zoom(factor);
    }

    /// Reframe the camera head-on for the current document.
    pub fn reset(&mut self) {
        if let Some((_, viewport)) = self.document.as_ref().map(|(d, v)| (d, *v)) {
            self.camera.reset(viewport, self.target_aspect());
        }
    }

    /// React to a canvas resize (physical pixels): track the new size and
    /// refresh the percentage-resolution viewport. The worker keeps the canvas
    /// backing store matched to this size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.refresh_viewport();
    }

    /// Render the current document + camera into a fresh row-major RGBA8 buffer
    /// (`4 * width * height` bytes, sRGB, premultiplied-free) the worker paints
    /// onto the canvas via `putImageData`.
    ///
    /// Async: after submitting the GPU work it awaits the buffer map (the web
    /// has no blocking `device.poll(Wait)`); the browser drives completion.
    pub async fn render_frame(&self) -> Result<Vec<u8>, JsValue> {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("svg3 web target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bytes_per_row = width * 4;
        let padded_bytes_per_row = bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("svg3 web readback"),
            size: padded_bytes_per_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("svg3 web frame encoder"),
        });
        clear_target(&mut encoder, &view, CLEAR_COLOR, "svg3 web clear pass");
        if let Some((document, viewport)) = &self.document {
            self.renderer.encode_document(
                document,
                *viewport,
                self.view_projection(document, *viewport),
                &view,
                extent,
                &mut encoder,
            );
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        queue.submit(std::iter::once(encoder.finish()));

        // Bridge the map_async callback to an awaitable future. On the web the
        // browser drives the GPU queue, so awaiting yields until the map is
        // ready — no blocking `device.poll`.
        let (sender, receiver) = futures_channel::oneshot::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        receiver
            .await
            .map_err(|_| JsValue::from_str("readback map canceled"))?
            .map_err(|e| JsValue::from_str(&format!("readback map failed: {e:?}")))?;

        let row_bytes = (width * 4) as usize;
        let mut pixels = Vec::with_capacity(row_bytes * height as usize);
        {
            let mapped = readback.slice(..).get_mapped_range();
            for row in 0..height as usize {
                let start = row * padded_bytes_per_row as usize;
                pixels.extend_from_slice(&mapped[start..start + row_bytes]);
            }
        }
        readback.unmap();
        Ok(pixels)
    }

    /// Camera yaw in radians — relayed to the UI status read-out.
    pub fn camera_yaw(&self) -> f32 {
        self.camera.yaw()
    }

    /// Camera pitch in radians.
    pub fn camera_pitch(&self) -> f32 {
        self.camera.pitch()
    }

    /// Camera eye distance in user units.
    pub fn camera_distance(&self) -> f32 {
        self.camera.distance()
    }
}

impl WebRenderer {
    /// The render target viewed as a [`Viewport`], the fallback for root
    /// `<svg>` sizing when the document omits `width`/`height`.
    fn target_viewport(&self) -> Viewport {
        Viewport {
            width: self.width.max(1) as f32,
            height: self.height.max(1) as f32,
        }
    }

    /// The render target's width / height ratio, for framing the document
    /// inside the canvas's horizontal field of view.
    fn target_aspect(&self) -> f32 {
        self.width.max(1) as f32 / self.height.max(1) as f32
    }

    /// Recompute the document viewport from the current target size (after a
    /// resize): the parsed document is unchanged, only percentage lengths track
    /// the new size.
    fn refresh_viewport(&mut self) {
        let target = self.target_viewport();
        if let Some((document, viewport)) = self.document.as_mut() {
            *viewport = document_viewport(document, target);
        }
    }

    /// The view-projection matrix for the current document and viewport: the
    /// orbit camera's perspective for 3D documents, the flat orthographic
    /// projection for plain SVG — exactly as `app-macos` chooses.
    fn view_projection(&self, document: &Document, viewport: Viewport) -> glam::Mat4 {
        let config = RenderConfig {
            format: TARGET_FORMAT,
            width: self.width.max(1),
            height: self.height.max(1),
            camera: Some(self.camera.to_camera(viewport)),
        };
        if document.svg3_extension_enabled() {
            config.view_projection()
        } else {
            config.projection()
        }
    }
}
