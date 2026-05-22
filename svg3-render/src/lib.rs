//! `svg3-render` — GPU rendering for SVG3 scenes.
//!
//! Turns a parsed [`svg3_dom`] document into GPU geometry and rasterises it
//! with [wgpu](https://crates.io/crates/wgpu).
//!
//! This milestone implements the SVG 1.1 `<rect>`, `<circle>` and
//! `<ellipse>` basic shapes: [`build_scene`] tessellates every such shape
//! in a document into a [`Mesh`], and [`Renderer::render_to_image`]
//! rasterises that mesh
//! headlessly — no window or swapchain — into an [`Image`]. Basic shapes
//! are two-dimensional, so geometry lies in the world plane `z = 0`: by
//! default it is drawn flat through the orthographic
//! [`RenderConfig::projection`], but an optional [`Camera`] on
//! [`RenderConfig`] instead views that plane through a movable 3D
//! perspective camera. The root `<svg width>` / `<svg height>` set the
//! default viewport for percentage lengths; when either is omitted or
//! invalid, the render target dimension is used.

mod circle;
mod ellipse;
mod rect;
mod shape;

pub use shape::Viewport;

use glam::{Mat4, Vec3};
use svg3_dom::{Document, ElementKind};
use thiserror::Error;
use wgpu::util::DeviceExt;

/// Texture format the headless renderer draws into. sRGB-encoded so linear
/// vertex colours are stored correctly; read back as `RGBA8`.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Vertex buffer layout: object-space position then linear RGBA colour.
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

/// A single GPU vertex: position + linear RGBA colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Linear RGBA colour in `[0, 1]`.
    pub color: [f32; 4],
}

/// Uniform data consumed by `shader.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TransformUniform {
    /// Column-major view-projection matrix, matching WGSL matrix layout.
    view_projection: [[f32; 4]; 4],
}

impl TransformUniform {
    fn new(view_projection: Mat4) -> Self {
        Self {
            view_projection: view_projection.to_cols_array_2d(),
        }
    }
}

/// Target-surface configuration for a render pass.
#[derive(Debug, Clone, Copy)]
pub struct RenderConfig {
    /// Swapchain / texture format to render into. Reserved for the future
    /// windowed path; [`Renderer::render_to_image`] always targets an
    /// sRGB `RGBA8` texture.
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

/// A combined triangle mesh: a vertex list plus a triangle index list.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    /// Vertices, in SVG user space (origin top-left, y-down, `z = 0`).
    pub vertices: Vec<Vertex>,
    /// Triangle indices into [`Mesh::vertices`].
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Append `other`'s geometry, offsetting its indices to stay valid.
    fn append(&mut self, other: Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
    }

    /// Whether the mesh contains no triangles.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// An RGBA8 image produced by a headless render.
#[derive(Debug, Clone)]
pub struct Image {
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
    /// Row-major `RGBA8` pixels: `4 * width * height` bytes, no row padding.
    pub pixels: Vec<u8>,
}

impl Image {
    /// The `RGBA8` bytes of the pixel at `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics if `(x, y)` is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height, "pixel out of bounds");
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }
}

