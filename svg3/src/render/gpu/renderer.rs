//! The wgpu [`Renderer`] and its render paths.
//!
//! The [`Renderer`] owns and caches the wgpu device, queue, render pipelines
//! and bind-group layouts. It exposes one unified document encoder,
//! [`Renderer::encode_document`], which walks a parsed document, tessellates
//! every supported shape, applies referenced clip-paths, masks, and filter
//! chains via offscreen GPU passes, and composites them into a caller-owned
//! target view through a caller-owned command encoder. The two render entry
//! points are built on top of it:
//!
//! - [`Renderer::render_to_image`] drives the headless path — it allocates
//!   the offscreen sRGB texture, clears it, calls `encode_document`, then
//!   copies the result back into an [`Image`].
//! - A windowed caller (see `app-macos`) brings up its own
//!   [`wgpu::Surface`] and per-frame encoder, clears the swap-chain texture
//!   to its background colour, and calls `encode_document` to draw the
//!   current document into that texture.
//!
//! [`Renderer::create_scene`] + [`Renderer::draw`] remain as a lower-level
//! API for callers that pre-tessellate a [`Mesh`] outside the document walk;
//! the high-level paths go through [`Renderer::encode_document`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use glam::Mat4;
use thiserror::Error;
use wgpu::util::DeviceExt;

use crate::dom::Document;
use crate::render::filters::{
    FilterImage, FilterPrimitive, FilterPrimitiveKind, MaskMode, Merge, PrimitiveSubregion,
};
use crate::render::mesh::PaintServer;
use crate::render::scene::{build_render_plan, MaskRender, RenderOp, ViewportClip};
use crate::render::{document_viewport, Mesh, RenderConfig, Viewport};

use super::clear::{clear_depth, clear_target, DepthTexture, DEPTH_FORMAT};
use super::device::acquire_gpu;
use super::image_decode::{decode_image_href, DecodedImage, GpuImage};
use super::pipeline::{
    build_composite_bind_group_layout, build_filter_bind_group_layout, build_filter_pipeline,
    build_image_bind_group_layout, build_image_pipeline, build_pipeline, premultiplied_alpha_blend,
    target_filter_depth_stencil,
};
use super::readback::{read_back, Image};
use super::uniforms::{
    alpha_mask_uniform, blend_uniform, color_matrix_uniform, component_transfer_uniform,
    convolve_uniform, displacement_uniform, drop_shadow_alpha_uniform, fe_composite_uniform,
    flood_uniform, lighting_uniform, morphology_uniform, offset_uniform, resolve_input,
    resolve_input_uv, subregion_clip_uniform, tile_uniform, turbulence_uniform,
    viewport_clip_uniform, FilterUniform, ImageUniform, TransformUniform,
};

