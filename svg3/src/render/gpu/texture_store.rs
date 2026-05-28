//! Per-encode SVG-texture paint-server cache.
//!
//! When a 3D primitive's `fill` resolves to a `<defs><svg id="…">` reference,
//! the renderer rasterizes that subtree once into a layer of a 2D-array
//! texture and the shader samples the layer at the vertex `uv`. The store
//! owns the texture array, sampler, and a [`NodeId`] → layer-index map so
//! repeated references in the same encode reuse the same rasterized layer.
//!
//! Lifecycle is per-`encode_document` call: the store is constructed up
//! front (after collecting referenced node ids from the render plan),
//! rasterizes one layer per unique referenced subtree, and is bound into
//! the shape pipeline's group 2 for the duration of that encode.

use std::collections::HashMap;

use glam::Mat4;

use crate::dom::{Document, NodeId};
use crate::render::scene::texture_subtree_viewport;
use crate::render::Viewport;

use super::clear::{clear_depth, clear_target, DepthTexture, DEPTH_FORMAT};

/// Minimum side length, in texels, of one rasterized texture layer. A
/// document with very small `<defs><svg>` content rects is still rasterized
/// at this resolution so anti-aliased edges and small features survive the
/// `LinearFilter` sample on the 3D surface.
const MIN_TEXTURE_SIDE: u32 = 256;

/// Maximum side length, capped so an absurdly large defs-`<svg>` doesn't
/// allocate an unreasonable texture.
const MAX_TEXTURE_SIDE: u32 = 2048;

/// Texture-array layer count used when no SVG textures are referenced. The
/// dummy layer is `1×1` transparent so the shape pipeline's bind group can
/// stay constant regardless of document content.
const DUMMY_LAYER_COUNT: u32 = 1;

/// Side length of the dummy texture array used when no SVG textures are
/// referenced.
const DUMMY_TEXTURE_SIDE: u32 = 1;

/// Owns the texture-array binding for one `encode_document` call. Build via
/// [`TextureStore::empty`] when no textures are referenced (binds a 1×1
/// dummy), or via [`TextureStore::new`] with a rasterizer callback per
/// referenced node id.
pub(super) struct TextureStore {
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    layers: HashMap<NodeId, u32>,
    #[allow(dead_code)]
    side: u32,
}

