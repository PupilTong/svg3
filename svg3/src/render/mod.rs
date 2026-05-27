//! GPU rendering for SVG3 scenes.
//!
//! Turns a parsed [`crate::dom`] document into GPU geometry and rasterises it
//! with [wgpu](https://crates.io/crates/wgpu).
//!
//! This milestone implements the SVG 1.1 basic shapes (`<rect>`, `<circle>`,
//! `<ellipse>`, `<polygon>`, `<polyline>`, `<line>`, `<path>`), SVG paint
//! servers (`<linearGradient>`, `<radialGradient>`, `<pattern>`, `<stop>`),
//! svg3's 3D `<cube>`, `<ellipsoid>`, `<cylinder>`, and `<surface>` primitives
//! when the root has `extension="pupiltong"`, plus
//! referenced `<filter>` elements composed from a multi-primitive chain
//! executed end-to-end on the GPU. [`Renderer::encode_document`] is the
//! single GPU entry point; both render paths are built on top of it:
//! [`Renderer::render_to_image`] wraps it with offscreen-texture allocation
//! and CPU readback to produce an [`Image`]; a windowed caller wraps it with
//! surface acquisition and present.
//!
//! Module layout:
//! - [`camera`] — [`Camera`] and [`RenderConfig`] (target-surface +
//!   view-projection setup).
//! - [`transform`] — the SVG `transform` attribute parser used by
//!   `<surface>` paths.
//! - [`mesh`] — the [`Vertex`] / [`Mesh`] geometry data model.
//! - [`shapes`] — per-shape geometry resolution and tessellation, one
//!   submodule per element kind.
//! - [`paint`] — paint server definition / reference resolution
//!   (`<linearGradient>`, `<radialGradient>`, `<pattern>`).
//! - [`filters`] — `<filter>` definition resolution into a primitive chain
//!   plus the `<clipPath>` resolver.
//! - [`scene`] — walks the document and produces a render plan (the painter-
//!   order mesh / filter operations the GPU executes).
//! - [`gpu`] — wgpu pipelines and the [`Renderer`] that drives them.

mod camera;
mod filters;
mod mesh;
mod paint;
mod scene;
mod shapes;
mod transform;

pub mod gpu;

pub use camera::{Camera, RenderConfig};
pub use gpu::{clear_target, GpuScene, Image, RenderError, Renderer, DEPTH_FORMAT};
pub use mesh::{Mesh, Vertex};
pub use scene::{build_scene, document_viewport};
pub use shapes::Viewport;