/// Texture format the headless renderer draws into. sRGB-encoded so linear
/// vertex colours are stored correctly; read back as `RGBA8`.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

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
pub(super) struct FilterTexture {
    pub(super) _texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
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
    /// Composite-specific bind-group layout — extends the filter layout
    /// with the source depth texture so the composite shader can write
    /// per-pixel `gl_FragDepth` matching the source geometry's depth.
    composite_bind_group_layout: wgpu::BindGroupLayout,
    /// Comparison-free depth sampler used by the composite shader. Only
    /// the depth value is read — no comparison test happens at sample
    /// time.
    composite_depth_sampler: wgpu::Sampler,
    image_bind_group_layout: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
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
    offset_pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    fe_composite_pipeline: wgpu::RenderPipeline,
    merge_step_pipeline: wgpu::RenderPipeline,
    tile_pipeline: wgpu::RenderPipeline,
    alpha_mask_pipeline: wgpu::RenderPipeline,
    subregion_clip_pipeline: wgpu::RenderPipeline,
    viewport_clip_pipeline: wgpu::RenderPipeline,
    filter_sampler: wgpu::Sampler,
    image_sampler: wgpu::Sampler,
    image_cache: Mutex<BTreeMap<String, Arc<GpuImage>>>,
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
        let composite_bind_group_layout = build_composite_bind_group_layout(&device);
        let image_bind_group_layout = build_image_bind_group_layout(&device);
        // Every filter pipeline except the composite writes to an offscreen
        // texture that has no depth attachment, so `depth_stencil: None`.
        // The composite writes to the final target, which IS depth-attached.
        let blur_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 Gaussian blur pipeline",
            "fs_blur",
            None,
            None,
        );
        let composite_pipeline = build_filter_pipeline(
            &device,
            format,
            &composite_bind_group_layout,
            "svg3 filter composite pipeline",
            "fs_composite",
            Some(premultiplied_alpha_blend()),
            Some(target_filter_depth_stencil()),
        );
        let image_pipeline = build_image_pipeline(&device, format, &image_bind_group_layout);
        let color_matrix_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feColorMatrix pipeline",
            "fs_color_matrix",
            None,
            None,
        );
        let turbulence_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feTurbulence pipeline",
            "fs_turbulence",
            None,
            None,
        );
        let lighting_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feLighting pipeline",
            "fs_lighting",
            None,
            None,
        );
        let morphology_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feMorphology pipeline",
            "fs_morphology",
            None,
            None,
        );
        let flood_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feFlood pipeline",
            "fs_flood",
            None,
            None,
        );
        let drop_shadow_alpha_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feDropShadow alpha pipeline",
            "fs_drop_shadow_alpha",
            None,
            None,
        );
        let drop_shadow_composite_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feDropShadow composite pipeline",
            "fs_drop_shadow_composite",
            None,
            None,
        );
        let displacement_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feDisplacementMap pipeline",
            "fs_displacement",
            None,
            None,
        );
        let convolve_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feConvolveMatrix pipeline",
            "fs_convolve",
            None,
            None,
        );
        let component_transfer_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feComponentTransfer pipeline",
            "fs_component_transfer",
            None,
            None,
        );
        let offset_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feOffset pipeline",
            "fs_offset",
            None,
            None,
        );
        let blend_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feBlend pipeline",
            "fs_blend",
            None,
            None,
        );
        let fe_composite_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feComposite pipeline",
            "fs_fe_composite",
            None,
            None,
        );
        let merge_step_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feMerge step pipeline",
            "fs_merge_step",
            None,
            None,
        );
        let tile_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 feTile pipeline",
            "fs_tile",
            None,
            None,
        );
        let alpha_mask_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 alpha mask pipeline",
            "fs_alpha_mask",
            None,
            None,
        );
        let subregion_clip_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 primitive subregion clip pipeline",
            "fs_subregion_clip",
            None,
            None,
        );
        let viewport_clip_pipeline = build_filter_pipeline(
            &device,
            format,
            &filter_bind_group_layout,
            "svg3 nested viewport clip pipeline",
            "fs_svg_viewport_clip",
            None,
            None,
        );
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
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("svg3 image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let composite_depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("svg3 composite depth sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            // The composite samples depth at integer-pixel UV positions, so
            // nearest filtering suffices and works on backends that don't
            // support filtering depth textures.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            filter_bind_group_layout,
            composite_bind_group_layout,
            composite_depth_sampler,
            image_bind_group_layout,
            blur_pipeline,
            composite_pipeline,
            image_pipeline,
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
            offset_pipeline,
            blend_pipeline,
            fe_composite_pipeline,
            merge_step_pipeline,
            tile_pipeline,
            alpha_mask_pipeline,
            subregion_clip_pipeline,
            viewport_clip_pipeline,
            filter_sampler,
            image_sampler,
            image_cache: Mutex::new(BTreeMap::new()),
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
        let mut paint_servers = Vec::with_capacity(mesh.paint_servers().len() + 1);
        // Paint id 0 is reserved for inline solid vertex colour. The dummy
        // first entry keeps non-zero ids aligned with their storage index.
        paint_servers.push(PaintServer::zeroed());
        paint_servers.extend_from_slice(mesh.paint_servers());
        let paint_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 paint server buffer"),
                contents: bytemuck::cast_slice(&paint_servers),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let transform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("svg3 transform bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: transform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: paint_buffer.as_entire_binding(),
                },
            ],
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
    ///
    /// # Render pass requirements
    ///
    /// As of the depth-aware 2D ↔ 3D rendering work, the shape pipeline
    /// has a [`DEPTH_FORMAT`] depth-stencil attachment, so `pass` MUST
    /// be created with a `depth_stencil_attachment` whose view targets a
    /// `Depth32Float` texture sized to the colour target. Callers that
    /// don't manage their own depth buffer should use
    /// [`Renderer::encode_document`] instead — it owns the depth texture
    /// lifecycle internally.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &GpuScene) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &scene.transform_bind_group, &[]);
        pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
        pass.set_index_buffer(scene.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..scene.index_count, 0, 0..1);
    }

    /// Render every supported SVG shape in `document` headlessly into an
    /// [`Image`] of `config.width × config.height` pixels.
    ///
    /// svg3-only 3D elements, 3D transform functions, and the optional 3D
    /// camera are active only when the root `<svg>` has
    /// `extension="pupiltong"`. Without that opt-in, the document is rendered
    /// as normal SVG with the flat orthographic projection.
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
        let view_projection = if document.svg3_extension_enabled() {
            config.view_projection()
        } else {
            config.projection()
        };

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
    /// Walks `document` once, tessellates every supported shape, applies any
    /// referenced clip-path, mask, or filter through offscreen GPU passes
    /// (allocated lazily against `target_extent`), and composites the result
    /// onto `target` in painter's order. This is the single GPU path shared by the headless
    /// [`Renderer::render_to_image`] and any windowed caller driving its own
    /// surface.
    ///
    /// `target` is **loaded, not cleared** — callers are responsible for
    /// clearing it to their desired background colour before this call.
    /// `viewport` is the basis for percentage lengths in the document;
    /// callers that want SVG root sizing should resolve it via
    /// [`document_viewport`]. `view_projection` maps SVG user space to clip
    /// space; for the standard 2D/3D mix, build it via
    /// [`RenderConfig::view_projection`]. `target_extent` is the texture's
    /// pixel size, used to size the offscreen filter textures.
    pub fn encode_document(
        &self,
        document: &Document,
        viewport: Viewport,
        view_projection: Mat4,
        target: &wgpu::TextureView,
        target_extent: wgpu::Extent3d,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        // Allocate a fresh depth buffer for this encode. 2D content gets a
        // per-shape forward Z bias in `scene::build_render_plan`, so it
        // resolves painter's order via the depth test without z-fighting;
        // 3D primitives ride spatial Z. Cleared once up front so every
        // pass that follows can `LoadOp::Load`.
        let depth = self.create_depth_texture(target_extent, "svg3 target depth");
        clear_depth(encoder, &depth.view, "svg3 target depth clear");

        let plan = build_render_plan(document, viewport);
        for op in &plan {
            match op {
                RenderOp::Mesh(mesh) => {
                    if let Some(scene) = self.create_scene(mesh, view_projection) {
                        self.encode_scene_draw(
                            encoder,
                            target,
                            &depth.view,
                            &scene,
                            "svg3 shape pass",
                        );
                    }
                }
                RenderOp::Clip { mesh, clip } => {
                    self.encode_viewport_clip(
                        encoder,
                        target,
                        &depth.view,
                        mesh,
                        *clip,
                        view_projection,
                        target_extent,
                    );
                }
                RenderOp::Filter {
                    mesh,
                    primitives,
                    clip,
                    mask,
                } => {
                    self.encode_filter_chain(
                        encoder,
                        target,
                        &depth.view,
                        mesh,
                        primitives,
                        clip.as_ref(),
                        mask.as_ref(),
                        viewport,
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
        depth: &wgpu::TextureView,
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
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        self.draw(&mut pass, scene);
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_filter_chain(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_depth: &wgpu::TextureView,
        mesh: &Mesh,
        primitives: &[FilterPrimitive],
        clip: Option<&Mesh>,
        mask: Option<&MaskRender>,
        viewport: Viewport,
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
        // The shape pipeline is depth-tested, so the filter source pass
        // needs its own depth attachment to resolve any 3D occlusion
        // inside the filtered subtree. The composite later samples this
        // texture to forward source depth into the target depth buffer.
        let source_depth = self.create_depth_texture(extent, "svg3 filter source depth");
        clear_depth(
            encoder,
            &source_depth.view,
            "svg3 filter source depth clear",
        );

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
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &source_depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
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

        // SVG 2 render order: clip-path applies BEFORE filter, so apply the
        // rendered clip alpha to SourceGraphic ahead of every primitive pass.
        if let Some(clip_mesh) = clip {
            self.encode_alpha_effect(encoder, &source.view, clip_mesh, view_projection, extent);
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
        // Resolved UV subregion of each primitive's *output*. `[0, 0, 1, 1]`
        // (the full filter region) when the primitive has no authored
        // `x/y/width/height`. Used by `feTile` to wrap inside the input
        // primitive's actual paint rect rather than the whole texture.
        let mut output_uv: Vec<[f32; 4]> = Vec::with_capacity(primitives.len());
        for (i, primitive) in primitives.iter().enumerate() {
            let output_view = &outputs[i].view;
            if let FilterPrimitiveKind::Merge(merge) = &primitive.kind {
                self.encode_merge(
                    encoder,
                    merge,
                    primitive,
                    prev_index,
                    &source,
                    &source_alpha,
                    &outputs,
                    &named,
                    output_view,
                    &scratch.view,
                    extent,
                );
            } else {
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
                let input_uv = resolve_input_uv(&primitive.input, prev_index, &named, &output_uv);
                self.encode_primitive(
                    encoder,
                    primitive,
                    in1_view,
                    in2_view,
                    output_view,
                    &scratch.view,
                    input_uv,
                    viewport,
                    view_projection,
                    extent,
                );
            }
            // SVG 1.1 §15.5: clip the primitive's output to its authored
            // x/y/width/height subregion. Pixels outside are transparent
            // black.
            let primitive_uv = if let Some(subregion) = primitive.subregion {
                let uv = subregion.to_uv(viewport);
                self.encode_subregion_clip(
                    encoder,
                    output_view,
                    &scratch.view,
                    subregion,
                    viewport,
                    extent,
                );
                uv
            } else {
                [0.0, 0.0, 1.0, 1.0]
            };
            output_uv.push(primitive_uv);
            if let Some(name) = primitive.result.as_deref() {
                named.insert(name, i);
            }
            prev_index = Some(i);
        }

        // Composite the last primitive's output onto the destination. Falling
        // back to the raw source covers the "filter with no primitives" case,
        // which `scene` already filters out — but the guard keeps this
        // path safe if a caller invokes the renderer differently.
        let chain_view = match prev_index {
            Some(i) => &outputs[i].view,
            None => &source.view,
        };
        let masked_output;
        let final_view = if let Some(mask) = mask {
            masked_output = Some(self.create_filter_texture(extent, "svg3 mask output"));
            let masked = masked_output
                .as_ref()
                .expect("mask output just initialized");
            self.encode_masked_output(
                encoder,
                chain_view,
                &masked.view,
                mask,
                view_projection,
                extent,
            );
            &masked.view
        } else {
            chain_view
        };
        self.encode_composite_pass(
            encoder,
            final_view,
            &source.view,
            &source_depth.view,
            target,
            target_depth,
            extent,
            view_projection,
            clip.is_some() || mask.is_some(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_viewport_clip(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_depth: &wgpu::TextureView,
        mesh: &Mesh,
        clip: ViewportClip,
        view_projection: Mat4,
        extent: wgpu::Extent3d,
    ) {
        let source = self.create_filter_texture(extent, "svg3 nested viewport source");
        let source_depth = self.create_depth_texture(extent, "svg3 nested viewport source depth");
        clear_depth(
            encoder,
            &source_depth.view,
            "svg3 nested viewport source depth clear",
        );

        if let Some(scene) = self.create_scene(mesh, view_projection) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("svg3 nested viewport source pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &source.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &source_depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            self.draw(&mut pass, &scene);
        } else {
            clear_target(
                encoder,
                &source.view,
                wgpu::Color::TRANSPARENT,
                "svg3 nested viewport empty source clear",
            );
        }

        let scratch = self.create_filter_texture(extent, "svg3 nested viewport clip scratch");
        let passthrough = FilterUniform::passthrough(extent.width, extent.height);
        self.encode_filter_pass(
            encoder,
            &self.color_matrix_pipeline,
            &source.view,
            &source.view,
            &scratch.view,
            &passthrough,
            "svg3 nested viewport source stage",
        );
        let uniform = viewport_clip_uniform(extent, clip);
        self.encode_filter_pass(
            encoder,
            &self.viewport_clip_pipeline,
            &scratch.view,
            &scratch.view,
            &source.view,
            &uniform,
            "svg3 nested viewport clip apply",
        );
        // The source depth texture still contains the clipped-away geometry's
        // depths. This render op is only emitted for normal 2D SVG mode, where
        // later painter-biased shapes advance in front of those stale depths.
        self.encode_composite_pass(
            encoder,
            &source.view,
            &source.view,
            &source_depth.view,
            target,
            target_depth,
            extent,
            view_projection,
            true,
        );
    }

    /// Encode one primitive's GPU pass(es). Reads `in1` / `in2`, writes to
    /// `output`, and may use `scratch` as an internal bounce buffer for
    /// multi-pass primitives. `input_uv` is the resolved UV-space subregion
    /// of the primitive's `in` reference — used by `feTile` to wrap inside
    /// the upstream paint rect rather than the full texture.
    #[allow(clippy::too_many_arguments)]
    fn encode_primitive(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        primitive: &FilterPrimitive,
        in1: &wgpu::TextureView,
        in2: &wgpu::TextureView,
        output: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        input_uv: [f32; 4],
        viewport: Viewport,
        view_projection: Mat4,
        extent: wgpu::Extent3d,
    ) {
        match &primitive.kind {
            FilterPrimitiveKind::GaussianBlur(blur) => {
                self.encode_gaussian_blur_chain(encoder, in1, output, scratch, *blur, extent);
            }
            FilterPrimitiveKind::Image(image) => {
                self.encode_filter_image(encoder, output, image, viewport, view_projection);
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
                let uniform = lighting_uniform(extent, viewport, view_projection, *l, specular);
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
            FilterPrimitiveKind::Offset(o) => {
                let uniform = offset_uniform(extent, *o);
                self.encode_filter_pass(
                    encoder,
                    &self.offset_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feOffset pass",
                );
            }
            FilterPrimitiveKind::Blend(b) => {
                let uniform = blend_uniform(extent, *b);
                self.encode_filter_pass(
                    encoder,
                    &self.blend_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feBlend pass",
                );
            }
            FilterPrimitiveKind::Composite(c) => {
                let uniform = fe_composite_uniform(extent, *c);
                self.encode_filter_pass(
                    encoder,
                    &self.fe_composite_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feComposite pass",
                );
            }
            FilterPrimitiveKind::Tile => {
                let uniform = tile_uniform(extent, input_uv);
                self.encode_filter_pass(
                    encoder,
                    &self.tile_pipeline,
                    in1,
                    in2,
                    output,
                    &uniform,
                    "svg3 feTile pass",
                );
            }
            FilterPrimitiveKind::Merge(_) => {
                // `Merge` is dispatched at the chain level — see
                // `encode_filter_chain` — because it needs to consume an
                // arbitrary list of named inputs.
                unreachable!("feMerge dispatched at chain level");
            }
        }
    }

    /// Clip a primitive's output to its authored subregion (SVG 1.1 §15.5).
    /// Copies `output` into `scratch` via the passthrough pipeline, then
    /// runs `fs_subregion_clip` from `scratch` back into `output`.
    fn encode_subregion_clip(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        subregion: PrimitiveSubregion,
        viewport: Viewport,
        extent: wgpu::Extent3d,
    ) {
        let passthrough = FilterUniform::passthrough(extent.width, extent.height);
        self.encode_filter_pass(
            encoder,
            &self.color_matrix_pipeline,
            output,
            output,
            scratch,
            &passthrough,
            "svg3 subregion clip stage",
        );
        let uniform = subregion_clip_uniform(extent, subregion.to_uv(viewport));
        self.encode_filter_pass(
            encoder,
            &self.subregion_clip_pipeline,
            scratch,
            scratch,
            output,
            &uniform,
            "svg3 subregion clip",
        );
    }

    fn encode_alpha_effect(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        mask_mesh: &Mesh,
        view_projection: Mat4,
        extent: wgpu::Extent3d,
    ) {
        let mask = self.encode_mask_mesh(
            encoder,
            mask_mesh,
            view_projection,
            extent,
            "svg3 clip-path",
        );
        let scratch = self.create_filter_texture(extent, "svg3 alpha effect scratch");
        let passthrough = FilterUniform::passthrough(extent.width, extent.height);
        self.encode_filter_pass(
            encoder,
            &self.color_matrix_pipeline,
            source,
            source,
            &scratch.view,
            &passthrough,
            "svg3 alpha effect source stage",
        );
        let uniform = alpha_mask_uniform(extent, MaskMode::Alpha);
        self.encode_filter_pass(
            encoder,
            &self.alpha_mask_pipeline,
            &scratch.view,
            &mask.view,
            source,
            &uniform,
            "svg3 alpha effect apply",
        );
    }

    fn encode_masked_output(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        output: &wgpu::TextureView,
        mask: &MaskRender,
        view_projection: Mat4,
        extent: wgpu::Extent3d,
    ) {
        let mask_texture =
            self.encode_mask_mesh(encoder, &mask.mesh, view_projection, extent, "svg3 mask");
        let uniform = alpha_mask_uniform(extent, mask.mode);
        self.encode_filter_pass(
            encoder,
            &self.alpha_mask_pipeline,
            input,
            &mask_texture.view,
            output,
            &uniform,
            "svg3 mask apply",
        );
    }

    fn encode_mask_mesh(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &Mesh,
        view_projection: Mat4,
        extent: wgpu::Extent3d,
        label_prefix: &str,
    ) -> FilterTexture {
        let target = self.create_filter_texture(extent, label_prefix);
        let depth = self.create_depth_texture(extent, "svg3 mask depth");
        clear_depth(encoder, &depth.view, "svg3 mask depth clear");

        if let Some(scene) = self.create_scene(mesh, view_projection) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("svg3 mask draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            self.draw(&mut pass, &scene);
        } else {
            clear_target(
                encoder,
                &target.view,
                wgpu::Color::TRANSPARENT,
                "svg3 empty mask clear",
            );
        }

        target
    }

    /// Composite the ordered `<feMergeNode>` inputs of an `<feMerge>` into
    /// `output` in painter (source-over) order. SVG 1.1 §15.13: nodes are
    /// painted in document order, each on top of the accumulated result.
    #[allow(clippy::too_many_arguments)]
    fn encode_merge(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        merge: &Merge,
        primitive: &FilterPrimitive,
        prev_index: Option<usize>,
        source: &FilterTexture,
        source_alpha: &FilterTexture,
        outputs: &[FilterTexture],
        named: &BTreeMap<&str, usize>,
        output: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        extent: wgpu::Extent3d,
    ) {
        // SVG: a `<feMerge>` with no `<feMergeNode>` children is identity on
        // the primitive's own `in`.
        let resolved_nodes: Vec<&wgpu::TextureView> = if merge.nodes.is_empty() {
            vec![resolve_input(
                &primitive.input,
                prev_index,
                source,
                source_alpha,
                outputs,
                named,
            )]
        } else {
            merge
                .nodes
                .iter()
                .map(|input| resolve_input(input, prev_index, source, source_alpha, outputs, named))
                .collect()
        };

        clear_target(
            encoder,
            output,
            wgpu::Color::TRANSPARENT,
            "svg3 feMerge clear",
        );
        // Painter order: node[0] is painted first (bottom), node[n-1] last
        // (top). For each node we run `merge_step(in1=node, in2=accum)` →
        // `dst = node + accum * (1 - node.a)`. We ping-pong, leaving the
        // final accumulator in `output` (an extra passthrough copy handles
        // the even-count case).
        let uniform = FilterUniform::composite(extent.width, extent.height);
        let mut accum_in_output = false;
        for (i, node_view) in resolved_nodes.iter().enumerate() {
            let (dst_view, accum_view): (&wgpu::TextureView, &wgpu::TextureView) =
                if accum_in_output {
                    (scratch, output)
                } else {
                    (output, scratch)
                };
            if i == 0 {
                clear_target(
                    encoder,
                    scratch,
                    wgpu::Color::TRANSPARENT,
                    "svg3 feMerge scratch clear",
                );
            }
            self.encode_filter_pass(
                encoder,
                &self.merge_step_pipeline,
                node_view,
                accum_view,
                dst_view,
                &uniform,
                "svg3 feMerge step",
            );
            accum_in_output = !accum_in_output;
        }
        if !accum_in_output {
            let passthrough = FilterUniform::passthrough(extent.width, extent.height);
            self.encode_filter_pass(
                encoder,
                &self.color_matrix_pipeline,
                scratch,
                scratch,
                output,
                &passthrough,
                "svg3 feMerge passthrough",
            );
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
        blur: crate::render::filters::GaussianBlur,
        extent: wgpu::Extent3d,
    ) {
        let want_x = blur.std_deviation_x > 0.0;
        let want_y = blur.std_deviation_y > 0.0;

        if !want_x && !want_y {
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

    fn encode_filter_image(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::TextureView,
        image: &FilterImage,
        viewport: Viewport,
        view_projection: Mat4,
    ) {
        let Some(gpu_image) = self.gpu_image(&image.href) else {
            clear_target(
                encoder,
                destination,
                wgpu::Color::TRANSPARENT,
                "svg3 skipped feImage clear",
            );
            return;
        };
        let Some(rect) = image.resolve_rect(viewport, gpu_image.width, gpu_image.height) else {
            clear_target(
                encoder,
                destination,
                wgpu::Color::TRANSPARENT,
                "svg3 skipped feImage rect clear",
            );
            return;
        };

        self.encode_image_draw(
            encoder,
            destination,
            &gpu_image,
            rect,
            view_projection,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
        shadow: crate::render::filters::DropShadow,
        extent: wgpu::Extent3d,
    ) {
        // 1) Colour the source alpha at offset uv into `scratch`.
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
        // 3) Blur Y: output -> scratch.
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
        // 4) Composite shadow (scratch) under input image, into output.
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

    #[allow(clippy::too_many_arguments)]
    fn encode_composite_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        chain_output: &wgpu::TextureView,
        source_view: &wgpu::TextureView,
        source_depth: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        destination_depth: &wgpu::TextureView,
        extent: wgpu::Extent3d,
        view_projection: Mat4,
        discard_zero_alpha: bool,
    ) {
        let mut uniform =
            FilterUniform::composite_with_projection(extent.width, extent.height, view_projection);
        uniform.flags = u32::from(discard_zero_alpha);
        let bind_group = self.create_composite_bind_group(
            chain_output,
            source_view,
            source_depth,
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
            // `LessEqual` + depth-write: the composite's per-pixel
            // `frag_depth` (sampled from the source depth texture) updates
            // the target depth buffer so subsequent 3D draws depth-test
            // against the filter source's spatial Z.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: destination_depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn create_composite_bind_group(
        &self,
        source: &wgpu::TextureView,
        source2: &wgpu::TextureView,
        source_depth: &wgpu::TextureView,
        uniform: &FilterUniform,
        label: &str,
    ) -> wgpu::BindGroup {
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 filter composite uniform"),
                contents: bytemuck::bytes_of(uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.composite_bind_group_layout,
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(source_depth),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.composite_depth_sampler),
                },
            ],
        })
    }

    fn encode_image_draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::TextureView,
        image: &GpuImage,
        rect: crate::render::filters::ImageRect,
        view_projection: Mat4,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        let uniform = ImageUniform::new(view_projection, rect);
        let bind_group =
            self.create_image_bind_group(&image.view, &uniform, "svg3 image bind group");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("svg3 filtered image pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.image_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..6, 0..1);
    }

    fn gpu_image(&self, href: &str) -> Option<Arc<GpuImage>> {
        {
            let cache = self
                .image_cache
                .lock()
                .expect("svg3 image cache lock should not be poisoned");
            if let Some(image) = cache.get(href) {
                return Some(Arc::clone(image));
            }
        }

        let decoded = match decode_image_href(href) {
            Ok(decoded) => decoded,
            Err(error) => {
                log::debug!("svg3::render: skipping <feImage>: {error}");
                return None;
            }
        };
        let uploaded = Arc::new(self.upload_image(decoded));

        let mut cache = self
            .image_cache
            .lock()
            .expect("svg3 image cache lock should not be poisoned");
        Some(Arc::clone(
            cache
                .entry(href.to_owned())
                .or_insert_with(|| Arc::clone(&uploaded)),
        ))
    }

    fn upload_image(&self, image: DecodedImage) -> GpuImage {
        let extent = wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("svg3 feImage texture"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            extent,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        GpuImage {
            _texture: texture,
            view,
            width: image.width,
            height: image.height,
        }
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

    fn create_image_bind_group(
        &self,
        image: &wgpu::TextureView,
        uniform: &ImageUniform,
        label: &str,
    ) -> wgpu::BindGroup {
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("svg3 image uniform"),
                contents: bytemuck::bytes_of(uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.image_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(image),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.image_sampler),
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

    /// Allocate a depth texture sized to `extent` and return a view into
    /// it. The first render pass that attaches it should clear depth to
    /// 1.0 (the far plane) so the depth test starts from a known state.
    ///
    /// `TEXTURE_BINDING` is included alongside `RENDER_ATTACHMENT` so the
    /// filter composite shader can sample the filter source's depth and
    /// forward it as `frag_depth`.
    fn create_depth_texture(&self, extent: wgpu::Extent3d, label: &str) -> DepthTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        DepthTexture {
            _texture: texture,
            view,
        }
    }
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
    use crate::render::Camera;

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
    fn render_to_image_draws_blue_rect() {
        let document = crate::dom::parse(
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
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "centre pixel not blue: {centre:?}"
        );
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_paints_linear_gradient_fill() {
        let document = crate::dom::parse(
            r##"<svg><defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="16" y1="0" x2="48" y2="0"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><rect x="16" y="16" width="32" height="32" fill="url(#g)"/></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_paints_linear_gradient_fill") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");

        let left = image.pixel(18, 32);
        let right = image.pixel(46, 32);
        assert!(
            left[0] > 180 && left[2] < 90,
            "left side should be red-dominant, got {left:?}"
        );
        assert!(
            right[2] > 180 && right[0] < 90,
            "right side should be blue-dominant, got {right:?}"
        );
    }

    #[test]
    fn render_to_image_paints_radial_gradient_fill() {
        let document = crate::dom::parse(
            r##"<svg><defs><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="32" cy="32" r="20"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></radialGradient></defs><circle cx="32" cy="32" r="20" fill="url(#g)"/></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_paints_radial_gradient_fill") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");

        let centre = image.pixel(32, 32);
        let edge = image.pixel(49, 32);
        assert!(
            centre[0] > 180 && centre[2] < 90,
            "radial centre should be red, got {centre:?}"
        );
        assert!(
            edge[2] > edge[0] && edge[2] > 90,
            "radial edge should shift toward blue, got {edge:?}"
        );
    }

    #[test]
    fn render_to_image_paints_pattern_fill() {
        let document = crate::dom::parse(
            r##"<svg><defs><pattern id="p" patternUnits="userSpaceOnUse" width="8" height="8"><rect width="4" height="8" fill="red"/><rect x="4" width="4" height="8" fill="blue"/></pattern></defs><rect x="16" y="16" width="32" height="32" fill="url(#p)"/></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_paints_pattern_fill") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");

        let red = image.pixel(18, 32);
        let blue = image.pixel(22, 32);
        let repeated_red = image.pixel(26, 32);
        assert!(
            red[0] > 180 && red[2] < 90,
            "first pattern stripe should be red, got {red:?}"
        );
        assert!(
            blue[2] > 180 && blue[0] < 90,
            "second pattern stripe should be blue, got {blue:?}"
        );
        assert!(
            repeated_red[0] > 180 && repeated_red[2] < 90,
            "pattern should repeat horizontally, got {repeated_red:?}"
        );
    }

    #[test]
    fn render_to_image_applies_gaussian_blur_filter() {
        let filtered = crate::dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4"/></filter><rect x="24" y="24" width="16" height="16" fill="blue" filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let unfiltered = crate::dom::parse(
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
        let document = crate::dom::parse(
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
    fn render_to_image_applies_path_clip_path_without_filter() {
        let document = crate::dom::parse(
            r##"<svg><defs><clipPath id="right"><path d="M 32 0 H 64 V 64 H 32 Z"/></clipPath></defs><rect width="64" height="64" fill="blue" clip-path="url(#right)"/></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_applies_path_clip_path_without_filter")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("clipped render failed");

        assert_eq!(image.pixel(16, 32)[3], 0, "left side should be clipped");
        let right = image.pixel(48, 32);
        assert!(
            right[2] > 180 && right[3] > 200,
            "right side should remain blue: {right:?}"
        );
    }

    #[test]
    fn render_to_image_applies_ancestor_transform_to_clip_path() {
        let document = crate::dom::parse(
            r##"<svg><defs><clipPath id="left"><rect width="16" height="64"/></clipPath></defs><g transform="translate(32 0)"><rect width="32" height="64" fill="blue" clip-path="url(#left)"/></g></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_applies_ancestor_transform_to_clip_path")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("transformed clipped render failed");

        assert_eq!(image.pixel(24, 32)[3], 0, "left side should be empty");
        let clipped = image.pixel(40, 32);
        assert!(
            clipped[2] > 180 && clipped[3] > 200,
            "translated clip should reveal the left half of the translated rect: {clipped:?}"
        );
        assert_eq!(
            image.pixel(56, 32)[3],
            0,
            "right half should be clipped by the translated clip"
        );
    }

    #[test]
    fn render_to_image_applies_luminance_mask_without_filter() {
        let document = crate::dom::parse(
            r##"<svg><defs><mask id="reveal"><rect x="32" y="0" width="32" height="64" fill="white"/></mask></defs><rect width="64" height="64" fill="red" mask="url(#reveal)"/></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_applies_luminance_mask_without_filter")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("masked render failed");

        assert_eq!(image.pixel(16, 32)[3], 0, "left side should be masked");
        let right = image.pixel(48, 32);
        assert!(
            right[0] > 180 && right[3] > 200,
            "right side should remain red: {right:?}"
        );
    }

    #[test]
    fn render_to_image_applies_ancestor_transform_to_mask() {
        let document = crate::dom::parse(
            r##"<svg><defs><mask id="left"><rect width="16" height="64" fill="white"/></mask></defs><g transform="translate(32 0)"><rect width="32" height="64" fill="red" mask="url(#left)"/></g></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_applies_ancestor_transform_to_mask")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("transformed masked render failed");

        assert_eq!(image.pixel(24, 32)[3], 0, "left side should be empty");
        let masked = image.pixel(40, 32);
        assert!(
            masked[0] > 180 && masked[3] > 200,
            "translated mask should reveal the left half of the translated rect: {masked:?}"
        );
        assert_eq!(
            image.pixel(56, 32)[3],
            0,
            "right half should be masked by the translated mask"
        );
    }

    #[test]
    fn render_to_image_applies_alpha_mask_type() {
        let document = crate::dom::parse(
            r##"<svg><defs><mask id="reveal" mask-type="alpha"><rect x="32" y="0" width="32" height="64" fill="black"/></mask></defs><rect width="64" height="64" fill="red" mask="url(#reveal)"/></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_applies_alpha_mask_type") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("alpha masked render failed");

        assert_eq!(image.pixel(16, 32)[3], 0, "left side should be masked");
        let right = image.pixel(48, 32);
        assert!(
            right[0] > 180 && right[3] > 200,
            "black alpha mask content should reveal in alpha mode: {right:?}"
        );
    }

    #[test]
    fn render_to_image_ignores_filter_definitions_as_paint() {
        let document = crate::dom::parse(
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
        let document =
            crate::dom::parse(r#"<svg><circle cx="32" cy="32" r="20" fill="blue"/></svg>"#)
                .unwrap();
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
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "centre pixel not blue: {centre:?}"
        );
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_ellipse() {
        let document = crate::dom::parse(
            r#"<svg><ellipse cx="32" cy="32" rx="28" ry="14" fill="blue"/></svg>"#,
        )
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
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "centre pixel not blue: {centre:?}"
        );
        let on_x_axis = image.pixel(54, 32);
        assert!(
            on_x_axis[2] > 200 && on_x_axis[0] < 60 && on_x_axis[1] < 60,
            "x-axis pixel not blue: {on_x_axis:?}"
        );
        assert_eq!(
            image.pixel(32, 54)[3],
            0,
            "pixel beyond ry should be transparent"
        );
        assert_eq!(
            image.pixel(58, 44)[3],
            0,
            "bbox corner outside the curve should be transparent"
        );
    }

    #[test]
    fn render_to_image_draws_polygon() {
        let document =
            crate::dom::parse(r#"<svg><polygon points="32,8 56,52 8,52" fill="blue"/></svg>"#)
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
        let inside = image.pixel(32, 40);
        assert!(
            inside[2] > 200 && inside[0] < 60 && inside[1] < 60,
            "interior pixel not blue: {inside:?}"
        );
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_polyline_fill() {
        let document =
            crate::dom::parse(r#"<svg><polyline points="16,48 32,16 48,48" fill="blue"/></svg>"#)
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
            crate::dom::parse(r#"<svg><path d="M 32 8 L 56 56 L 8 56 Z" fill="blue"/></svg>"#)
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
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
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><rect x="16" y="16" width="32" height="32" fill="blue"/></svg>"#,
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
    fn render_to_image_ignores_camera_without_extension_attribute() {
        let document = crate::dom::parse(
            r#"<svg width="64" height="64"><rect x="16" y="16" width="32" height="32" fill="blue"/></svg>"#,
        )
        .unwrap();
        let orthographic = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let mut angled = Camera::facing(64, 64);
        angled.eye.x += 48.0;
        let with_camera = RenderConfig {
            camera: Some(angled),
            ..orthographic
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_ignores_camera_without_extension_attribute")
        else {
            return;
        };

        let flat = renderer
            .render_to_image(&document, orthographic)
            .expect("orthographic render failed");
        let camera = renderer
            .render_to_image(&document, with_camera)
            .expect("camera render failed");

        assert_eq!(
            flat.pixels, camera.pixels,
            "normal SVG documents should ignore the 3D camera"
        );
    }

    #[test]
    fn render_to_image_skips_cube_without_extension_attribute() {
        let document = crate::dom::parse(
            r#"<svg width="64" height="64"><cube cx="32" cy="32" cz="0" size="32" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_skips_cube_without_extension_attribute")
        else {
            return;
        };

        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");

        assert_eq!(
            image.pixel(32, 32)[3],
            0,
            "plain SVG documents must not paint svg3 <cube> elements"
        );
    }

    #[test]
    fn render_to_image_skips_surface_and_children_without_extension_attribute() {
        let document = crate::dom::parse(
            r#"<svg width="64" height="64"><surface d="M 0 L 1" fill="blue"><path d="M 16 16 H 48 V 48 H 16 Z"/><path d="M 16 16 H 48 V 48 H 16 Z" transform="translateZ(20)"/></surface></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer(
            "render_to_image_skips_surface_and_children_without_extension_attribute",
        ) else {
            return;
        };

        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");

        assert_eq!(
            image.pixel(32, 32)[3],
            0,
            "plain SVG documents must not paint svg3 <surface> elements or their child paths"
        );
    }

    #[test]
    fn render_to_image_draws_cube_orthographic() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><cube cx="32" cy="32" cz="0" size="32" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_cube_orthographic") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "cube centre pixel not blue: {centre:?}"
        );
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_cube_through_camera() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><cube cx="32" cy="32" cz="0" size="20" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            camera: Some(Camera::facing(64, 64)),
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_cube_through_camera") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "cube centre pixel not blue: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_skips_degenerate_cube() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><cube cx="32" cy="32" size="32" depth="0" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_skips_degenerate_cube") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!(image.pixel(32, 32)[3], 0);
    }

    #[test]
    fn render_to_image_draws_ellipsoid_orthographic() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><ellipsoid cx="32" cy="32" cz="0" r="16" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_ellipsoid_orthographic")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "ellipsoid centre pixel not blue: {centre:?}"
        );
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_ellipsoid_through_camera() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><ellipsoid cx="32" cy="32" cz="0" r="14" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            camera: Some(Camera::facing(64, 64)),
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_ellipsoid_through_camera")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "ellipsoid centre pixel not blue: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_skips_degenerate_ellipsoid() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><ellipsoid cx="32" cy="32" r="16" rz="0" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_skips_degenerate_ellipsoid") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!(image.pixel(32, 32)[3], 0);
    }

    #[test]
    fn render_to_image_draws_cylinder_orthographic() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><cylinder cx="32" cy="32" cz="0" r="16" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_cylinder_orthographic") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "cylinder centre pixel not blue: {centre:?}"
        );
        assert_eq!(image.pixel(2, 2)[3], 0, "background should be transparent");
    }

    #[test]
    fn render_to_image_draws_cylinder_through_camera() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><cylinder cx="32" cy="32" cz="0" r="14" depth="28" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            camera: Some(Camera::facing(64, 64)),
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_draws_cylinder_through_camera")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60 && centre[1] < 60,
            "cylinder centre pixel not blue: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_skips_degenerate_cylinder() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><cylinder cx="32" cy="32" r="16" depth="0" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_skips_degenerate_cylinder") else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        assert_eq!(image.pixel(32, 32)[3], 0);
    }

    #[test]
    fn render_to_image_2d_rect_occludes_ellipsoid_fully_behind_z0() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><rect x="16" y="16" width="32" height="32" fill="red"/><ellipsoid cx="32" cy="32" cz="-20" r="15" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_2d_rect_occludes_ellipsoid_fully_behind_z0")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[0] > 200 && centre[2] < 60,
            "2D rect at z=0 should occlude an ellipsoid fully behind: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_2d_rect_occludes_cube_behind_z0() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><rect x="32" y="16" width="32" height="32" fill="red"/><cube cx="32" cy="32" cz="0" size="30" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) = skip_or_renderer("render_to_image_2d_rect_occludes_cube_behind_z0")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let left = image.pixel(20, 32);
        assert!(
            left[2] > 200 && left[0] < 60,
            "left-of-rect should be cube blue: {left:?}"
        );
        let overlap = image.pixel(40, 32);
        assert!(
            overlap[2] > 200 && overlap[0] < 60,
            "overlap: cube front-half should occlude rect: {overlap:?}"
        );
        let right = image.pixel(60, 20);
        assert!(
            right[0] > 200 && right[2] < 60,
            "right-of-cube should be rect red: {right:?}"
        );
    }

    #[test]
    fn render_to_image_2d_rect_fully_occludes_cube_fully_behind_z0() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><rect x="16" y="16" width="32" height="32" fill="red"/><cube cx="32" cy="32" cz="-15" size="20" fill="blue"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_2d_rect_fully_occludes_cube_fully_behind_z0")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[0] > 200 && centre[2] < 60,
            "2D rect at z=0 should occlude cube fully behind z=0: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_cube_in_front_occludes_later_2d_rect() {
        let document = crate::dom::parse(
            r#"<svg extension="pupiltong" width="64" height="64"><cube cx="32" cy="32" cz="20" size="20" fill="blue"/><rect x="16" y="16" width="32" height="32" fill="red"/></svg>"#,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_cube_in_front_occludes_later_2d_rect")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[2] > 200 && centre[0] < 60,
            "earlier cube in front of z=0 should occlude a later 2D rect: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_filtered_rect_occludes_cube_behind_z0() {
        let document = crate::dom::parse(
            r##"<svg extension="pupiltong" width="64" height="64"><filter id="soft"><feGaussianBlur stdDeviation="2"/></filter><rect x="16" y="16" width="32" height="32" fill="red" filter="url(#soft)"/><cube cx="32" cy="32" cz="-15" size="20" fill="blue"/></svg>"##,
        )
        .unwrap();
        let config = RenderConfig {
            width: 64,
            height: 64,
            ..RenderConfig::default()
        };
        let Some(renderer) =
            skip_or_renderer("render_to_image_filtered_rect_occludes_cube_behind_z0")
        else {
            return;
        };
        let image = renderer
            .render_to_image(&document, config)
            .expect("headless render failed");
        let centre = image.pixel(32, 32);
        assert!(
            centre[0] > 100 && centre[2] < 80,
            "filtered rect at z=0 should occlude a cube fully behind: {centre:?}"
        );
    }

    #[test]
    fn render_to_image_fills_target_with_percent_sized_rect() {
        let document =
            crate::dom::parse(r#"<svg><rect width="100%" height="100%" fill="blue"/></svg>"#)
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
        let document = crate::dom::parse(
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
