//! The wgpu [`Renderer`] and its render paths.
//!
//! The [`Renderer`] owns and caches the wgpu device, queue, render pipelines
//! and bind-group layouts. It exposes one unified document encoder,
//! [`Renderer::encode_document`], which walks a parsed document, tessellates
//! every supported 2D shape, applies referenced `<feGaussianBlur>` filters via
//! offscreen GPU passes, and composites them into a caller-owned target view
//! through a caller-owned command encoder. The two render entry points are
//! built on top of it:
//!
//! - [`Renderer::render_to_image`] drives the headless path — it owns the
//!   offscreen sRGB texture, clears it, calls `encode_document`, then copies
//!   the result back into an [`Image`].
//! - A windowed caller (see `app-macos`) brings up its own
//!   [`wgpu::Surface`] and per-frame encoder, clears the swap-chain texture
//!   to its background colour, and calls `encode_document` to draw the
//!   current document into that texture.
//!
//! [`Renderer::create_scene`] + [`Renderer::draw`] remain as a lower-level
//! API for callers that pre-tessellate a [`Mesh`] outside the document walk;
//! the high-level paths go through [`Renderer::encode_document`].

use std::collections::BTreeMap;

use glam::Mat4;
use svg3_dom::Document;
use thiserror::Error;
use wgpu::util::DeviceExt;

use crate::filters::{
    ColorMatrix, ComponentTransfer, ConvolveMatrix, DisplacementMap, DropShadow, FilterInput,
    FilterPrimitive, FilterPrimitiveKind, Flood, GaussianBlur, LightSource, Lighting, Morphology,
    Turbulence,
};
use crate::mesh::VERTEX_ATTRIBUTES;
use crate::scene::{build_render_plan, RenderOp};
use crate::{document_viewport, Mesh, RenderConfig, Vertex, Viewport};

/// Texture format the headless renderer draws into. sRGB-encoded so linear
/// vertex colours are stored correctly; read back as `RGBA8`.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Largest one-sided kernel radius used by the GPU Gaussian blur pass.
const MAX_BLUR_RADIUS: u32 = 64;

/// Uniform data consumed by `shader.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TransformUniform {
    /// Column-major view-projection matrix, matching WGSL matrix layout.
    view_projection: [[f32; 4]; 4],
}

/// Uniform data consumed by `filter.wgsl`.
///
/// One shared layout fills every filter pass; only the fields a particular
/// fragment entry point reads are meaningful, the rest are zeroed. Field
/// names map 1:1 to the WGSL `FilterUniform` struct in `filter.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FilterUniform {
    texel_size: [f32; 2],
    direction: [f32; 2],
    color: [f32; 4],
    extra: [f32; 4],
    light: [f32; 4],
    light_dir: [f32; 4],
    lighting: [f32; 4],
    matrix_r0: [f32; 4],
    matrix_r1: [f32; 4],
    matrix_r2: [f32; 4],
    matrix_r3: [f32; 4],
    matrix_col4: [f32; 4],
    transfer_r0: [f32; 4],
    transfer_r1: [f32; 4],
    transfer_g0: [f32; 4],
    transfer_g1: [f32; 4],
    transfer_b0: [f32; 4],
    transfer_b1: [f32; 4],
    transfer_a0: [f32; 4],
    transfer_a1: [f32; 4],
    transfer_kinds: [u32; 4],
    transfer_counts: [u32; 4],
    sigma: f32,
    radius: u32,
    mode: u32,
    flags: u32,
}

impl FilterUniform {
    fn empty(width: u32, height: u32) -> Self {
        let mut uniform = Self::zeroed();
        uniform.texel_size = [1.0 / width as f32, 1.0 / height as f32];
        uniform
    }

    fn zeroed() -> Self {
        bytemuck::Zeroable::zeroed()
    }

    fn blur(width: u32, height: u32, direction: [f32; 2], sigma: f32) -> Self {
        let mut uniform = Self::empty(width, height);
        uniform.direction = direction;
        uniform.sigma = sigma;
        uniform.radius = (sigma * 3.0).ceil().clamp(0.0, MAX_BLUR_RADIUS as f32) as u32;
        uniform
    }

    fn composite(width: u32, height: u32) -> Self {
        Self::empty(width, height)
    }

    /// An identity colour matrix in the matrix block — used to copy the
    /// source verbatim through the colour-matrix pipeline when a chain step
    /// reduces to a passthrough (e.g. a zero-deviation Gaussian blur).
    fn passthrough(width: u32, height: u32) -> Self {
        let mut uniform = Self::empty(width, height);
        uniform.matrix_r0 = [1.0, 0.0, 0.0, 0.0];
        uniform.matrix_r1 = [0.0, 1.0, 0.0, 0.0];
        uniform.matrix_r2 = [0.0, 0.0, 1.0, 0.0];
        uniform.matrix_r3 = [0.0, 0.0, 0.0, 1.0];
        uniform
    }

    /// SourceAlpha extractor: RGB rows zero, A row copies input alpha. Used
    /// to derive the filter's `SourceAlpha` pseudo-input from `SourceGraphic`
    /// via the colour-matrix pipeline.
    fn source_alpha(width: u32, height: u32) -> Self {
        let mut uniform = Self::empty(width, height);
        // R = G = B = 0; A = 0*R + 0*G + 0*B + 1*A + 0 = src.a.
        uniform.matrix_r3 = [0.0, 0.0, 0.0, 1.0];
        uniform
    }
}

