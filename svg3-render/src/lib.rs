//! `svg3-render` — GPU rendering for SVG3 scenes.
//!
//! Turns a parsed [`svg3_dom`] document into GPU geometry and rasterises it
//! with [wgpu](https://crates.io/crates/wgpu).
//!
//! This milestone implements the SVG 1.1 `<rect>`, `<circle>`, `<ellipse>`,
//! `<polygon>`, `<polyline>`, `<line>`, and `<path>` shapes plus referenced
//! `<filter>` elements whose first supported primitive is `<feGaussianBlur>`.
//! [`Renderer::encode_document`] is the single GPU entry point: it walks a
//! parsed document, tessellates every supported shape, applies any
//! referenced Gaussian blur through offscreen render/composite passes, and
//! encodes the draws into a caller-supplied [`wgpu::CommandEncoder`] /
//! [`wgpu::TextureView`]. Both render paths are built on top of it:
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
//! Basic shapes are two-dimensional, so geometry lies in the world plane
//! `z = 0`: by default it is drawn flat through the orthographic
//! [`RenderConfig::projection`], but an optional [`Camera`] on
//! [`RenderConfig`] instead views that plane through a movable 3D
//! perspective camera. The root `<svg width>` / `<svg height>` set the
//! default viewport for percentage lengths; when either is omitted or
//! invalid, the render target dimension is used.
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
pub use renderer::{clear_target, GpuScene, Image, RenderError, Renderer};
pub use scene::{build_scene, document_viewport};
pub use shapes::Viewport;
