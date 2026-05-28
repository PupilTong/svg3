//! The [`Vertex`] / [`Mesh`] geometry data model.
//!
//! Shape tessellation — the [`crate::render::shapes`] modules, dispatched by
//! [`build_scene`](crate::build_scene) — produces a [`Mesh`] of these
//! vertices, which the [`Renderer`](crate::Renderer) uploads to the GPU.
//! Every [`Vertex`] also carries SDF data so the fragment shader can compute
//! analytic, anti-aliased coverage for the curved primitives.

/// Vertex buffer layout: object-space position, linear RGBA colour, the
/// shape-local SDF coordinate, the SDF parameters, the shape-kind tag, a
/// paint-server index, and the surface-local UV that texture paint servers
/// sample at.
pub(crate) const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x4,
    2 => Float32x2,
    3 => Float32x4,
    4 => Uint32,
    5 => Uint32,
    6 => Float32x2,
];

/// A single GPU vertex.
///
/// Beyond `position` and `color`, every vertex carries SDF data so the
/// fragment shader can compute analytic, anti-aliased coverage: `local` is
/// the shape-local coordinate (interpolated across a bounding quad), `params`
/// the SDF parameters (constant per quad), and `kind` the shape-kind tag.
/// Solid triangle geometry tags `kind` as `KIND_SOLID` and leaves
/// `local`/`params` zeroed — see the `KIND_*` constants in the `shapes`
/// modules. `paint_id == 0` means "use `color` directly"; non-zero ids index
/// the mesh's paint-server list. `uv` is the surface-local `(u, v)` that
/// `PAINT_SVG_TEXTURE` paint servers sample at; 2D shapes carry `[0, 0]`
/// because their paint kinds never branch into the texture sampler.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Linear RGBA colour in `[0, 1]`.
    pub color: [f32; 4],
    /// Shape-local coordinate for the SDF, interpolated across the quad.
    pub local: [f32; 2],
    /// SDF parameters, constant across a shape's quad; meaning depends on
    /// `kind`.
    pub params: [f32; 4],
    /// Shape-kind tag — one of the `shapes` modules' `KIND_*` constants.
    pub kind: u32,
    /// Paint-server index; `0` is the inline solid [`Vertex::color`].
    pub paint_id: u32,
    /// Surface-local `(u, v)` for `PAINT_SVG_TEXTURE` paint servers.
    /// Unused (and `[0, 0]`) on 2D shapes.
    pub uv: [f32; 2],
}

/// Maximum number of `<stop>` children evaluated for one gradient paint.
pub(crate) const MAX_GRADIENT_STOPS: usize = 8;

/// Maximum number of rectangular child paints evaluated for one pattern tile.
pub(crate) const MAX_PATTERN_ITEMS: usize = 8;

/// Paint server kinds carried in [`PaintServer::meta[0]`]. Must stay in sync
/// with the `PAINT_*` constants in `shader.wgsl`.
pub(crate) const PAINT_LINEAR_GRADIENT: u32 = 1;
pub(crate) const PAINT_RADIAL_GRADIENT: u32 = 2;
pub(crate) const PAINT_PATTERN: u32 = 3;
/// `<svg>` placed in `<defs>` used as a paint server: the renderer rasterizes
/// the subtree into an offscreen texture layer and the fragment stage samples
/// it at the vertex `uv`. `meta = [PAINT_SVG_TEXTURE, layer_index,
/// mapping_kind, mapping_aux]` — `mapping_kind` selects the per-primitive
/// remap (e.g. cube-cross), and `mapping_aux` carries optional parameters.
pub(crate) const PAINT_SVG_TEXTURE: u32 = 4;

/// `mapping_kind` value: identity — sample `uv` as-is. Used by `<cube
/// cube-map="same">`, `<ellipsoid>`, `<cylinder>`, and `<surface>`.
pub(crate) const TEXTURE_MAP_IDENTITY: u32 = 0;
/// `mapping_kind` value: 4×3 horizontal-cross atlas. Each cube face's
/// face-local `(s, t)` is remapped into one of the six atlas slots — top
/// row `+Y`, middle row `-X / +Z / +X / -Z`, bottom row `-Y`.
pub(crate) const TEXTURE_MAP_CUBE_CROSS: u32 = 1;

/// GPU representation of a referenced SVG paint server.
///
/// `meta` is `[kind, stop_count, pattern_item_count, 0]`. For gradients,
/// `geometry` is `[x1, y1, x2, y2]` for linear gradients and
/// `[cx, cy, r, 0]` for radial gradients, all in SVG user units. For
/// patterns it is `[x, y, width, height]` for the tile in user units, and
/// `pattern_rects` / `pattern_colors` describe up to
/// [`MAX_PATTERN_ITEMS`] solid rectangular child paints inside that tile.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PaintServer {
    pub meta: [u32; 4],
    pub geometry: [f32; 4],
    pub colors: [[f32; 4]; MAX_GRADIENT_STOPS],
    pub offsets: [[f32; 4]; 2],
    pub pattern_rects: [[f32; 4]; MAX_PATTERN_ITEMS],
    pub pattern_colors: [[f32; 4]; MAX_PATTERN_ITEMS],
}

impl PaintServer {
    pub(crate) fn zeroed() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

/// A combined triangle mesh: a vertex list plus a triangle index list.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    /// Vertices, in SVG user space (origin top-left, y-down, `z = 0`).
    pub vertices: Vec<Vertex>,
    /// Triangle indices into [`Mesh::vertices`].
    pub indices: Vec<u32>,
    /// Referenced paint servers used by non-zero [`Vertex::paint_id`] values.
    pub(crate) paint_servers: Vec<PaintServer>,
}

impl Mesh {
    /// Build a mesh from vertices and indices with no referenced paint
    /// servers. Vertices with `paint_id == 0` use their inline colour.
    pub(crate) fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self {
            vertices,
            indices,
            paint_servers: Vec::new(),
        }
    }

    /// Append `other`'s geometry, offsetting its indices to stay valid.
    pub(crate) fn append(&mut self, other: Mesh) {
        let base = self.vertices.len() as u32;
        let paint_base = self.paint_servers.len() as u32;
        self.vertices
            .extend(other.vertices.into_iter().map(|mut vertex| {
                if vertex.paint_id != 0 {
                    vertex.paint_id += paint_base;
                }
                vertex
            }));
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
        self.paint_servers.extend(other.paint_servers);
    }

    /// Whether the mesh contains no triangles.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Add one paint server and return the non-zero id vertices should carry
    /// to reference it.
    pub(crate) fn push_paint_server(&mut self, paint: PaintServer) -> u32 {
        self.paint_servers.push(paint);
        self.paint_servers.len() as u32
    }

    /// Paint servers in the order addressed by vertex `paint_id` values.
    pub(crate) fn paint_servers(&self) -> &[PaintServer] {
        &self.paint_servers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_layout_is_tightly_packed() {
        // position(3) + color(4) + local(2) + params(4) + kind(1) +
        // paint_id(1) + uv(2): seventeen
        // 4-byte fields, no padding — the `vertex_attr_array!` offsets in
        // `VERTEX_ATTRIBUTES` assume exactly this tight `#[repr(C)]` layout.
        assert_eq!(
            std::mem::size_of::<Vertex>(),
            17 * std::mem::size_of::<f32>()
        );
    }

    #[test]
    fn paint_server_layout_is_wgsl_aligned() {
        // meta + geometry + 8 stop colours + 2 offset vec4s +
        // 8 pattern rects + 8 pattern colours = 28 vec4-sized words.
        assert_eq!(std::mem::size_of::<PaintServer>(), 28 * 16);
    }
}