/// Walk `document` and tessellate every `<rect>`, `<circle>` and `<ellipse>`
/// into one combined [`Mesh`].
///
/// `viewport` is the basis for percentage lengths (e.g. `width="100%"`);
/// callers that want SVG root sizing should pass [`document_viewport`]. The
/// mesh is in SVG user space (origin top-left, y-down, `z = 0`). Shapes are
/// appended in document order, so a later shape paints over an earlier one. A
/// shape that is not rendered — a degenerate size, or `fill="none"` —
/// contributes nothing. `transform` and grouping are not applied yet, so a
/// shape is placed at its own coordinates regardless of any ancestor `<g>`.
pub fn build_scene(document: &Document, viewport: Viewport) -> Mesh {
    let mut mesh = Mesh::default();
    // Pre-order DFS; children pushed in reverse so they pop in document
    // order, giving the painter's-algorithm draw order.
    let mut stack = vec![document.root()];
    while let Some(id) = stack.pop() {
        let node = document.node(id);
        match &node.element.kind {
            ElementKind::Rect => {
                if let (Some(geo), Some(color)) = (
                    rect::resolve_rect(&node.element, viewport),
                    shape::resolve_fill(&node.element),
                ) {
                    mesh.append(rect::tessellate_rect(&geo, color));
                }
            }
            ElementKind::Circle => {
                if let (Some(geo), Some(color)) = (
                    circle::resolve_circle(&node.element, viewport),
                    shape::resolve_fill(&node.element),
                ) {
                    mesh.append(circle::tessellate_circle(&geo, color));
                }
            }
            ElementKind::Ellipse => {
                if let (Some(geo), Some(color)) = (
                    ellipse::resolve_ellipse(&node.element, viewport),
                    shape::resolve_fill(&node.element),
                ) {
                    mesh.append(ellipse::tessellate_ellipse(&geo, color));
                }
            }
            _ => {}
        }
        stack.extend(node.children.iter().rev().copied());
    }
    mesh
}

/// Resolve the document viewport from the root `<svg width>` / `<svg height>`.
///
/// Missing, unparseable, or non-positive dimensions fall back to `fallback`.
/// Percentage dimensions resolve against `fallback`, matching SVG's default
/// `100%` sizing behavior for a standalone document.
pub fn document_viewport(document: &Document, fallback: Viewport) -> Viewport {
    let root = document.element(document.root());
    Viewport {
        width: root
            .attributes
            .get("width")
            .and_then(|value| shape::Length::parse(value))
            .map(|length| length.resolve(fallback.width))
            .filter(|value| *value > 0.0)
            .unwrap_or(fallback.width),
        height: root
            .attributes
            .get("height")
            .and_then(|value| shape::Length::parse(value))
            .map(|length| length.resolve(fallback.height))
            .filter(|value| *value > 0.0)
            .unwrap_or(fallback.height),
    }
}

/// Renders SVG3 documents to GPU images.
#[derive(Debug, Default)]
pub struct Renderer;

impl Renderer {
    /// Create a new renderer.
    pub fn new() -> Self {
        Self
    }

    /// Render every `<rect>`, `<circle>` and `<ellipse>` in `document`
    /// headlessly into an [`Image`] of `config.width × config.height` pixels.
    ///
    /// Brings up a wgpu device with no surface, rasterises the tessellated
    /// scene into an offscreen sRGB texture, and reads the pixels back. The
    /// surface is cleared to transparent before drawing.
    ///
    /// Returns [`RenderError::NoAdapter`] when the machine exposes no GPU
    /// adapter — callers should treat that as "skip", not "fail".
    pub fn render_to_image(
        &self,
        document: &Document,
        config: RenderConfig,
    ) -> Result<Image, RenderError> {
        let width = config.width.max(1);
        let height = config.height.max(1);
        let target_viewport = Viewport {
            width: width as f32,
            height: height as f32,
        };
        let mesh = build_scene(document, document_viewport(document, target_viewport));

        let (device, queue) = acquire_gpu()?;

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("svg3 headless target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let pipeline = build_pipeline(&device);

        // Vertices are uploaded in SVG/world space; `shader.wgsl` projects
        // them to clip space from the uniform view-projection matrix.
        let buffers = (!mesh.is_empty()).then(|| {
            let transform_bind_group =
                build_transform_bind_group(&device, &pipeline, config.view_projection());
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 vertex buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 index buffer"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            (vertex_buffer, index_buffer, transform_bind_group)
        });

        let bytes_per_row = width * 4;
        let padded_bytes_per_row = bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("svg3 readback buffer"),
            size: padded_bytes_per_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("svg3 headless encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("svg3 shape pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let Some((vertex_buffer, index_buffer, transform_bind_group)) = &buffers {
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, transform_bind_group, &[]);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
            }
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

        let pixels = read_back(&device, &readback, width, height, padded_bytes_per_row)?;
        Ok(Image {
            width,
            height,
            pixels,
        })
    }
}

/// Acquire a headless wgpu device, or [`RenderError::NoAdapter`] if none.
fn acquire_gpu() -> Result<(wgpu::Device, wgpu::Queue), RenderError> {
    let instance = wgpu::Instance::default();
    pollster::block_on(async {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|_| {
                log::debug!("svg3-render: no compatible GPU adapter; render skipped");
                RenderError::NoAdapter
            })?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("svg3 headless device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await?;
        Ok((device, queue))
    })
}

/// Build the basic-shape render pipeline.
fn build_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("svg3 shape pipeline"),
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
                format: TARGET_FORMAT,
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
    view_projection: Mat4,
) -> wgpu::BindGroup {
    let uniform = TransformUniform::new(view_projection);
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("svg3 transform uniform"),
        contents: bytemuck::bytes_of(&uniform),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("svg3 transform bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

/// Map the readback buffer and copy its rows into a tightly-packed
/// (unpadded) row-major `RGBA8` buffer.
fn read_back(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
) -> Result<Vec<u8>, RenderError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::Readback(format!("GPU poll failed: {e:?}")))?;
    receiver
        .recv()
        .map_err(|e| RenderError::Readback(e.to_string()))?
        .map_err(|e| RenderError::Readback(e.to_string()))?;

    let row_bytes = (width * 4) as usize;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    {
        let mapped = buffer.slice(..).get_mapped_range();
        for row in 0..height as usize {
            let start = row * padded_bytes_per_row as usize;
            pixels.extend_from_slice(&mapped[start..start + row_bytes]);
        }
    }
    buffer.unmap();
    Ok(pixels)
}