/// Pick the texture view that satisfies a primitive's `in` / `in2` reference.
///
/// Unresolvable references — a `Named` reference with no matching earlier
/// `result`, or `Default` for the very first primitive — fall back to
/// `SourceGraphic`, matching SVG's "if the value is unresolved, use
/// SourceGraphic" behaviour.
fn resolve_input<'a>(
    input: &FilterInput,
    prev_index: Option<usize>,
    source: &'a FilterTexture,
    source_alpha: &'a FilterTexture,
    outputs: &'a [FilterTexture],
    named: &BTreeMap<&str, usize>,
) -> &'a wgpu::TextureView {
    match input {
        FilterInput::Default => match prev_index {
            Some(i) => &outputs[i].view,
            None => &source.view,
        },
        FilterInput::SourceGraphic => &source.view,
        FilterInput::SourceAlpha => &source_alpha.view,
        FilterInput::Named(name) => match named.get(name.as_str()) {
            Some(i) => &outputs[*i].view,
            None => &source.view,
        },
    }
}

fn color_matrix_uniform(extent: wgpu::Extent3d, cm: &ColorMatrix) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    let m = &cm.matrix;
    uniform.matrix_r0 = [m[0][0], m[0][1], m[0][2], m[0][3]];
    uniform.matrix_r1 = [m[1][0], m[1][1], m[1][2], m[1][3]];
    uniform.matrix_r2 = [m[2][0], m[2][1], m[2][2], m[2][3]];
    uniform.matrix_r3 = [m[3][0], m[3][1], m[3][2], m[3][3]];
    uniform.matrix_col4 = [m[0][4], m[1][4], m[2][4], m[3][4]];
    uniform
}

fn turbulence_uniform(extent: wgpu::Extent3d, t: Turbulence) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.extra = [t.base_frequency[0], t.base_frequency[1], 0.0, 0.0];
    uniform.radius = t.num_octaves;
    uniform.sigma = t.seed;
    uniform.flags = if t.fractal_noise { 1 } else { 0 };
    uniform
}

fn lighting_uniform(extent: wgpu::Extent3d, l: Lighting, specular: bool) -> FilterUniform {
    // `lighting.w` encodes the light source kind: 0 = distant, 1 = point,
    // 2 = spot. Must stay in sync with `LIGHT_TYPE_*` constants in
    // `filter.wgsl`.
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.color = l.lighting_color;
    let is_specular = if specular { 1.0 } else { 0.0 };
    match l.light {
        LightSource::Distant(dir) => {
            uniform.light = [dir[0], dir[1], dir[2], l.specular_exponent];
            uniform.lighting = [l.surface_scale, l.constant, is_specular, 0.0];
        }
        LightSource::Point(pos) => {
            uniform.light = [pos[0], pos[1], pos[2], l.specular_exponent];
            uniform.lighting = [l.surface_scale, l.constant, is_specular, 1.0];
        }
        LightSource::Spot {
            position,
            direction,
            cone_exponent,
            cos_limit,
        } => {
            uniform.light = [position[0], position[1], position[2], l.specular_exponent];
            // `light_dir.xyz` carries the cone axis (light -> pointsAt);
            // `light_dir.w` carries `cos(limitingConeAngle)` or `-1.0` if
            // the user did not constrain the cone.
            uniform.light_dir = [direction[0], direction[1], direction[2], cos_limit];
            uniform.lighting = [l.surface_scale, l.constant, is_specular, 2.0];
            // `extra.x` is reused as the cone-falloff exponent — distinct
            // from the surface Phong exponent stored in `light.w`.
            uniform.extra[0] = cone_exponent;
        }
    }
    uniform
}

fn morphology_uniform(
    extent: wgpu::Extent3d,
    m: Morphology,
    direction: [f32; 2],
    radius_pixels: f32,
) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.direction = direction;
    uniform.radius = radius_pixels.round().clamp(0.0, MAX_BLUR_RADIUS as f32) as u32;
    uniform.flags = if m.dilate { 1 } else { 0 };
    uniform
}

fn flood_uniform(extent: wgpu::Extent3d, f: Flood) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.color = f.color;
    uniform
}

fn drop_shadow_alpha_uniform(extent: wgpu::Extent3d, shadow: DropShadow) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.color = shadow.color;
    // The shader subtracts `direction` in UV space, so convert the pixel offset.
    uniform.direction = [
        shadow.offset[0] * uniform.texel_size[0],
        shadow.offset[1] * uniform.texel_size[1],
    ];
    uniform
}

fn displacement_uniform(extent: wgpu::Extent3d, d: DisplacementMap) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.extra = [d.scale, 0.0, 0.0, 0.0];
    uniform.transfer_kinds = [d.x_channel, d.y_channel, 0, 0];
    uniform
}

fn convolve_uniform(extent: wgpu::Extent3d, c: ConvolveMatrix) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.matrix_r0 = [c.kernel[0][0], c.kernel[0][1], c.kernel[0][2], 0.0];
    uniform.matrix_r1 = [c.kernel[1][0], c.kernel[1][1], c.kernel[1][2], 0.0];
    uniform.matrix_r2 = [c.kernel[2][0], c.kernel[2][1], c.kernel[2][2], 0.0];
    uniform.matrix_col4 = [c.divisor, c.bias, 0.0, 0.0];
    uniform.flags = if c.preserve_alpha { 1 } else { 0 };
    uniform
}

