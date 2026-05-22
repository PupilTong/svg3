//! The wgpu [`Renderer`] and its render paths.
//!
//! The [`Renderer`] owns and caches the wgpu device, queue, render pipelines
//! and bind-group layouts. It drives two paths over the same GPU state: the
//! headless [`Renderer::render_to_image`], which rasterises a document into
//! an [`Image`] and applies supported filter passes, and the caller-owned
//! windowed pass built from [`Renderer::create_scene`] + [`Renderer::draw`].
//! A tessellated scene's GPU buffers live in [`GpuScene`].

use glam::Mat4;
use svg3_dom::Document;
use thiserror::Error;
use wgpu::util::DeviceExt;

use crate::filters::GaussianBlur;
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
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FilterUniform {
    /// One source texel in normalised texture coordinates.
    texel_size: [f32; 2],
    /// Blur axis: `[1, 0]` for horizontal, `[0, 1]` for vertical.
    direction: [f32; 2],
    /// Gaussian standard deviation for this axis, in render pixels.
    sigma: f32,
    /// One-sided sample radius, derived from `sigma`.
    radius: u32,
    /// Explicit padding so the Rust and WGSL uniform layouts stay aligned.
    _pad: [u32; 2],
}

impl FilterUniform {
    fn blur(width: u32, height: u32, direction: [f32; 2], sigma: f32) -> Self {
        let radius = (sigma * 3.0).ceil().clamp(0.0, MAX_BLUR_RADIUS as f32) as u32;
        Self {
            texel_size: [1.0 / width as f32, 1.0 / height as f32],
            direction,
            sigma,
            radius,
            _pad: [0; 2],
        }
    }

    fn composite(width: u32, height: u32) -> Self {
        Self {
            texel_size: [1.0 / width as f32, 1.0 / height as f32],
            direction: [0.0, 0.0],
            sigma: 0.0,
            radius: 0,
            _pad: [0; 2],
        }
    }
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
    /// [`create_scene`](Renderer::create_scene) and [`draw`](Renderer::draw);
    /// [`render_to_image`](Renderer::render_to_image) additionally requires
    /// `format` to be sRGB `RGBA8`.
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
        let filter_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("svg3 filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
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
            blur_pipeline,
            composite_pipeline,
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
    /// Rasterises the tessellated scene into an offscreen sRGB texture and
    /// reads the pixels back; the surface is cleared to transparent first.
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
        let plan = build_render_plan(document, document_viewport(document, target_viewport));
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
        {
            // Execute the target clear once before replaying the render plan;
            // later direct and filtered ops load from this cleared texture.
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("svg3 target clear pass"),
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
        }
        for op in &plan {
            match op {
                RenderOp::Mesh(mesh) => {
                    if let Some(scene) = self.create_scene(mesh, view_projection) {
                        self.encode_scene_draw(&mut encoder, &view, &scene, "svg3 shape pass");
                    }
                }
                RenderOp::GaussianBlur { mesh, blur } => {
                    self.encode_gaussian_blur(
                        &mut encoder,
                        &view,
                        mesh,
                        *blur,
                        view_projection,
                        extent,
                    );
                }
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
        self.queue.submit(std::iter::once(encoder.finish()));

        let pixels = read_back(&self.device, &readback, width, height, padded_bytes_per_row)?;
        Ok(Image {
            width,
            height,
            pixels,
        })
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

    fn encode_gaussian_blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        mesh: &Mesh,
        blur: GaussianBlur,
        view_projection: Mat4,
        extent: wgpu::Extent3d,
    ) {
        let source = self.create_filter_texture(extent, "svg3 filter source");
        let ping = self.create_filter_texture(extent, "svg3 filter ping");

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
        }

        let mut current = &source;
        let mut wrote_ping = false;
        if blur.std_deviation_x > 0.0 {
            self.encode_blur_pass(
                encoder,
                &source.view,
                &ping.view,
                extent,
                [1.0, 0.0],
                blur.std_deviation_x,
            );
            current = &ping;
            wrote_ping = true;
        }
        if blur.std_deviation_y > 0.0 {
            let destination = if wrote_ping { &source } else { &ping };
            self.encode_blur_pass(
                encoder,
                &current.view,
                &destination.view,
                extent,
                [0.0, 1.0],
                blur.std_deviation_y,
            );
            current = destination;
        }
        self.encode_composite_pass(encoder, &current.view, target, extent);
    }

    fn encode_blur_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        extent: wgpu::Extent3d,
        direction: [f32; 2],
        sigma: f32,
    ) {
        let uniform = FilterUniform::blur(extent.width, extent.height, direction, sigma);
        let bind_group = self.create_filter_bind_group(source, &uniform, "svg3 blur bind group");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("svg3 Gaussian blur pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.blur_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn encode_composite_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        extent: wgpu::Extent3d,
    ) {
        let uniform = FilterUniform::composite(extent.width, extent.height);
        let bind_group =
            self.create_filter_bind_group(source, &uniform, "svg3 filter composite bind group");
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
        assert_eq!(
            std::mem::size_of::<FilterUniform>(),
            8 * std::mem::size_of::<f32>()
        );
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
