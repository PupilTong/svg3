//! Render-target clearing helpers + the offscreen depth-texture wrapper.
//!
//! [`Renderer::encode_document`](super::renderer::Renderer::encode_document)
//! always *loads* (rather than clears) its target so it can composite filter
//! results on top, so the caller — headless or windowed — has to clear the
//! target up front. [`clear_target`] does that with a single render pass.

/// Depth buffer format used by the shape pipeline and by
/// [`Renderer::encode_document`](super::renderer::Renderer::encode_document)
/// for the target / filter source depth textures. 32-bit float gives ample
/// precision over the wide orthographic depth range svg3 uses (±100k
/// units) so the depth test correctly resolves 2D-3D occlusion at practical
/// authoring sizes per [SPEC.md](../../../../SPEC.md) §7.3.
///
/// Exposed so windowed callers driving
/// [`Renderer::draw`](super::renderer::Renderer::draw) directly can allocate
/// a matching depth texture for the render pass they construct.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Execute a single render pass that clears `target` to `color`. Used by both
/// the headless and the windowed paths to prime their target before
/// [`Renderer::encode_document`](super::renderer::Renderer::encode_document),
/// which always loads (rather than clears) the existing target so it can
/// composite filter results on top.
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

/// Clear a depth attachment to `1.0` (the far plane). Used by
/// [`Renderer::encode_document`](super::renderer::Renderer::encode_document)
/// once per encode for the target depth buffer, and per filter chain for the
/// offscreen filter source depth buffer.
pub(super) fn clear_depth(
    encoder: &mut wgpu::CommandEncoder,
    depth: &wgpu::TextureView,
    label: &str,
) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
}

/// An offscreen depth texture allocated per encode (target or filter source).
pub(super) struct DepthTexture {
    pub(super) _texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
}
