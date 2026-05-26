//! `svg3-render` — GPU rendering for SVG3 scenes.
//!
//! Turns a parsed [`svg3_dom`] document into GPU geometry and rasterises it
//! with [wgpu](https://crates.io/crates/wgpu).
//!
//! This milestone implements the SVG 1.1 `<rect>`, `<circle>`, `<ellipse>`,
//! `<polygon>`, `<polyline>`, `<line>`, and `<path>` shapes, SVG paint
//! servers (`<linearGradient>`, `<radialGradient>`, `<pattern>`, and
//! `<stop>`), svg3's 3D `<cube>` and `<ellipsoid>` primitives, plus
//! referenced `<filter>` elements composed from a multi-primitive chain
//! executed end-to-end on the GPU. The supported primitives are
//! `<feGaussianBlur>`,
//! `<feImage>`, `<feColorMatrix>`, `<feTurbulence>`, `<feSpecularLighting>`,
//! `<feDiffuseLighting>`, `<feMorphology>`, `<feFlood>`, `<feDropShadow>`,
//! `<feDisplacementMap>`, `<feConvolveMatrix>`, and `<feComponentTransfer>`.
//! Filter primitives ship as fragment-shader pipelines backed by
//! `filter.wgsl`, while PNG data-URL `<feImage>` sources decode/upload
//! lazily when a referenced filter paints and are sampled through
//! `image.wgsl`. [`Renderer::encode_document`]
//! is the single GPU entry point: it walks a parsed document, tessellates
//! every supported shape, resolves referenced paint servers, runs each
//! referenced filter's primitive chain through offscreen ping/pong textures,
//! and encodes the draws into a
//! caller-supplied [`wgpu::CommandEncoder`] / [`wgpu::TextureView`]. Both
//! render paths are built on top of it:
//! [`Renderer::render_to_image`] wraps it with offscreen-texture allocation,
//! a transparent [`clear_target`], and CPU readback to produce an [`Image`];
//! a windowed caller (see the `app-macos` demo) wraps it with surface
//! acquisition, [`clear_target`] to its background colour, and a `present`.
//!
//! [`Renderer::create_scene`] + [`Renderer::draw`] remain available for
//! callers that want to pre-tessellate and re-draw a [`Mesh`] without going
//! through the document walk; new code should prefer
//! [`Renderer::encode_document`].
//!
//! Basic shapes are two-dimensional, so their geometry lies in the world
//! plane `z = 0`; the 3D `<cube>` and `<ellipsoid>` primitives occupy the
//! cuboid / implicit-surface volume implied by their attributes. By
//! default content is drawn through the orthographic
//! [`RenderConfig::projection`] — collapsing the 3D primitives to their
//! axis-aligned 2D projection — but an optional [`Camera`] on
//! [`RenderConfig`] instead views the scene through a movable 3D
//! perspective camera, where the depth dimension is visible. The root
//! `<svg width>` / `<svg height>` set the default viewport for percentage
//! lengths; when either is omitted or invalid, the render target
//! dimension is used.
//!
//! The crate is split into focused modules: `mesh` holds the [`Vertex`] /
//! [`Mesh`] geometry model that shapes tessellate into; `shapes` resolves
//! and tessellates each supported element; `paint` resolves SVG paint-server
//! definitions; `scene` walks the document ([`build_scene`]); `camera` holds
//! [`RenderConfig`] and the movable 3D [`Camera`]; `filters` resolves
//! supported SVG filter definitions; and `renderer` owns the wgpu
//! [`Renderer`], its [`GpuScene`] GPU buffers, filter post-processing
//! pipelines, and headless [`Image`] readback.

mod camera;
mod filters;
mod mesh;
mod paint;
mod renderer;
mod scene;
mod shapes;
mod transform;

pub use camera::{Camera, RenderConfig};
pub use mesh::{Mesh, Vertex};
pub use renderer::{clear_target, GpuScene, Image, RenderError, Renderer, DEPTH_FORMAT};
pub use scene::{build_scene, document_viewport};
pub use shapes::Viewport;