fn component_transfer_uniform(
    extent: wgpu::Extent3d,
    transfer: ComponentTransfer,
) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    let (r0, r1) = split_transfer_table(&transfer.r.table);
    let (g0, g1) = split_transfer_table(&transfer.g.table);
    let (b0, b1) = split_transfer_table(&transfer.b.table);
    let (a0, a1) = split_transfer_table(&transfer.a.table);
    uniform.transfer_r0 = r0;
    uniform.transfer_r1 = r1;
    uniform.transfer_g0 = g0;
    uniform.transfer_g1 = g1;
    uniform.transfer_b0 = b0;
    uniform.transfer_b1 = b1;
    uniform.transfer_a0 = a0;
    uniform.transfer_a1 = a1;
    uniform.transfer_kinds = [
        transfer.r.kind,
        transfer.g.kind,
        transfer.b.kind,
        transfer.a.kind,
    ];
    uniform.transfer_counts = [
        transfer.r.count,
        transfer.g.count,
        transfer.b.count,
        transfer.a.count,
    ];
    uniform
}

/// Split an 8-entry transfer table into two `vec4` halves matching the WGSL
/// `transfer_<c>0` / `transfer_<c>1` layout.
fn split_transfer_table(table: &[f32; 8]) -> ([f32; 4], [f32; 4]) {
    (
        [table[0], table[1], table[2], table[3]],
        [table[4], table[5], table[6], table[7]],
    )
}

impl TransformUniform {
    fn new(view_projection: Mat4) -> Self {
        Self {
            view_projection: view_projection.to_cols_array_2d(),
        }
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

/// GPU buffers for one tessellated scene, ready to draw.
///
/// Produced by [`Renderer::create_scene`]; consumed by [`Renderer::draw`] and
/// rewritten by [`Renderer::update_view_projection`]. Opaque, and tied to the
/// [`Renderer`] that created it — its bind group references that renderer's
/// pipeline layout.
#[derive(Debug)]
pub struct GpuScene {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    transform_buffer: wgpu::Buffer,
    transform_bind_group: wgpu::BindGroup,
    index_count: u32,
}

/// One offscreen texture used by the GPU filter passes.
struct FilterTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Renders SVG3 documents to GPU images and into caller-owned render passes.
///
/// Owns and caches the wgpu device, queue, render pipeline and bind-group
/// layout, so the shader and pipeline are built once — at construction —
/// rather than per render.
#[derive(Debug)]
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    filter_bind_group_layout: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    color_matrix_pipeline: wgpu::RenderPipeline,
    turbulence_pipeline: wgpu::RenderPipeline,
    lighting_pipeline: wgpu::RenderPipeline,
    morphology_pipeline: wgpu::RenderPipeline,
    flood_pipeline: wgpu::RenderPipeline,
    drop_shadow_alpha_pipeline: wgpu::RenderPipeline,
    drop_shadow_composite_pipeline: wgpu::RenderPipeline,
    displacement_pipeline: wgpu::RenderPipeline,
    convolve_pipeline: wgpu::RenderPipeline,
    component_transfer_pipeline: wgpu::RenderPipeline,
    filter_sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}

impl Renderer {
    /// Create a headless renderer, bringing up a wgpu device with no surface.
    ///
    /// The pipeline targets the sRGB `RGBA8` format that
    /// [`render_to_image`](Renderer::render_to_image) reads back. Returns
    /// [`RenderError::NoAdapter`] when the machine exposes no GPU adapter —
    /// callers should treat that as "skip", not "fail".
    pub fn headless() -> Result<Self, RenderError> {
        let (device, queue) = acquire_gpu()?;
        Ok(Self::with_device(device, queue, TARGET_FORMAT))
    }

