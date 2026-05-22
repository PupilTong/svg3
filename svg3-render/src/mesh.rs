//! The [`Vertex`] / [`Mesh`] geometry data model.
//!
//! Shape tessellation — the [`crate::shapes`] modules, dispatched by
//! [`build_scene`](crate::build_scene) — produces a [`Mesh`] of these
//! vertices, which the [`Renderer`](crate::Renderer) uploads to the GPU.
//! Every [`Vertex`] also carries SDF data so the fragment shader can compute
//! analytic, anti-aliased coverage for the curved primitives.

/// Vertex buffer layout: object-space position, linear RGBA colour, the
/// shape-local SDF coordinate, the SDF parameters, and the shape-kind tag.
pub(crate) const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x4,
    2 => Float32x2,
    3 => Float32x4,
    4 => Uint32,
];

/// A single GPU vertex.
///
/// Beyond `position` and `color`, every vertex carries SDF data so the
/// fragment shader can compute analytic, anti-aliased coverage: `local` is
/// the shape-local coordinate (interpolated across a bounding quad), `params`
/// the SDF parameters (constant per quad), and `kind` the shape-kind tag.
/// Solid triangle geometry tags `kind` as `KIND_SOLID` and leaves
/// `local`/`params` zeroed — see the `KIND_*` constants in the `shapes`
/// modules.
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
}

/// A combined triangle mesh: a vertex list plus a triangle index list.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    /// Vertices, in SVG user space (origin top-left, y-down, `z = 0`).
    pub vertices: Vec<Vertex>,
    /// Triangle indices into [`Mesh::vertices`].
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Append `other`'s geometry, offsetting its indices to stay valid.
    pub(crate) fn append(&mut self, other: Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
    }

    /// Whether the mesh contains no triangles.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_layout_is_tightly_packed() {
        // position(3) + color(4) + local(2) + params(4) + kind(1): fourteen
        // 4-byte fields, no padding — the `vertex_attr_array!` offsets in
        // `VERTEX_ATTRIBUTES` assume exactly this tight `#[repr(C)]` layout.
        assert_eq!(
            std::mem::size_of::<Vertex>(),
            14 * std::mem::size_of::<f32>()
        );
    }
}
