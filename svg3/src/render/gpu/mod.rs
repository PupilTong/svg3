//! The GPU layer: wgpu pipelines and the [`Renderer`] that drives them.
//!
//! The module is split along construction-vs-encode lines so the
//! [`Renderer`] file itself stays focused on the per-frame encode path:
//!
//! - [`clear`] / [`device`]: tiny utilities (`clear_target`, `clear_depth`,
//!   `acquire_gpu`, [`DEPTH_FORMAT`]) used by the renderer and the headless
//!   path.
//! - [`pipeline`]: stateless device construction — every render pipeline and
//!   bind-group layout, called once from [`Renderer::with_device`].
//! - [`uniforms`]: the WGSL-aligned uniform structs and the builders that
//!   pack a resolved filter primitive into them.
//! - [`image_decode`]: PNG data-URL decoding for `<feImage>` sources.
//! - [`readback`]: the [`Image`] type the headless renderer produces, plus
//!   the GPU → CPU readback routine.
//! - [`renderer`]: the [`Renderer`] struct + the [`GpuScene`] handle and
//!   every encode method.

mod clear;
mod device;
mod image_decode;
mod pipeline;
mod readback;
mod renderer;
mod texture_store;
mod uniforms;

pub use clear::{clear_target, DEPTH_FORMAT};
pub use readback::Image;
pub use renderer::{GpuScene, RenderError, Renderer};
