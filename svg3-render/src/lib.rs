//! `svg3-render` — GPU rendering for SVG3 scenes.
//!
//! Turns a parsed [`svg3_dom`] document into GPU geometry and rasterises it
//! with [wgpu](https://crates.io/crates/wgpu).
//!
//! This milestone implements the SVG 1.1 `<rect>`, `<circle>`, `<ellipse>`,
//! `<polygon>`, `<polyline>`, `<line>`, and `<path>` shapes: [`build_scene`]
//! tessellates every such shape in a document into a [`Mesh`], and
//! [`Renderer::render_to_image`] rasterises them headlessly — no window or
//! swapchain — into an [`Image`]. Headless rendering also supports referenced
//! `<filter>` elements whose first supported primitive is `<feGaussianBlur>`,
//! applying the blur on the GPU with offscreen render/composite passes. The
//! same [`Renderer`] also drives a
//! caller-owned windowed render pass via [`Renderer::create_scene`] and
//! [`Renderer::draw`] — see the `app-macos` demo. Basic shapes are
//! two-dimensional, so geometry lies in the world plane `z = 0`: by default
//! it is drawn flat through the orthographic [`RenderConfig::projection`],
//! but an optional [`Camera`] on [`RenderConfig`] instead views that plane
//! through a movable 3D perspective camera. The root `<svg width>` /
//! `<svg height>` set the default viewport for percentage lengths; when
//! either is omitted or invalid, the render target dimension is used.
//!
//! The crate is split into focused modules: `mesh` holds the [`Vertex`] /
//! [`Mesh`] geometry model that shapes tessellate into; `shapes` resolves
//! and tessellates each supported element; `scene` walks the document
//! ([`build_scene`]); `camera` holds [`RenderConfig`] and the movable 3D
//! [`Camera`]; `filters` resolves supported SVG filter definitions; and
//! `renderer` owns the wgpu [`Renderer`], its [`GpuScene`] GPU buffers,
//! filter post-processing pipelines, and headless [`Image`] readback.

mod camera;
mod filters;
mod mesh;
mod renderer;
mod scene;
mod shapes;

pub use camera::{Camera, RenderConfig};
pub use mesh::{Mesh, Vertex};
pub use renderer::{GpuScene, Image, RenderError, Renderer};
pub use scene::{build_scene, document_viewport};
pub use shapes::Viewport;