    /// Create a renderer over a caller-supplied `device` and `queue`, with a
    /// pipeline targeting `format`.
    ///
    /// For the windowed path: the caller brings up a surface-compatible device
    /// (so the adapter is chosen with `compatible_surface`) and hands it here
    /// with the surface's texture `format`. Such a renderer is driven through
    /// [`encode_document`](Renderer::encode_document) into the caller's own
    /// surface texture and encoder. [`render_to_image`](Renderer::render_to_image)
    /// additionally requires `format` to be sRGB `RGBA8`.
    pub fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let pipeline = build_pipeline(&device, format);
        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let filter_bind_group_layout = build_filter_bind_group_layout(&device);
        let blur_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 Gaussian blur pipeline",
            "fs_blur",
            None,
        );
        let composite_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 filter composite pipeline",
            "fs_composite",
            Some(premultiplied_alpha_blend()),
        );
        let color_matrix_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feColorMatrix pipeline",
            "fs_color_matrix",
            None,
        );
        let turbulence_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feTurbulence pipeline",
            "fs_turbulence",
            None,
        );
        let lighting_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feLighting pipeline",
            "fs_lighting",
            None,
        );
        let morphology_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feMorphology pipeline",
            "fs_morphology",
            None,
        );
        let flood_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feFlood pipeline",
            "fs_flood",
            None,
        );
        let drop_shadow_alpha_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feDropShadow alpha pipeline",
            "fs_drop_shadow_alpha",
            None,
        );
        let drop_shadow_composite_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feDropShadow composite pipeline",
            "fs_drop_shadow_composite",
            None,
        );
        let displacement_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feDisplacementMap pipeline",
            "fs_displacement",
            None,
        );
        let convolve_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feConvolveMatrix pipeline",
            "fs_convolve",
            None,
        );
        let component_transfer_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feComponentTransfer pipeline",
            "fs_component_transfer",
            None,
        );
        // Filter passes need bilinear sampling for fractional displacements
        // (feDisplacementMap, feConvolveMatrix kernel taps); the blur kernel
        // is fine with either filter mode because it samples on the integer
        // pixel grid.
        let filter_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("svg3 filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            filter_bind_group_layout,
            blur_pipeline,
            composite_pipeline,
            color_matrix_pipeline,
            turbulence_pipeline,
            lighting_pipeline,
            morphology_pipeline,
            flood_pipeline,
            drop_shadow_alpha_pipeline,
            drop_shadow_composite_pipeline,
            displacement_pipeline,
            convolve_pipeline,
            component_transfer_pipeline,
            filter_sampler,
            format,
        }
    }

    /// The wgpu device backing this renderer.
    ///
    /// Exposed so a windowed caller can configure its own [`wgpu::Surface`]
    /// and encode commands against the same device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The wgpu queue backing this renderer.
    ///
    /// Exposed so a windowed caller can submit its own command buffers.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Upload `mesh` and an initial `view_projection` into GPU buffers.
    ///
    /// Returns `None` for an empty mesh: a scene with no triangles draws
    /// nothing, so it needs no buffers. Vertices are uploaded in SVG/world
    /// space; `shader.wgsl` projects them to clip space with the
    /// `view_projection` matrix, which
    /// [`update_view_projection`](Renderer::update_view_projection) can later
    /// rewrite.
    pub fn create_scene(&self, mesh: &Mesh, view_projection: Mat4) -> Option<GpuScene> {
        if mesh.is_empty() {
            return None;
        }
        let uniform = TransformUniform::new(view_projection);
        let transform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 transform uniform"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let transform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("svg3 transform bind group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 vertex buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 index buffer"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        Some(GpuScene {
            vertex_buffer,
            index_buffer,
            transform_buffer,
            transform_bind_group,
            index_count: mesh.indices.len() as u32,
        })
    }

    /// Rewrite a scene's view-projection matrix.
    ///
    /// Cheap enough to call on every camera move: it writes only the 64-byte
    /// transform uniform, leaving the vertex and index buffers untouched.
    pub fn update_view_projection(&self, scene: &GpuScene, view_projection: Mat4) {
        let uniform = TransformUniform::new(view_projection);
        self.queue
            .write_buffer(&scene.transform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Record the draw commands for `scene` into `pass`.
    ///
    /// The caller owns the render pass — its target, load/store ops and clear
    /// colour — so one renderer drives both the headless image pass and a
    /// windowed surface pass.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &GpuScene) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &scene.transform_bind_group, &[]);
        pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
        pass.set_index_buffer(scene.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..scene.index_count, 0, 0..1);
    }

    /// Render every supported 2D SVG shape in `document` headlessly into an
    /// [`Image`] of `config.width × config.height` pixels.
    ///
    /// Wraps [`encode_document`](Renderer::encode_document) with the
    /// headless-specific glue: it allocates the offscreen sRGB texture,
    /// clears it to transparent, encodes the document draws, then copies
    /// the texture into a CPU-mappable readback buffer.
    ///
    /// Returns [`RenderError::UnsupportedImageFormat`] when this renderer's
    /// pipeline does not target sRGB `RGBA8` — the readback assumes that byte
    /// layout. A renderer from [`Renderer::headless`] always satisfies this.
    pub fn render_to_image(
        &self,
        document: &Document,
        config: RenderConfig,
    ) -> Result<Image, RenderError> {
        if self.format != TARGET_FORMAT {
            return Err(RenderError::UnsupportedImageFormat(self.format));
        }
        let width = config.width.max(1);
        let height = config.height.max(1);
        let target_viewport = Viewport {
            width: width as f32,
            height: height as f32,
        };
        let viewport = document_viewport(document, target_viewport);
        let view_projection = config.view_projection();

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("svg3 headless target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bytes_per_row = width * 4;
        let padded_bytes_per_row = bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("svg3 readback buffer"),
            size: padded_bytes_per_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("svg3 headless encoder"),
            });
        // Clear the headless target up front so the document draws — direct
        // meshes and filter composites — can `LoadOp::Load` from it.
        clear_target(
            &mut encoder,
            &view,
            wgpu::Color::TRANSPARENT,
            "svg3 headless clear pass",
        );
        self.encode_document(
            document,
            viewport,
            view_projection,
            &view,
            extent,
            &mut encoder,
        );
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
        self.queue.submit(std::iter::once(encoder.finish()));

        let pixels = read_back(&self.device, &readback, width, height, padded_bytes_per_row)?;
        Ok(Image {
            width,
            height,
            pixels,
        })
    }

    /// Encode the GPU draw commands for `document` into `encoder`, painting
    /// into `target`.
    ///
    /// Walks `document` once, tessellates every supported 2D shape, applies
    /// any referenced `<feGaussianBlur>` filter through offscreen GPU passes
    /// (allocated lazily against `target_extent`), and composites the result
    /// onto `target` in painter's order. This is the single GPU path shared
    /// by the headless [`Renderer::render_to_image`] and any windowed caller
    /// driving its own surface.
    ///
    /// `target` is **loaded, not cleared** — callers are responsible for
    /// clearing it to their desired background colour before this call.
    /// `viewport` is the basis for percentage lengths in the document;
    /// callers that want SVG root sizing should resolve it via
    /// [`document_viewport`]. `view_projection` maps SVG user space to clip
    /// space; for the standard 2D/3D mix, build it via
    /// [`RenderConfig::view_projection`]. `target_extent` is the texture's
    /// pixel size, used to size the offscreen filter textures.
    ///
    /// The caller owns the encoder, so the draws integrate into any larger
    /// frame the caller is composing (e.g. a background-clear pass before
    /// this call, an overlay UI pass after).
    pub fn encode_document(
        &self,
        document: &Document,
        viewport: Viewport,
        view_projection: Mat4,
        target: &wgpu::TextureView,
        target_extent: wgpu::Extent3d,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let plan = build_render_plan(document, viewport);
        for op in &plan {
            match op {
                RenderOp::Mesh(mesh) => {
                    if let Some(scene) = self.create_scene(mesh, view_projection) {
                        self.encode_scene_draw(encoder, target, &scene, "svg3 shape pass");
                    }
                }
                RenderOp::Filter { mesh, primitives } => {
                    self.encode_filter_chain(
                        encoder,
                        target,
                        mesh,
                        primitives,
                        view_projection,
                        target_extent,
                    );
                }
            }
        }
    }

    fn encode_scene_draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        scene: &GpuScene,
        label: &str,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        self.draw(&mut pass, scene);
    }

    fn encode_filter_chain(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        mesh: &Mesh,
        primitives: &[FilterPrimitive],
        view_projection: Mat4,
        extent: wgpu::Extent3d,
    ) {
        // SVG filter graphs are DAGs, not linear chains: a primitive's `in` /
        // `in2` can reference SourceGraphic, SourceAlpha, or an earlier
        // primitive's named `result`. The executor keeps one immutable
        // texture per node — `source`, `source_alpha`, and one
        // `outputs[i]` per primitive — and a shared `scratch` for multi-pass
        // primitives (separable blurs, drop shadow). When a primitive's `in`
        // is the default, it picks up the previous primitive's output, which
        // recovers the linear-chain behaviour for documents that don't use
        // `result`.
        let source = self.create_filter_texture(extent, "svg3 filter source");

        if let Some(scene) = self.create_scene(mesh, view_projection) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("svg3 filtered shape pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &source.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.draw(&mut pass, &scene);
        } else {
            // No geometry — leave the source texture transparent so that
            // generator primitives (feFlood, feTurbulence) still produce
            // useful output, and so an `in="SourceAlpha"` reference is
            // a well-defined zero.
            clear_target(
                encoder,
                &source.view,
                wgpu::Color::TRANSPARENT,
                "svg3 filter empty source clear",
            );
        }

        // SourceAlpha = (0, 0, 0, src.a). Derive it through the colour-matrix
        // pipeline once — primitives that don't reference SourceAlpha still
        // pay this one pass, but the cost is trivial and the code stays
        // simple.
        let source_alpha = self.create_filter_texture(extent, "svg3 filter source alpha");
        let alpha_uniform = FilterUniform::source_alpha(extent.width, extent.height);
        self.encode_filter_pass(
            encoder,
            &self.color_matrix_pipeline,
            &source.view,
            &source.view,
            &source_alpha.view,
            &alpha_uniform,
            "svg3 SourceAlpha derivation",
        );

        // One output texture per primitive, plus one shared scratch buffer.
        let outputs: Vec<FilterTexture> = (0..primitives.len())
            .map(|i| self.create_filter_texture(extent, &format!("svg3 filter output {i}")))
            .collect();
        let scratch = self.create_filter_texture(extent, "svg3 filter scratch");

        let mut named: BTreeMap<&str, usize> = BTreeMap::new();
        let mut prev_index: Option<usize> = None;
        for (i, primitive) in primitives.iter().enumerate() {
            let in1_view = resolve_input(
                &primitive.input,
                prev_index,
                &source,
                &source_alpha,
                &outputs,
                &named,
            );
            let in2_view = resolve_input(
                &primitive.input2,
                prev_index,
                &source,
                &source_alpha,
                &outputs,
                &named,
            );
            let output_view = &outputs[i].view;
            self.encode_primitive(
                encoder,
                primitive,
                in1_view,
                in2_view,
                output_view,
                &scratch.view,
                extent,
            );
            if let Some(name) = primitive.result.as_deref() {
                named.insert(name, i);
            }
            prev_index = Some(i);
        }

        // Composite the last primitive's output onto the destination. Falling
        // back to the raw source covers the "filter with no primitives" case,
        // which `scene.rs` already filters out — but the guard keeps this
        // path safe if a caller invokes the renderer differently.
        let final_view = match prev_index {
            Some(i) => &outputs[i].view,
            None => &source.view,
        };
        self.encode_composite_pass(encoder, final_view, &source.view, target, extent);
    }

    /// Encode one primitive's GPU pass(es). Reads `in1` / `in2`, writes to
    /// `output`, and may use `scratch` as an internal bounce buffer for
    /// multi-pass primitives.
    #[allow(clippy::too_many_arguments)]
    fn encode_primitive(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        primitive: &FilterPrimitive,
        in1: &wgpu::TextureView,
        in2: &wgpu::TextureView,
        output: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        extent: wgpu::Extent3d,
    ) {
        match &primitive.kind {
            FilterPrimitiveKind::GaussianBlur(blur) => {
                self.encode_gaussian_blur_chain(encoder, in1, output, scratch, *blur, extent);
            }
            FilterPrimitiveKind::ColorMatrix(cm) => {
                let uniform = color_matrix_uniform(extent, cm);
                self.encode_filter_pass(
                    encoder,
                    &self.color_matrix_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feColorMatrix pass",
                );
            }
            FilterPrimitiveKind::Turbulence(t) => {
                let uniform = turbulence_uniform(extent, *t);
                self.encode_filter_pass(
                    encoder,
                    &self.turbulence_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feTurbulence pass",
                );
            }
            FilterPrimitiveKind::SpecularLighting(l) | FilterPrimitiveKind::DiffuseLighting(l) => {
                let specular = matches!(primitive.kind, FilterPrimitiveKind::SpecularLighting(_));
                let uniform = lighting_uniform(extent, *l, specular);
                self.encode_filter_pass(
                    encoder,
                    &self.lighting_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feLighting pass",
                );
            }
            FilterPrimitiveKind::Morphology(m) => {
                // Separable: X pass writes scratch, Y pass writes output.
                let uniform_x = morphology_uniform(extent, *m, [1.0, 0.0], m.radius_x);
                self.encode_filter_pass(
                    encoder,
                    &self.morphology_pipeline,
                    in1,
                    in2,
                    scratch,
                    &uniform_x,
                    "svg3 feMorphology X pass",
                );
                let uniform_y = morphology_uniform(extent, *m, [0.0, 1.0], m.radius_y);
                self.encode_filter_pass(
                    encoder,
                    &self.morphology_pipeline,
                    scratch,
                    in2,
                    output,
                    &uniform_y,
                    "svg3 feMorphology Y pass",
                );
            }
            FilterPrimitiveKind::Flood(f) => {
                let uniform = flood_uniform(extent, *f);
                self.encode_filter_pass(
                    encoder,
                    &self.flood_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feFlood pass",
                );
            }
            FilterPrimitiveKind::DropShadow(d) => {
                self.encode_drop_shadow(encoder, in1, output, scratch, *d, extent);
            }
            FilterPrimitiveKind::DisplacementMap(d) => {
                let uniform = displacement_uniform(extent, *d);
                self.encode_filter_pass(
                    encoder,
                    &self.displacement_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feDisplacementMap pass",
                );
            }
            FilterPrimitiveKind::ConvolveMatrix(c) => {
                let uniform = convolve_uniform(extent, *c);
                self.encode_filter_pass(
                    encoder,
                    &self.convolve_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feConvolveMatrix pass",
                );
            }
            FilterPrimitiveKind::ComponentTransfer(t) => {
                let uniform = component_transfer_uniform(extent, *t);
                self.encode_filter_pass(
                    encoder,
                    &self.component_transfer_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feComponentTransfer pass",
                );
            }
        }
    }

    /// Separable Gaussian blur (input -> scratch -> output). For a single-
    /// axis or identity blur the helper still ends on `output`, so the
    /// caller's invariant ("output texture holds this primitive's result")
    /// always holds.
    fn encode_gaussian_blur_chain(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        output: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        blur: GaussianBlur,
        extent: wgpu::Extent3d,
    ) {
        let want_x = blur.std_deviation_x > 0.0;
        let want_y = blur.std_deviation_y > 0.0;

        if !want_x && !want_y {
            // Identity blur — copy through using the colour-matrix pipeline
            // with an identity matrix.
            let uniform = FilterUniform::passthrough(extent.width, extent.height);
            self.encode_filter_pass(
                encoder,
                &self.color_matrix_pipeline,
                input,
                input,
                output,
                &uniform,
                "svg3 Gaussian blur (passthrough) pass",
            );
            return;
        }

        if want_x && want_y {
            // input -> scratch (X), then scratch -> output (Y).
            let uniform_x = FilterUniform::blur(
                extent.width,
                extent.height,
                [1.0, 0.0],
                blur.std_deviation_x,
            );
            self.encode_filter_pass(
                encoder,
                &self.blur_pipeline,
                input,
                input,
                scratch,
                &uniform_x,
                "svg3 Gaussian blur X pass",
            );
            let uniform_y = FilterUniform::blur(
                extent.width,
                extent.height,
                [0.0, 1.0],
                blur.std_deviation_y,
            );
            self.encode_filter_pass(
                encoder,
                &self.blur_pipeline,
                scratch,
                scratch,
                output,
                &uniform_y,
                "svg3 Gaussian blur Y pass",
            );
            return;
        }

        // Single-axis blur — one pass, input -> output.
        let (direction, sigma) = if want_x {
            ([1.0, 0.0], blur.std_deviation_x)
        } else {
            ([0.0, 1.0], blur.std_deviation_y)
        };
        let uniform = FilterUniform::blur(extent.width, extent.height, direction, sigma);
        self.encode_filter_pass(
            encoder,
            &self.blur_pipeline,
            input,
            input,
            output,
            &uniform,
            "svg3 Gaussian blur axis pass",
        );
    }

    /// `feDropShadow` is a five-step pipeline; the helper ping-pongs through
    /// `output` and `scratch` so the primitive's own input texture stays
    /// untouched (it must remain readable, e.g. for an upstream `result`).
    fn encode_drop_shadow(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        output: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        shadow: DropShadow,
        extent: wgpu::Extent3d,
    ) {
        // 1) Colour the source alpha at offset uv into `scratch`. `input` is
        //    the SVG-level "input image" for the shadow (typically the
        //    SourceGraphic).
        let uniform_alpha = drop_shadow_alpha_uniform(extent, shadow);
        self.encode_filter_pass(
            encoder,
            &self.drop_shadow_alpha_pipeline,
            input,
            input,
            scratch,
            &uniform_alpha,
            "svg3 feDropShadow alpha pass",
        );
        // 2) Blur X: scratch -> output.
        let uniform_blur_x = FilterUniform::blur(
            extent.width,
            extent.height,
            [1.0, 0.0],
            shadow.std_deviation_x,
        );
        self.encode_filter_pass(
            encoder,
            &self.blur_pipeline,
            scratch,
            scratch,
            output,
            &uniform_blur_x,
            "svg3 feDropShadow blur X pass",
        );
        // 3) Blur Y: output -> scratch. After this, `scratch` holds the final
        //    blurred shadow.
        let uniform_blur_y = FilterUniform::blur(
            extent.width,
            extent.height,
            [0.0, 1.0],
            shadow.std_deviation_y,
        );
        self.encode_filter_pass(
            encoder,
            &self.blur_pipeline,
            output,
            output,
            scratch,
            &uniform_blur_y,
            "svg3 feDropShadow blur Y pass",
        );
        // 4) Composite: the shadow (in1 = scratch) under the primitive's own
        //    input image (in2 = input), into `output`. SVG-spec semantics:
        //    feDropShadow composites the input image on top of the shadow.
        let uniform_composite = FilterUniform::composite(extent.width, extent.height);
        self.encode_filter_pass(
            encoder,
            &self.drop_shadow_composite_pipeline,
            scratch,
            input,
            output,
            &uniform_composite,
            "svg3 feDropShadow composite pass",
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_filter_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        in1: &wgpu::TextureView,
        in2: &wgpu::TextureView,
        output: &wgpu::TextureView,
        uniform: &FilterUniform,
        label: &str,
    ) {
        let bind_group = self.create_filter_bind_group(in1, in2, uniform, "svg3 filter bind");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn encode_composite_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        chain_output: &wgpu::TextureView,
        source_view: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        extent: wgpu::Extent3d,
    ) {
        let uniform = FilterUniform::composite(extent.width, extent.height);
        let bind_group = self.create_filter_bind_group(
            chain_output,
            source_view,
            &uniform,
            "svg3 filter composite bind group",
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("svg3 filter composite pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn create_filter_bind_group(
        &self,
        source: &wgpu::TextureView,
        source2: &wgpu::TextureView,
        uniform: &FilterUniform,
        label: &str,
    ) -> wgpu::BindGroup {
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 filter uniform"),
                contents: bytemuck::bytes_of(uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.filter_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.filter_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(source2),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn create_filter_texture(&self, extent: wgpu::Extent3d, label: &str) -> FilterTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        FilterTexture {
            _texture: texture,
            view,
        }
    }
}

/// Execute a single render pass that clears `target` to `color`. Used by both
/// the headless and the windowed paths to prime their target before
/// [`Renderer::encode_document`], which always loads (rather than clears) the
/// existing target so it can composite filter results on top.
pub fn clear_target(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    color: wgpu::Color,
    label: &str,
) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
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

/// Build the 2D shape render pipeline targeting `format`.
fn build_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
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
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn build_filter_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("svg3 filter bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn build_filter_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    label: &str,
    fragment_entry_point: &str,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("filter.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("svg3 filter pipeline layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_fullscreen"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(fragment_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn premultiplied_alpha_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
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
    /// [`Renderer::render_to_image`] was called on a renderer whose pipeline
    /// does not target sRGB `RGBA8`. The headless image readback assumes that
    /// byte layout; a renderer from [`Renderer::headless`] always satisfies it.
    #[error("render_to_image requires an Rgba8UnormSrgb renderer, but this one targets {0:?}")]
    UnsupportedImageFormat(wgpu::TextureFormat),
    /// Reading the rendered texture back to CPU memory failed.
    #[error("reading the rendered image back from the GPU failed: {0}")]
    Readback(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Camera;

    /// A headless [`Renderer`], or `None` (after printing a skip message)
    /// when the host has no GPU adapter — so a GPU-backed test self-skips on
    /// a GPU-less runner instead of failing.
    fn skip_or_renderer(test: &str) -> Option<Renderer> {
        match Renderer::headless() {
            Ok(renderer) => Some(renderer),
            Err(RenderError::NoAdapter) => {
                eprintln!("skipping {test}: no GPU adapter");
                None
            }
            Err(error) => panic!("renderer construction failed: {error}"),
        }
    }

    #[test]
    fn transform_uniform_matches_wgsl_matrix_size() {
        assert_eq!(
            std::mem::size_of::<TransformUniform>(),
            16 * std::mem::size_of::<f32>()
        );
    }

    #[test]
    fn filter_uniform_matches_wgsl_layout() {
        // The shared filter uniform packs (16-byte-aligned vec4 blocks):
        //   texel_size(2) + direction(2)                                = 4 floats
        //   + color + extra + light + light_dir + lighting              = 5 * 4 = 20 floats
        //   + matrix rows r0..r3 + matrix_col4                          = 5 * 4 = 20 floats
        //   + 8 transfer halves (transfer_<rgba>{0,1})                  = 8 * 4 = 32 floats
        //   + transfer_kinds(u32×4) + transfer_counts(u32×4)            = 2 * 4 = 8 (4-byte words)
        //   + sigma, radius, mode, flags                                = 4 (4-byte words)
        // Total = 88 * 4 = 352 bytes; a multiple of 16, satisfying
        // WGSL std140-style alignment.
        assert_eq!(std::mem::size_of::<FilterUniform>(), 352);
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
        let Some(renderer) = skip_or_renderer("render_to_image_draws_blue_rect") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
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
    fn render_to_image_applies_gaussian_blur_filter() {
        let filtered = svg3_dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let unfiltered = svg3_dom::parse(
            r#"<svg><rect x="24" y="24" width="16" height="16" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_applies_gaussian_blur_filter")
        else {
            return;
        };

        let blurred = renderer
            .render_to_image(&filtered, config)
            .expect("filtered render failed");
        let sharp = renderer
            .render_to_image(&unfiltered, config)
            .expect("unfiltered render failed");

        assert_eq!(
            sharp.pixel(20, 32)[3],
            0,
            "control pixel is outside the rect"
        );
        let halo = blurred.pixel(20, 32);
        assert!(
            halo[3] > 8 && halo[2] > 8,
            "blurred rect should create a blue alpha halo outside the sharp edge: {halo:?}"
        );

        let centre = blurred.pixel(32, 32);
        assert!(
            centre[3] > 150 && centre[2] > 80,
            "blurred rect centre should remain visibly blue: {centre:?}"
        );
        assert!(
            blurred.pixel(4, 4)[3] < 4,
            "far-away pixels should remain transparent"
        );
    }

    #[test]
    fn render_to_image_applies_filter_to_group_as_one_surface() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="3"/></filter><g filter="url(#soft)"><rect x="22" y="22" width="10" height="20" fill="red"/><rect x="32" y="22" width="10" height="20" fill="blue"/></g></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_applies_filter_to_group_as_one_surface")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("filtered render failed");

        let left_halo = image.pixel(18, 32);
        let right_halo = image.pixel(46, 32);
        assert!(
            left_halo[3] > 4 && left_halo[0] > left_halo[2],
            "group blur should carry the red side outward: {left_halo:?}"
        );
        assert!(
            right_halo[3] > 4 && right_halo[2] > right_halo[0],
            "group blur should carry the blue side outward: {right_halo:?}"
        );
    }

    #[test]
    fn render_to_image_ignores_filter_definitions_as_paint() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="soft"><rect width="64" height="64" fill="red"/><feGaussianBlur stdDeviation="4"/></filter></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_ignores_filter_definitions_as_paint")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");

        assert_eq!(
            image.pixel(32, 32)[3],
            0,
            "filter definition contents should not render directly"
        );
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
        let Some(renderer) = skip_or_renderer("render_to_image_draws_circle") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
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
        let Some(renderer) = skip_or_renderer("render_to_image_draws_ellipse") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
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
    fn render_to_image_draws_polygon() {
        // A blue triangle filling the centre of an otherwise empty surface.
        let document =
            svg3_dom::parse(r#"<svg><polygon points="32,8 56,52 8,52" fill="blue"/></svg>"#)
                .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_polygon") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!((image.width, image.height), (64, 64));
        // A pixel well inside the triangle is blue.
        let inside = image.pixel(32, 40);
        assert!(
            inside[2] > 200 && inside[0] < 60 && inside[1] < 60,
            "interior pixel not blue: {inside:?}"
        );
        // A corner outside the triangle keeps the transparent clear colour.
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_polyline_fill() {
        // The open point list is closed for SVG fill rendering.
        let document =
            svg3_dom::parse(r#"<svg><polyline points="16,48 32,16 48,48" fill="blue"/></svg>"#)
                .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_polyline_fill") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!((image.width, image.height), (64, 64));
        let inside = image.pixel(32, 36);
        assert!(
            inside[2] > 200 && inside[0] < 60 && inside[1] < 60,
            "inside pixel not blue: {inside:?}"
        );
        assert_eq!(
            image.pixel(32, 56)[3],
            0,
            "background should be transparent"
        );
    }

    #[test]
    fn render_to_image_draws_concave_polyline_fill() {
        // An L-shaped polyline exercises ear clipping through the public
        // parse -> build_scene -> render path. The bottom-right notch must
        // stay transparent while both bars are filled.
        let document = svg3_dom::parse(
            r#"<svg><polyline points="8,8 56,8 56,24 24,24 24,56 8,56" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_concave_polyline_fill") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!((image.width, image.height), (64, 64));

        for (x, y) in [(16, 16), (16, 48), (48, 16)] {
            let px = image.pixel(x, y);
            assert!(
                px[2] > 200 && px[0] < 60 && px[1] < 60,
                "filled pixel ({x}, {y}) not blue: {px:?}"
            );
        }

        for (x, y) in [(48, 48), (4, 4)] {
            assert_eq!(
                image.pixel(x, y)[3],
                0,
                "pixel ({x}, {y}) should be transparent"
            );
        }
    }

    #[test]
    fn render_to_image_draws_line() {
        // A stroked line is rendered through its stroke paint; fill does not
        // apply to `<line>`.
        let document = svg3_dom::parse(
            r#"<svg><line x1="8" y1="32" x2="56" y2="32" stroke="blue" stroke-width="8"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_line") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!((image.width, image.height), (64, 64));
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "line centre pixel not blue: {centre:?}"
        );
        assert_eq!(
            image.pixel(32, 20)[3],
            0,
            "background should be transparent"
        );
    }

    #[test]
    fn render_to_image_draws_path_fill() {
        let document =
            svg3_dom::parse(r#"<svg><path d="M 32 8 L 56 56 L 8 56 Z" fill="blue"/></svg>"#)
                .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_path_fill") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!((image.width, image.height), (64, 64));
        let inside = image.pixel(32, 40);
        assert!(
            inside[2] > 200 && inside[0] < 60 && inside[1] < 60,
            "path interior pixel not blue: {inside:?}"
        );
        assert_eq!(image.pixel(4, 4)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_path_stroke() {
        let document = svg3_dom::parse(
            r#"<svg><path d="M 8 32 H 56" fill="none" stroke="blue" stroke-width="8"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_path_stroke") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!((image.width, image.height), (64, 64));
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "path stroke pixel not blue: {centre:?}"
        );
        assert_eq!(
            image.pixel(32, 20)[3],
            0,
            "background should be transparent"
        );
    }

    #[test]
    fn render_to_image_honors_path_evenodd_fill_rule() {
        let document = svg3_dom::parse(
            r#"<svg><path fill="blue" fill-rule="evenodd" d="M 8 8 H 56 V 56 H 8 Z M 20 20 H 44 V 44 H 20 Z"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_honors_path_evenodd_fill_rule")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let outer = image.pixel(12, 12);
        assert!(
            outer[2] > 200 && outer[0] < 60 && outer[1] < 60,
            "outer ring pixel not blue: {outer:?}"
        );
        assert_eq!(
            image.pixel(32, 32)[3],
            0,
            "evenodd hole should be transparent"
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
        let Some(renderer) = skip_or_renderer("render_to_image_draws_rect_through_camera") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
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
        let Some(renderer) =
            skip_or_renderer("render_to_image_fills_target_with_percent_sized_rect")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
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
        let Some(renderer) = skip_or_renderer("render_to_image_uses_svg_root_size_for_percentages")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
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