impl TextureStore {
    /// Build an empty store with a single 1×1 transparent layer. Used when
    /// the document has no `<svg>` texture paint servers; the bind group
    /// still references this view so the pipeline layout doesn't change
    /// based on document content.
    ///
    /// `format` MUST match the shape pipeline's colour-attachment format:
    /// the rasterize pass re-uses that pipeline to draw 2D paths into a
    /// texture-array layer, and wgpu rejects a pass whose colour formats
    /// differ from the pipeline's. On the headless test renderer that's
    /// `Rgba8UnormSrgb`; on the macOS swapchain it's `Bgra8UnormSrgb`.
    pub(super) fn empty(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("svg3 texture array (empty)"),
            size: wgpu::Extent3d {
                width: DUMMY_TEXTURE_SIDE,
                height: DUMMY_TEXTURE_SIDE,
                depth_or_array_layers: DUMMY_LAYER_COUNT,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("svg3 texture array view (empty)"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Self {
            view,
            sampler: linear_sampler(device, "svg3 texture sampler (empty)"),
            layers: HashMap::new(),
            side: DUMMY_TEXTURE_SIDE,
        }
    }

    /// Rasterize each `<defs><svg>` referenced in `node_ids` (deduplicated
    /// internally) into a layer of a 2D-array texture. `rasterize_layer` is
    /// invoked once per unique node id with the chosen target view and
    /// viewport so the renderer can dispatch the existing 2D path against
    /// it. Returns `None` and falls back to [`TextureStore::empty`] when
    /// every node either disappears (degenerate viewport) or rasterizes
    /// empty — there's no work for the texture array to do.
    pub(super) fn new<F>(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        document: &Document,
        node_ids: &[NodeId],
        format: wgpu::TextureFormat,
        mut rasterize_layer: F,
    ) -> Self
    where
        F: FnMut(&mut wgpu::CommandEncoder, &wgpu::TextureView, NodeId, Viewport, Mat4, u32),
    {
        let mut unique: Vec<(NodeId, Viewport)> = Vec::new();
        let mut layers: HashMap<NodeId, u32> = HashMap::new();
        for &node_id in node_ids {
            if layers.contains_key(&node_id) {
                continue;
            }
            let Some(viewport) = texture_subtree_viewport(document, node_id) else {
                continue;
            };
            let layer = unique.len() as u32;
            layers.insert(node_id, layer);
            unique.push((node_id, viewport));
        }

        if unique.is_empty() {
            return Self::empty(device, format);
        }

        // Pick a side length per resolution. For simplicity in MVP, every
        // layer of the array is the same size — the largest needed. WGPU
        // texture-array layers must share dimensions anyway.
        let side = unique
            .iter()
            .map(|(_, viewport)| {
                let max_dim = viewport.width.max(viewport.height).ceil() as u32;
                next_power_of_two(max_dim).clamp(MIN_TEXTURE_SIDE, MAX_TEXTURE_SIDE)
            })
            .max()
            .unwrap_or(MIN_TEXTURE_SIDE);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("svg3 texture array"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: unique.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        for (layer, (node_id, viewport)) in unique.iter().enumerate() {
            let layer_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("svg3 texture array layer"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            // Clear the layer up front so subtree-empty rasters leave a
            // transparent texture rather than reading uninitialised memory.
            clear_target(
                encoder,
                &layer_view,
                wgpu::Color::TRANSPARENT,
                "svg3 texture layer clear",
            );
            let view_projection = ortho_view_projection(*viewport);
            rasterize_layer(
                encoder,
                &layer_view,
                *node_id,
                *viewport,
                view_projection,
                side,
            );
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("svg3 texture array view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        Self {
            view,
            sampler: linear_sampler(device, "svg3 texture sampler"),
            layers,
            side,
        }
    }

    /// Bindable texture-array view.
    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Bindable sampler (linear, clamp-to-edge).
    pub(super) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Resolved layer index for a referenced node id, or `0` (the dummy /
    /// first layer) when the node never rasterized to a real layer. The
    /// dummy is fully transparent, so missing references render the same
    /// as a transparent paint.
    pub(super) fn layer_for(&self, node_id: NodeId) -> u32 {
        self.layers.get(&node_id).copied().unwrap_or(0)
    }

    /// Number of distinct rasterized layers (excludes the dummy used by
    /// [`TextureStore::empty`]).
    #[allow(dead_code)]
    pub(super) fn layer_count(&self) -> usize {
        self.layers.len()
    }
}

/// Allocate a depth texture for one texture rasterize pass. The 2D pipeline
/// is depth-tested for painter-order Z bias resolution, so each layer
/// rasterize needs its own depth attachment.
pub(super) fn texture_layer_depth(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    side: u32,
) -> DepthTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("svg3 texture layer depth"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    clear_depth(encoder, &view, "svg3 texture layer depth clear");
    DepthTexture {
        _texture: texture,
        view,
    }
}

fn linear_sampler(device: &wgpu::Device, label: &str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}

/// Orthographic view-projection for one texture rasterize pass: maps
/// `(0..viewport.width × 0..viewport.height)` user space to NDC, with the
/// y-down convention SVG uses. Z range is wide so painter-order Z-bias on
/// 2D shapes doesn't clip.
fn ortho_view_projection(viewport: Viewport) -> Mat4 {
    Mat4::orthographic_rh(
        0.0,
        viewport.width.max(1.0),
        viewport.height.max(1.0),
        0.0,
        -100_000.0,
        100_000.0,
    )
}

/// Round `value` up to the next power of two, with a floor of 1. Returns
/// `value` itself when it's already a power of two. Useful for picking a
/// power-of-two texture side from a user-space content rect.
fn next_power_of_two(value: u32) -> u32 {
    if value <= 1 {
        return 1;
    }
    value.next_power_of_two()
}

/// Collect the unique `<defs><svg>` node ids referenced as
/// `PAINT_SVG_TEXTURE` paint servers by every mesh in `plan`. Preserves
/// first-seen document order so the resulting layer indices are stable
/// across re-encodes for the same document.
pub(super) fn collect_texture_node_ids(plan: &[crate::render::scene::RenderOp]) -> Vec<NodeId> {
    use crate::render::mesh::PAINT_SVG_TEXTURE;
    use std::collections::HashSet;
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut ids: Vec<NodeId> = Vec::new();
    let mut visit_mesh = |mesh: &crate::render::Mesh| {
        for server in mesh.paint_servers() {
            if server.meta[0] == PAINT_SVG_TEXTURE {
                // Stored placeholder: the defs node's arena index.
                let node_id = NodeId::from_index(server.meta[1] as usize);
                if seen.insert(node_id) {
                    ids.push(node_id);
                }
            }
        }
    };
    for op in plan {
        match op {
            crate::render::scene::RenderOp::Mesh(mesh) => visit_mesh(mesh),
            crate::render::scene::RenderOp::Clip { mesh, .. } => visit_mesh(mesh),
            crate::render::scene::RenderOp::Filter {
                mesh, clip, mask, ..
            } => {
                visit_mesh(mesh);
                if let Some(clip_mesh) = clip {
                    visit_mesh(clip_mesh);
                }
                if let Some(m) = mask {
                    visit_mesh(&m.mesh);
                }
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::parse;
    use crate::render::scene::build_render_plan;

    /// One `<defs><svg>` referenced by both a `<cube>` and an `<ellipsoid>`
    /// should appear exactly once in the collected node-id list — the
    /// renderer rasterizes it just once and the texture store hands out a
    /// single layer.
    #[test]
    fn collect_dedupes_one_svg_referenced_by_multiple_3d_shapes() {
        let document = parse(
            r##"<svg extension="pupiltong" width="100" height="100"><defs><svg id="t" viewBox="0 0 100 100"><rect width="100" height="100" fill="red"/></svg></defs><cube cx="32" cy="50" cz="0" size="20" fill="url(#t)"/><ellipsoid cx="72" cy="50" cz="0" r="10" fill="url(#t)"/></svg>"##,
        )
        .unwrap();
        let plan = build_render_plan(
            &document,
            crate::render::Viewport {
                width: 100.0,
                height: 100.0,
            },
        );
        let ids = collect_texture_node_ids(&plan);
        assert_eq!(
            ids.len(),
            1,
            "the same <defs><svg> should be collected exactly once, got {ids:?}"
        );
    }

    /// Distinct `<defs><svg>` definitions referenced by separate 3D shapes
    /// produce one collected node id each, in first-seen document order.
    #[test]
    fn collect_returns_distinct_ids_in_first_seen_order() {
        let document = parse(
            r##"<svg extension="pupiltong" width="100" height="100"><defs><svg id="a" viewBox="0 0 100 100"><rect width="100" height="100" fill="red"/></svg><svg id="b" viewBox="0 0 100 100"><rect width="100" height="100" fill="blue"/></svg></defs><cube cx="32" cy="50" cz="0" size="20" fill="url(#a)"/><ellipsoid cx="72" cy="50" cz="0" r="10" fill="url(#b)"/></svg>"##,
        )
        .unwrap();
        let plan = build_render_plan(
            &document,
            crate::render::Viewport {
                width: 100.0,
                height: 100.0,
            },
        );
        let ids = collect_texture_node_ids(&plan);
        assert_eq!(ids.len(), 2);
    }

    /// `<defs><svg>` defined but unreferenced by any 3D primitive is a
    /// no-op: no node id is collected because no mesh's paint server
    /// points at it.
    #[test]
    fn collect_returns_empty_for_unreferenced_defs_svg() {
        let document = parse(
            r##"<svg extension="pupiltong" width="100" height="100"><defs><svg id="t" viewBox="0 0 100 100"><rect width="100" height="100" fill="red"/></svg></defs><cube cx="50" cy="50" cz="0" size="20" fill="blue"/></svg>"##,
        )
        .unwrap();
        let plan = build_render_plan(
            &document,
            crate::render::Viewport {
                width: 100.0,
                height: 100.0,
            },
        );
        assert!(collect_texture_node_ids(&plan).is_empty());
    }
}