/// Errors that can occur during rendering.
#[derive(Debug, Error)]
pub enum RenderError {
    /// No GPU adapter is available (e.g. a headless machine with no
    /// software fallback). Treat as "skip rendering", not a hard failure.
    #[error("no compatible GPU adapter is available")]
    NoAdapter,
    /// A GPU device could not be acquired from the adapter.
    #[error("could not acquire a GPU device: {0}")]
    DeviceUnavailable(#[from] wgpu::RequestDeviceError),
    /// Reading the rendered texture back to CPU memory failed.
    #[error("reading the rendered image back from the GPU failed: {0}")]
    Readback(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100×100 viewport for `build_scene` tests, whose fixtures use
    /// absolute lengths (so the viewport value does not affect the result).
    fn vp() -> Viewport {
        Viewport {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn vertex_layout_is_tightly_packed() {
        assert_eq!(
            std::mem::size_of::<Vertex>(),
            7 * std::mem::size_of::<f32>()
        );
    }

    #[test]
    fn transform_uniform_matches_wgsl_matrix_size() {
        assert_eq!(
            std::mem::size_of::<TransformUniform>(),
            16 * std::mem::size_of::<f32>()
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

    #[test]
    fn build_scene_tessellates_each_rect_in_document() {
        // Two rects: one renderable, one zero-width and skipped
        // (WPT `shapes/rect-05`).
        let document = svg3_dom::parse(
            r#"<svg><rect width="10" height="10"/><rect width="0" height="10"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        // Only the first rect contributes: one sharp quad.
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn build_scene_offsets_indices_across_rects() {
        let document = svg3_dom::parse(
            r#"<svg><rect width="10" height="10"/><rect x="20" width="10" height="10"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        assert_eq!(mesh.vertices.len(), 8);
        // The second quad's indices are offset past the first quad's vertices.
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn build_scene_is_empty_without_shapes() {
        let document = svg3_dom::parse("<svg><g/></svg>").unwrap();
        assert!(build_scene(&document, vp()).is_empty());
    }

    #[test]
    fn build_scene_tessellates_circle() {
        // A `<circle>` is dispatched to the circle tessellator and
        // contributes a centre-pivoted triangle fan to the combined mesh.
        let document = svg3_dom::parse(r#"<svg><circle cx="20" cy="20" r="10"/></svg>"#).unwrap();
        let mesh = build_scene(&document, vp());
        assert!(!mesh.is_empty());
        // Fan topology: a centre vertex plus one vertex per fan triangle.
        assert_eq!(mesh.vertices.len(), mesh.indices.len() / 3 + 1);
    }

    #[test]
    fn build_scene_tessellates_ellipse() {
        // An `<ellipse>` is dispatched to the ellipse tessellator and
        // contributes a centre-pivoted triangle fan to the combined mesh.
        let document =
            svg3_dom::parse(r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/></svg>"#).unwrap();
        let mesh = build_scene(&document, vp());
        assert!(!mesh.is_empty());
        // Fan topology: a centre vertex plus one vertex per fan triangle.
        assert_eq!(mesh.vertices.len(), mesh.indices.len() / 3 + 1);
    }

    #[test]
    fn build_scene_offsets_indices_across_ellipses() {
        // Two `<ellipse>`s combine into one mesh; the second fan's indices
        // are offset past the first fan's vertices so the triangle list
        // stays valid.
        let one = build_scene(
            &svg3_dom::parse(r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/></svg>"#).unwrap(),
            vp(),
        );
        let two = build_scene(
            &svg3_dom::parse(
                r#"<svg><ellipse cx="20" cy="20" rx="15" ry="8"/><ellipse cx="60" cy="60" rx="12" ry="9"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        let single = one.vertices.len();
        // The combined mesh holds both fans.
        assert_eq!(two.vertices.len(), 2 * single);
        assert_eq!(two.indices.len(), 2 * one.indices.len());
        // The first fan is copied verbatim; the second is that same fan with
        // every index shifted by the first ellipse's vertex count.
        assert_eq!(two.indices[..one.indices.len()], one.indices[..]);
        let shifted: Vec<u32> = one.indices.iter().map(|i| i + single as u32).collect();
        assert_eq!(two.indices[one.indices.len()..], shifted[..]);
    }

    #[test]
    fn build_scene_combines_ellipse_with_rect_and_circle() {
        // A heterogeneous document: `<rect>`, `<circle>` and `<ellipse>` are
        // each dispatched to their own tessellator and appended in document
        // order into one combined mesh.
        let prefix = build_scene(
            &svg3_dom::parse(
                r#"<svg><rect width="10" height="10"/><circle cx="40" cy="40" r="12"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        let full = build_scene(
            &svg3_dom::parse(
                r#"<svg><rect width="10" height="10"/><circle cx="40" cy="40" r="12"/><ellipse cx="70" cy="30" rx="18" ry="9"/></svg>"#,
            )
            .unwrap(),
            vp(),
        );
        // Adding the ellipse only grows the mesh past the rect+circle prefix.
        assert!(full.vertices.len() > prefix.vertices.len());
        assert!(full.indices.len() > prefix.indices.len());
        // The ellipse is last in document order, so its fan pivot — the
        // ellipse centre — is the first vertex past that prefix.
        assert_eq!(
            full.vertices[prefix.vertices.len()].position,
            [70.0, 30.0, 0.0]
        );
    }

    #[test]
    fn build_scene_finds_ellipse_inside_nested_groups() {
        // `build_scene` walks the whole tree, so an `<ellipse>` buried under
        // `<g>` wrappers is still found and tessellated.
        let document = svg3_dom::parse(
            r#"<svg><g><g><ellipse cx="25" cy="35" rx="10" ry="6"/></g></g></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        assert!(!mesh.is_empty());
        // The fan pivot is the ellipse centre, reached despite the wrappers.
        assert_eq!(mesh.vertices[0].position, [25.0, 35.0, 0.0]);
    }

    #[test]
    fn build_scene_skips_ellipse_with_fill_none() {
        // `fill="none"` resolves to no paint, so that ellipse contributes no
        // geometry — only the second, filled ellipse is tessellated.
        let document = svg3_dom::parse(
            r#"<svg><ellipse cx="10" cy="10" rx="8" ry="5" fill="none"/><ellipse cx="40" cy="40" rx="8" ry="5" fill="blue"/></svg>"#,
        )
        .unwrap();
        let mesh = build_scene(&document, vp());
        assert!(!mesh.is_empty());
        // Exactly one fan: `vertices == indices / 3 + 1` holds only for a
        // single fan (two fans leave `2N + 2` vertices, not `2N + 1`).
        assert_eq!(mesh.vertices.len(), mesh.indices.len() / 3 + 1);
        // ...and that fan's pivot is the filled ellipse's centre.
        assert_eq!(mesh.vertices[0].position, [40.0, 40.0, 0.0]);
    }

    #[test]
    fn document_viewport_reads_root_width_and_height() {
        let document = svg3_dom::parse(r#"<svg width="300" height="200"/>"#).unwrap();
        let viewport = document_viewport(
            &document,
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(viewport.width, 300.0);
        assert_eq!(viewport.height, 200.0);
    }

    #[test]
    fn document_viewport_resolves_percentage_root_dimensions_against_fallback() {
        let document = svg3_dom::parse(r#"<svg width="50%" height="25%"/>"#).unwrap();
        let viewport = document_viewport(
            &document,
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(viewport.width, 400.0);
        assert_eq!(viewport.height, 150.0);
    }

    #[test]
    fn build_scene_resolves_percentages_against_svg_root_size() {
        let document = svg3_dom::parse(
            r#"<svg width="300" height="200"><rect width="100%" height="100%"/></svg>"#,
        )
        .unwrap();
        let viewport = document_viewport(
            &document,
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        let mesh = build_scene(&document, viewport);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.vertices[0].position, [0.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[1].position, [300.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[2].position, [300.0, 200.0, 0.0]);
        assert_eq!(mesh.vertices[3].position, [0.0, 200.0, 0.0]);
    }

    #[test]
    fn render_to_image_draws_blue_rect() {
        // WPT `shapes/rect-01`: a blue rect on an otherwise empty surface.
        let document = svg3_dom::parse(
            r#"<svg><rect x="16" y="16" width="32" height="32" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let image = match Renderer::new().render_to_image(&document, config) {
            Ok(image) => image,
            Err(RenderError::NoAdapter) => {
                eprintln!("skipping render_to_image_draws_blue_rect: no GPU adapter");
                return;
            }
            Err(e) => panic!("headless render failed: {e}"),
        };
        assert_eq!((image.width, image.height), (64, 64));
        // The rect covers x,y in 16..48: its centre pixel is blue.
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "centre pixel not blue: {centre:?}"
        );
        // A pixel outside the rect keeps the transparent clear colour.
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_circle() {
        // A blue circle centred on an otherwise empty surface.
        let document =
            svg3_dom::parse(r#"<svg><circle cx="32" cy="32" r="20" fill="blue"/></svg>"#).unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let image = match Renderer::new().render_to_image(&document, config) {
            Ok(image) => image,
            Err(RenderError::NoAdapter) => {
                eprintln!("skipping render_to_image_draws_circle: no GPU adapter");
                return;
            }
            Err(e) => panic!("headless render failed: {e}"),
        };
        assert_eq!((image.width, image.height), (64, 64));
        // The circle's centre pixel is blue.
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "centre pixel not blue: {centre:?}"
        );
        // A corner pixel lies outside the disc — the transparent clear colour.
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_ellipse() {
        // A blue ellipse centred on an otherwise empty surface — wider than
        // it is tall, so it reaches along its x-axis but not its y-axis.
        let document =
            svg3_dom::parse(r#"<svg><ellipse cx="32" cy="32" rx="28" ry="14" fill="blue"/></svg>"#)
                .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let image = match Renderer::new().render_to_image(&document, config) {
            Ok(image) => image,
            Err(RenderError::NoAdapter) => {
                eprintln!("skipping render_to_image_draws_ellipse: no GPU adapter");
                return;
            }
            Err(e) => panic!("headless render failed: {e}"),
        };
        assert_eq!((image.width, image.height), (64, 64));
        // The ellipse's centre pixel is blue.
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "centre pixel not blue: {centre:?}"
        );
        // 22px along the x-axis is within `rx` and covered...
        let on_x_axis = image.pixel(54, 32);
        assert!(
            on_x_axis[2] > 200 && on_x_axis[0] < 60 && on_x_axis[1] < 60,
            "x-axis pixel not blue: {on_x_axis:?}"
        );
        // ...but the same distance along the y-axis is beyond `ry`, so it
        // keeps the transparent clear colour — `rx`/`ry` apply independently.
        assert_eq!(
            image.pixel(32, 54)[3],
            0,
            "pixel beyond ry should be transparent"
        );
        // A point inside the bounding box but outside the elliptical curve —
        // near a bbox corner — stays transparent: the rasterised shape is a
        // genuine ellipse, not its bounding rectangle.
        assert_eq!(
            image.pixel(58, 44)[3],
            0,
            "bbox corner outside the curve should be transparent"
        );
    }

    #[test]
    fn render_to_image_draws_rect_through_camera() {
        // The perspective camera path reaches pixels through the WGSL
        // transform uniform, not the CPU orthographic projection.
        let document = svg3_dom::parse(
            r#"<svg width="64" height="64"><rect x="16" y="16" width="32" height="32" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            camera: Some(Camera::facing(64, 64)),
            ..RenderConfig::default()
        };
        let image = match Renderer::new().render_to_image(&document, config) {
            Ok(image) => image,
            Err(RenderError::NoAdapter) => {
                eprintln!("skipping render_to_image_draws_rect_through_camera: no GPU adapter");
                return;
            }
            Err(e) => panic!("headless render failed: {e}"),
        };
        assert_eq!((image.width, image.height), (64, 64));
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "camera centre pixel not blue: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_fills_target_with_percent_sized_rect() {
        // `<rect width="100%" height="100%">` resolves against the render
        // target and covers it edge to edge — including a non-square target.
        let document =
            svg3_dom::parse(r#"<svg><rect width="100%" height="100%" fill="blue"/></svg>"#)
                .unwrap();
        let config = RenderConfig {
            width: 40,
            height: 24,
            ..RenderConfig::default()
        };
        let image = match Renderer::new().render_to_image(&document, config) {
            Ok(image) => image,
            Err(RenderError::NoAdapter) => {
                eprintln!(
                    "skipping render_to_image_fills_target_with_percent_sized_rect: no GPU adapter"
                );
                return;
            }
            Err(e) => panic!("headless render failed: {e}"),
        };
        assert_eq!((image.width, image.height), (40, 24));
        // Every corner and the centre are blue — the percentage rect bled to
        // all four edges of the non-square target.
        for (x, y) in [(0, 0), (39, 0), (0, 23), (39, 23), (20, 12)] {
            let px = image.pixel(x, y);
            assert!(
                px[2] > 200 && px[0] < 60 && px[1] < 60,
                "pixel ({x}, {y}) not blue: {px:?}"
            );
        }
    }

    #[test]
    fn render_to_image_uses_svg_root_size_for_percentages() {
        // The output target is larger than the root SVG viewport. The
        // percentage rect uses the root 20x10 viewport and therefore leaves
        // the rest of the render target transparent.
        let document = svg3_dom::parse(
            r#"<svg width="20" height="10"><rect width="100%" height="100%" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 40,
            height: 24,
            ..RenderConfig::default()
        };
        let image = match Renderer::new().render_to_image(&document, config) {
            Ok(image) => image,
            Err(RenderError::NoAdapter) => {
                eprintln!(
                    "skipping render_to_image_uses_svg_root_size_for_percentages: no GPU adapter"
                );
                return;
            }
            Err(e) => panic!("headless render failed: {e}"),
        };
        assert_eq!((image.width, image.height), (40, 24));

        let inside = image.pixel(19, 9);
        assert!(
            inside[2] > 200 && inside[0] < 60 && inside[1] < 60,
            "inside pixel not blue: {inside:?}"
        );
        assert_eq!(
            image.pixel(21, 12)[3],
            0,
            "outside root viewport should be transparent"
        );
    }
}
