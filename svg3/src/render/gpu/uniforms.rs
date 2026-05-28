//! GPU uniform buffer layouts + the helpers that build them from resolved
//! filter primitives.
//!
//! Three uniform structs feed the three shader programs:
//! - [`TransformUniform`] holds the view-projection matrix consumed by
//!   `shaders/shader.wgsl`.
//! - [`FilterUniform`] is a single shared layout filled by every filter
//!   pass; `shaders/filter.wgsl` interprets only the fields its entry
//!   point uses.
//! - [`ImageUniform`] is consumed by `shaders/image.wgsl` to draw a
//!   referenced `<feImage>` source into a filter primitive's output.
//!
//! Each `*_uniform` builder zeroes the unused fields, so it's always safe to
//! pass one [`FilterUniform`] into any filter shader entry point.

use glam::{Mat3, Mat4};

use crate::render::filters::{
    Blend, ColorMatrix, ComponentTransfer, Composite, ConvolveMatrix, DisplacementMap, DropShadow,
    FilterInput, Flood, ImageRect, LightSource, Lighting, MaskMode, Morphology, Offset, Turbulence,
};
use crate::render::scene::ViewportClip;

use std::collections::BTreeMap;

use super::renderer::FilterTexture;

/// Largest one-sided kernel radius used by the GPU Gaussian blur pass.
pub(super) const MAX_BLUR_RADIUS: u32 = 64;

/// Uniform data consumed by `shaders/shader.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct TransformUniform {
    /// Column-major view-projection matrix, matching WGSL matrix layout.
    pub(super) view_projection: [[f32; 4]; 4],
}

impl TransformUniform {
    pub(super) fn new(view_projection: Mat4) -> Self {
        Self {
            view_projection: view_projection.to_cols_array_2d(),
        }
    }
}

/// Uniform data consumed by `shaders/filter.wgsl`.
///
/// One shared layout fills every filter pass; only the fields a particular
/// fragment entry point reads are meaningful, the rest are zeroed. Field
/// names map 1:1 to the WGSL `FilterUniform` struct in `filter.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct FilterUniform {
    pub(super) texel_size: [f32; 2],
    pub(super) direction: [f32; 2],
    pub(super) color: [f32; 4],
    pub(super) extra: [f32; 4],
    pub(super) light: [f32; 4],
    pub(super) light_dir: [f32; 4],
    pub(super) lighting: [f32; 4],
    pub(super) matrix_r0: [f32; 4],
    pub(super) matrix_r1: [f32; 4],
    pub(super) matrix_r2: [f32; 4],
    pub(super) matrix_r3: [f32; 4],
    pub(super) matrix_col4: [f32; 4],
    pub(super) transfer_r0: [f32; 4],
    pub(super) transfer_r1: [f32; 4],
    pub(super) transfer_g0: [f32; 4],
    pub(super) transfer_g1: [f32; 4],
    pub(super) transfer_b0: [f32; 4],
    pub(super) transfer_b1: [f32; 4],
    pub(super) transfer_a0: [f32; 4],
    pub(super) transfer_a1: [f32; 4],
    pub(super) transfer_kinds: [u32; 4],
    pub(super) transfer_counts: [u32; 4],
    pub(super) sigma: f32,
    pub(super) radius: u32,
    pub(super) mode: u32,
    pub(super) flags: u32,
}

/// Uniform data consumed by `shaders/image.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct ImageUniform {
    /// Column-major view-projection matrix, matching WGSL matrix layout.
    pub(super) view_projection: [[f32; 4]; 4],
    /// Image draw rectangle: x, y, width, height in SVG user space.
    pub(super) rect: [f32; 4],
}

impl FilterUniform {
    pub(super) fn empty(width: u32, height: u32) -> Self {
        let mut uniform = Self::zeroed();
        uniform.texel_size = [1.0 / width as f32, 1.0 / height as f32];
        uniform
    }

    pub(super) fn zeroed() -> Self {
        bytemuck::Zeroable::zeroed()
    }

    pub(super) fn blur(width: u32, height: u32, direction: [f32; 2], sigma: f32) -> Self {
        let mut uniform = Self::empty(width, height);
        uniform.direction = direction;
        uniform.sigma = sigma;
        uniform.radius = (sigma * 3.0).ceil().clamp(0.0, MAX_BLUR_RADIUS as f32) as u32;
        uniform
    }

    pub(super) fn composite(width: u32, height: u32) -> Self {
        // The orthographic identity projection maps `z = 0` to NDC `0.5`
        // everywhere, so a per-pixel computation with this VP collapses
        // to the constant `0.5` the headless renderer used to ship.
        Self::composite_with_projection(width, height, Mat4::IDENTITY)
    }

    /// Composite uniform that lets the shader compute the NDC depth of
    /// the world `z = 0` plane *per fragment* under the current
    /// projection. Earlier code passed a single scalar (`plane_depth`),
    /// taken at the document centre — exact for orthographic, but under
    /// any non-axis-aligned projection (a pitched or yawed orbit camera)
    /// each fragment's z = 0 plane depth varies across the screen, so a
    /// single value pegs the halo at the centre's depth and rejects
    /// fragments that lie on the same plane but at a different NDC depth.
    ///
    /// The fix packs:
    ///   - `matrix_r0..r2.xyz`: rows of the inverse homography of the
    ///     forward projection restricted to the z = 0 plane (recovers
    ///     user-space `(X, Y)` from an NDC `(nx, ny)`).
    ///   - `matrix_r3.xyz`: row 2 of the view-projection at columns
    ///     `(0, 1, 3)` — i.e. the coefficients that produce `clip.z`
    ///     from a user-space point on the z = 0 plane.
    ///   - `matrix_col4.xyz`: row 3 at the same columns — the
    ///     `clip.w` coefficients.
    ///
    /// The shader runs the homography to recover `(X, Y)`, then
    /// forward-projects `(X, Y, 0)` to read out `clip.z / clip.w`.
    pub(super) fn composite_with_projection(
        width: u32,
        height: u32,
        view_projection: Mat4,
    ) -> Self {
        let mut uniform = Self::empty(width, height);
        let plane_inv = z0_plane_inverse(view_projection);
        let p = plane_inv.to_cols_array();
        uniform.matrix_r0 = [p[0], p[3], p[6], 0.0];
        uniform.matrix_r1 = [p[1], p[4], p[7], 0.0];
        uniform.matrix_r2 = [p[2], p[5], p[8], 0.0];
        let c0 = view_projection.col(0);
        let c1 = view_projection.col(1);
        let c3 = view_projection.col(3);
        // `clip.z` = c0.z * X + c1.z * Y + 0 * Z + c3.z * 1, taken at
        // `Z = 0`. Same for `clip.w` using the `.w` components.
        uniform.matrix_r3 = [c0.z, c1.z, c3.z, 0.0];
        uniform.matrix_col4 = [c0.w, c1.w, c3.w, 0.0];
        uniform
    }

    /// An identity colour matrix in the matrix block — used to copy the
    /// source verbatim through the colour-matrix pipeline when a chain step
    /// reduces to a passthrough (e.g. a zero-deviation Gaussian blur).
    pub(super) fn passthrough(width: u32, height: u32) -> Self {
        let mut uniform = Self::empty(width, height);
        uniform.matrix_r0 = [1.0, 0.0, 0.0, 0.0];
        uniform.matrix_r1 = [0.0, 1.0, 0.0, 0.0];
        uniform.matrix_r2 = [0.0, 0.0, 1.0, 0.0];
        uniform.matrix_r3 = [0.0, 0.0, 0.0, 1.0];
        uniform
    }

    /// SourceAlpha extractor: RGB rows zero, A row copies input alpha. Used
    /// to derive the filter's `SourceAlpha` pseudo-input from `SourceGraphic`
    /// via the colour-matrix pipeline.
    pub(super) fn source_alpha(width: u32, height: u32) -> Self {
        let mut uniform = Self::empty(width, height);
        // R = G = B = 0; A = 0*R + 0*G + 0*B + 1*A + 0 = src.a.
        uniform.matrix_r3 = [0.0, 0.0, 0.0, 1.0];
        uniform
    }
}

impl ImageUniform {
    pub(super) fn new(view_projection: Mat4, rect: ImageRect) -> Self {
        Self {
            view_projection: view_projection.to_cols_array_2d(),
            rect: [rect.x, rect.y, rect.width, rect.height],
        }
    }
}

/// Pick the texture view that satisfies a primitive's `in` / `in2` reference.
///
/// Unresolvable references — a `Named` reference with no matching earlier
/// `result`, or `Default` for the very first primitive — fall back to
/// `SourceGraphic`, matching SVG's "if the value is unresolved, use
/// SourceGraphic" behaviour.
pub(super) fn resolve_input<'a>(
    input: &FilterInput,
    prev_index: Option<usize>,
    source: &'a FilterTexture,
    source_alpha: &'a FilterTexture,
    outputs: &'a [FilterTexture],
    named: &BTreeMap<&str, usize>,
) -> &'a wgpu::TextureView {
    match input {
        FilterInput::Default => match prev_index {
            Some(i) => &outputs[i].view,
            None => &source.view,
        },
        FilterInput::SourceGraphic => &source.view,
        FilterInput::SourceAlpha => &source_alpha.view,
        // svg3 doesn't yet capture the destination surface for
        // `BackgroundImage` (needs `enable-background="new"` semantics).
        // Resolving to `SourceGraphic` keeps a chain using the pseudo-input
        // running rather than no-op'ing, and is consistent with the spec's
        // fallback ("undefined" -> implementation-defined).
        FilterInput::BackgroundImage => &source.view,
        FilterInput::BackgroundAlpha => &source_alpha.view,
        // `FillPaint` / `StrokePaint` are filter-local paint pseudo-inputs.
        // svg3 resolves paint servers for normal shape drawing, but does not
        // yet materialize these pseudo-inputs as standalone flood textures.
        // Falling back to `SourceGraphic` keeps the primitive running.
        FilterInput::FillPaint | FilterInput::StrokePaint => &source.view,
        FilterInput::Named(name) => match named.get(name.as_str()) {
            Some(i) => &outputs[*i].view,
            None => &source.view,
        },
    }
}

/// Resolve a primitive input reference to the UV-space subregion where the
/// upstream paint actually lives. SVG pseudo-inputs (`SourceGraphic`,
/// `SourceAlpha`, the background-image / paint approximations) span the
/// full filter region; a named or default reference uses the upstream
/// primitive's resolved output subregion. Unresolvable references fall back
/// to the full filter region to keep the downstream primitive sampling
/// against well-defined UVs.
pub(super) fn resolve_input_uv(
    input: &FilterInput,
    prev_index: Option<usize>,
    named: &BTreeMap<&str, usize>,
    output_uv: &[[f32; 4]],
) -> [f32; 4] {
    let pick = |i: usize| output_uv.get(i).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
    match input {
        FilterInput::Default => prev_index.map_or([0.0, 0.0, 1.0, 1.0], pick),
        FilterInput::Named(name) => named
            .get(name.as_str())
            .copied()
            .map_or([0.0, 0.0, 1.0, 1.0], pick),
        FilterInput::SourceGraphic
        | FilterInput::SourceAlpha
        | FilterInput::BackgroundImage
        | FilterInput::BackgroundAlpha
        | FilterInput::FillPaint
        | FilterInput::StrokePaint => [0.0, 0.0, 1.0, 1.0],
    }
}

pub(super) fn color_matrix_uniform(extent: wgpu::Extent3d, cm: &ColorMatrix) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    let m = &cm.matrix;
    uniform.matrix_r0 = [m[0][0], m[0][1], m[0][2], m[0][3]];
    uniform.matrix_r1 = [m[1][0], m[1][1], m[1][2], m[1][3]];
    uniform.matrix_r2 = [m[2][0], m[2][1], m[2][2], m[2][3]];
    uniform.matrix_r3 = [m[3][0], m[3][1], m[3][2], m[3][3]];
    uniform.matrix_col4 = [m[0][4], m[1][4], m[2][4], m[3][4]];
    uniform
}

pub(super) fn turbulence_uniform(extent: wgpu::Extent3d, t: Turbulence) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.extra = [t.base_frequency[0], t.base_frequency[1], 0.0, 0.0];
    uniform.radius = t.num_octaves;
    uniform.sigma = t.seed;
    uniform.flags = if t.fractal_noise { 1 } else { 0 };
    uniform
}

pub(super) fn lighting_uniform(
    extent: wgpu::Extent3d,
    viewport: crate::render::Viewport,
    view_projection: Mat4,
    l: Lighting,
    specular: bool,
) -> FilterUniform {
    // `lighting.w` encodes the light source kind: 0 = distant, 1 = point,
    // 2 = spot. Must stay in sync with `LIGHT_TYPE_*` constants in
    // `filter.wgsl`.
    //
    // SVG 1.1 §15.21.2 says light positions are in user space (default
    // `primitiveUnits="userSpaceOnUse"`). The filter shader works in the
    // source texture's UV space, and the source texture's pixel layout is
    // whatever `view_projection` projects user-space geometry to. For the
    // orthographic default and a texture sized to the document, 1 texel
    // equals 1 user-space unit — but for a window-sized texture or a
    // perspective projection, that equality breaks. Earlier code
    // multiplied positions by `extent / viewport`, which is the correct
    // factor only for orthographic projections; under perspective with a
    // moved camera the cone test went wildly off and dropped the lit
    // region.
    //
    // The robust fix: store the inverse projection of the world `z = 0`
    // plane as a 3×3 homography in `matrix_r0..2`, and have the shader
    // recover the user-space `(X, Y)` of each texel before evaluating
    // light vectors. Light positions then stay in their authored
    // user-space units, and the math is correct for any projection
    // (including perspective and off-axis cameras).
    //
    // `matrix_r0.w` / `matrix_r1.w` carry the per-axis texel-to-user-space
    // scale used by the surface-normal Sobel filter (it samples
    // neighbouring texels, not neighbouring user-space units, so its
    // gradient needs that conversion). `matrix_r2.w` is unused.
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.color = l.lighting_color;
    let is_specular = if specular { 1.0 } else { 0.0 };
    let plane_inverse = z0_plane_inverse(view_projection);
    let p = plane_inverse.to_cols_array();
    // glam stores `Mat3` column-major, so `to_cols_array` returns
    // `[c0.x, c0.y, c0.z, c1.x, c1.y, c1.z, c2.x, c2.y, c2.z]`. The
    // shader wants rows: matrix_rN = (p[0*3+N], p[1*3+N], p[2*3+N]).
    let scale_x = if viewport.width > 0.0 {
        extent.width as f32 / viewport.width
    } else {
        1.0
    };
    let scale_y = if viewport.height > 0.0 {
        extent.height as f32 / viewport.height
    } else {
        1.0
    };
    uniform.matrix_r0 = [p[0], p[3], p[6], scale_x];
    uniform.matrix_r1 = [p[1], p[4], p[7], scale_y];
    uniform.matrix_r2 = [p[2], p[5], p[8], 0.0];
    match l.light {
        LightSource::Distant(dir) => {
            // Distant-light is a direction, not a position — no length
            // scaling needed. The vector is already unit length.
            uniform.light = [dir[0], dir[1], dir[2], l.specular_exponent];
            uniform.lighting = [l.surface_scale, l.constant, is_specular, 0.0];
        }
        LightSource::Point(pos) => {
            uniform.light = [pos[0], pos[1], pos[2], l.specular_exponent];
            uniform.lighting = [l.surface_scale, l.constant, is_specular, 1.0];
        }
        LightSource::Spot {
            position,
            direction,
            cone_exponent,
            cos_limit,
        } => {
            uniform.light = [position[0], position[1], position[2], l.specular_exponent];
            // `light_dir.xyz` carries the cone axis (light -> pointsAt);
            // `light_dir.w` carries `cos(limitingConeAngle)` or `-1.0` if
            // the user did not constrain the cone. The cone axis is a unit
            // direction and stays unscaled.
            uniform.light_dir = [direction[0], direction[1], direction[2], cos_limit];
            uniform.lighting = [l.surface_scale, l.constant, is_specular, 2.0];
            // `extra.x` is reused as the cone-falloff exponent — distinct
            // from the surface Phong exponent stored in `light.w`.
            uniform.extra[0] = cone_exponent;
        }
    }
    uniform
}

/// Inverse of the projection restricted to the world `z = 0` plane.
///
/// `view_projection` maps `(X, Y, 0, 1)` to clip-space `(cx, cy, cz, cw)`,
/// with NDC `(cx, cy) / cw`. Dropping the irrelevant `cz` row and the `Z`
/// column gives the 3×3 forward matrix that the lighting shader needs to
/// invert: it maps `[X, Y, 1]ᵀ` (homogeneous user-space) to
/// `[cx, cy, cw]ᵀ`. The inverse returns `(X, Y)` from a normalized device
/// coordinate.
///
/// Falls back to the identity if the inverse is singular — under any
/// reasonable camera this never happens, but a degenerate view-projection
/// matrix (e.g. an all-zero one in a test) shouldn't NaN out the lighting
/// pass.
fn z0_plane_inverse(view_projection: Mat4) -> Mat3 {
    let c0 = view_projection.col(0);
    let c1 = view_projection.col(1);
    let c3 = view_projection.col(3);
    let m = Mat3::from_cols_array(&[
        c0.x, c0.y, c0.w, // first column: clip.x/y/w contribution of X
        c1.x, c1.y, c1.w, // second column: clip.x/y/w contribution of Y
        c3.x, c3.y, c3.w, // third column: clip.x/y/w contribution of the 1
    ]);
    let determinant = m.determinant();
    if determinant.abs() < 1e-9 {
        Mat3::IDENTITY
    } else {
        m.inverse()
    }
}

pub(super) fn morphology_uniform(
    extent: wgpu::Extent3d,
    m: Morphology,
    direction: [f32; 2],
    radius_pixels: f32,
) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.direction = direction;
    uniform.radius = radius_pixels.round().clamp(0.0, MAX_BLUR_RADIUS as f32) as u32;
    uniform.flags = if m.dilate { 1 } else { 0 };
    uniform
}

pub(super) fn flood_uniform(extent: wgpu::Extent3d, f: Flood) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.color = f.color;
    uniform
}

pub(super) fn drop_shadow_alpha_uniform(
    extent: wgpu::Extent3d,
    shadow: DropShadow,
) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.color = shadow.color;
    // The shader subtracts `direction` in UV space, so convert the pixel offset.
    uniform.direction = [
        shadow.offset[0] * uniform.texel_size[0],
        shadow.offset[1] * uniform.texel_size[1],
    ];
    uniform
}

pub(super) fn displacement_uniform(extent: wgpu::Extent3d, d: DisplacementMap) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.extra = [d.scale, 0.0, 0.0, 0.0];
    uniform.transfer_kinds = [d.x_channel, d.y_channel, 0, 0];
    uniform
}

pub(super) fn convolve_uniform(extent: wgpu::Extent3d, c: ConvolveMatrix) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.matrix_r0 = [c.kernel[0][0], c.kernel[0][1], c.kernel[0][2], 0.0];
    uniform.matrix_r1 = [c.kernel[1][0], c.kernel[1][1], c.kernel[1][2], 0.0];
    uniform.matrix_r2 = [c.kernel[2][0], c.kernel[2][1], c.kernel[2][2], 0.0];
    uniform.matrix_col4 = [c.divisor, c.bias, 0.0, 0.0];
    uniform.flags = if c.preserve_alpha { 1 } else { 0 };
    uniform.mode = c.edge_mode;
    uniform
}

pub(super) fn component_transfer_uniform(
    extent: wgpu::Extent3d,
    transfer: ComponentTransfer,
) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    let (r0, r1) = split_transfer_table(&transfer.r.table);
    let (g0, g1) = split_transfer_table(&transfer.g.table);
    let (b0, b1) = split_transfer_table(&transfer.b.table);
    let (a0, a1) = split_transfer_table(&transfer.a.table);
    uniform.transfer_r0 = r0;
    uniform.transfer_r1 = r1;
    uniform.transfer_g0 = g0;
    uniform.transfer_g1 = g1;
    uniform.transfer_b0 = b0;
    uniform.transfer_b1 = b1;
    uniform.transfer_a0 = a0;
    uniform.transfer_a1 = a1;
    uniform.transfer_kinds = [
        transfer.r.kind,
        transfer.g.kind,
        transfer.b.kind,
        transfer.a.kind,
    ];
    uniform.transfer_counts = [
        transfer.r.count,
        transfer.g.count,
        transfer.b.count,
        transfer.a.count,
    ];
    uniform
}

/// Split an 8-entry transfer table into two `vec4` halves matching the WGSL
/// `transfer_<c>0` / `transfer_<c>1` layout.
fn split_transfer_table(table: &[f32; 8]) -> ([f32; 4], [f32; 4]) {
    (
        [table[0], table[1], table[2], table[3]],
        [table[4], table[5], table[6], table[7]],
    )
}

pub(super) fn offset_uniform(extent: wgpu::Extent3d, offset: Offset) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    // The shader subtracts `direction` from the sampling UV, so a positive
    // `dx` translates the input to the right — same sign convention as the
    // drop-shadow alpha pass.
    uniform.direction = [
        offset.dx * uniform.texel_size[0],
        offset.dy * uniform.texel_size[1],
    ];
    uniform
}

pub(super) fn blend_uniform(extent: wgpu::Extent3d, b: Blend) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.mode = b.mode;
    uniform
}

pub(super) fn fe_composite_uniform(extent: wgpu::Extent3d, c: Composite) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.mode = c.op;
    uniform.extra = c.k;
    uniform
}

pub(super) fn tile_uniform(extent: wgpu::Extent3d, source_uv: [f32; 4]) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    // `source_uv = [x, y, w, h]` is the input primitive's subregion in UV
    // space (full texture `[0, 0, 1, 1]` when the upstream primitive has no
    // authored subregion). The fragment shader wraps UVs inside this rect.
    uniform.extra = source_uv;
    uniform
}

pub(super) fn alpha_mask_uniform(extent: wgpu::Extent3d, mode: MaskMode) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.mode = match mode {
        MaskMode::Alpha => 0,
        MaskMode::Luminance => 1,
    };
    uniform
}

pub(super) fn subregion_clip_uniform(extent: wgpu::Extent3d, uv: [f32; 4]) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.extra = uv;
    uniform
}

pub(super) fn viewport_clip_uniform(extent: wgpu::Extent3d, clip: ViewportClip) -> FilterUniform {
    let mut uniform = FilterUniform::empty(extent.width, extent.height);
    uniform.extra = [
        clip.root_viewport.width,
        clip.root_viewport.height,
        clip.rect[0],
        clip.rect[1],
    ];
    uniform.lighting = [clip.rect[2], clip.rect[3], 0.0, 0.0];
    uniform.matrix_r0 = [
        clip.inverse_rows[0][0],
        clip.inverse_rows[0][1],
        clip.inverse_rows[0][2],
        0.0,
    ];
    uniform.matrix_r1 = [
        clip.inverse_rows[1][0],
        clip.inverse_rows[1][1],
        clip.inverse_rows[1][2],
        0.0,
    ];
    uniform
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_uniform_matches_wgsl_matrix_size() {
        assert_eq!(
            std::mem::size_of::<TransformUniform>(),
            16 * std::mem::size_of::<f32>()
        );
    }

    #[test]
    fn filter_uniform_matches_wgsl_layout() {
        // The shared filter uniform packs (16-byte-aligned vec4 blocks):
        //   texel_size(2) + direction(2)                                = 4 floats
        //   + color + extra + light + light_dir + lighting              = 5 * 4 = 20 floats
        //   + matrix rows r0..r3 + matrix_col4                          = 5 * 4 = 20 floats
        //   + 8 transfer halves (transfer_<rgba>{0,1})                  = 8 * 4 = 32 floats
        //   + transfer_kinds(u32×4) + transfer_counts(u32×4)            = 2 * 4 = 8 (4-byte words)
        //   + sigma, radius, mode, flags                                = 4 (4-byte words)
        // Total = 22 vec4s = 352 bytes; a multiple of 16, satisfying
        // WGSL std140-style alignment.
        assert_eq!(std::mem::size_of::<FilterUniform>(), 352);
    }

    #[test]
    fn image_uniform_matches_wgsl_layout() {
        // 16 floats for the matrix + 4 floats for the rect.
        assert_eq!(
            std::mem::size_of::<ImageUniform>(),
            (16 + 4) * std::mem::size_of::<f32>()
        );
    }

    fn extent(w: u32, h: u32) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        }
    }

    fn lighting_with_point(pos: [f32; 3]) -> Lighting {
        Lighting {
            surface_scale: 1.0,
            constant: 1.0,
            specular_exponent: 1.0,
            lighting_color: [1.0; 4],
            lighting_color_uses_current: false,
            light: LightSource::Point(pos),
        }
    }

    /// Orthographic view-projection covering the document `(0, 0)..(W, H)`,
    /// matching the headless renderer's default. Used by the lighting
    /// tests so the inverse homography is exact / numerically stable.
    fn ortho_view_projection(viewport: crate::render::Viewport) -> Mat4 {
        Mat4::orthographic_lh(0.0, viewport.width, viewport.height, 0.0, -1000.0, 1000.0)
    }

    #[test]
    fn lighting_uniform_passes_light_positions_through_unchanged() {
        // Light positions now stay in user-space coordinates; the shader
        // recovers each texel's user-space `(X, Y)` via the inverse
        // homography in `matrix_r0..2`. Any earlier `extent / viewport`
        // pre-scaling has moved into the shader, so the uniform must
        // round-trip the authored coordinates byte-for-byte.
        let viewport = crate::render::Viewport {
            width: 220.0,
            height: 220.0,
        };
        let uniform = lighting_uniform(
            extent(200, 200),
            viewport,
            ortho_view_projection(viewport),
            lighting_with_point([50.0, 75.0, 200.0]),
            true,
        );
        assert_eq!(uniform.light[0], 50.0);
        assert_eq!(uniform.light[1], 75.0);
        assert_eq!(uniform.light[2], 200.0);
    }

    #[test]
    fn lighting_uniform_distant_light_keeps_direction_unscaled() {
        // Distant light is a unit direction, not a position — no length
        // scaling needed. The encoded `light.xyz` must equal the input
        // direction byte-for-byte regardless of viewport.
        let l = Lighting {
            surface_scale: 1.0,
            constant: 1.0,
            specular_exponent: 1.0,
            lighting_color: [1.0; 4],
            lighting_color_uses_current: false,
            light: LightSource::Distant([0.6, 0.0, 0.8]),
        };
        let viewport = crate::render::Viewport {
            width: 220.0,
            height: 220.0,
        };
        let uniform = lighting_uniform(
            extent(200, 200),
            viewport,
            ortho_view_projection(viewport),
            l,
            false,
        );
        assert_eq!(uniform.light[0], 0.6);
        assert_eq!(uniform.light[1], 0.0);
        assert_eq!(uniform.light[2], 0.8);
    }

    #[test]
    fn lighting_uniform_encodes_texel_to_user_scales_in_matrix_w() {
        // The Sobel-derived normal needs to convert from per-texel
        // gradient to per-user-space-unit gradient. The renderer packs
        // those scales into `matrix_r0.w` and `matrix_r1.w` so the
        // shader can multiply through.
        let viewport = crate::render::Viewport {
            width: 440.0,
            height: 140.0,
        };
        let uniform = lighting_uniform(
            extent(800, 600),
            viewport,
            ortho_view_projection(viewport),
            lighting_with_point([100.0, 50.0, 30.0]),
            false,
        );
        let expected_x = 800.0 / 440.0;
        let expected_y = 600.0 / 140.0;
        assert!((uniform.matrix_r0[3] - expected_x).abs() < 1e-4);
        assert!((uniform.matrix_r1[3] - expected_y).abs() < 1e-4);
        assert_eq!(uniform.matrix_r2[3], 0.0);
    }

    #[test]
    fn lighting_uniform_z0_inverse_round_trips_under_orthographic() {
        // For the ortho default, the inverse homography of the z=0 plane
        // is exact: applying it to a corner's NDC must produce the
        // corresponding user-space corner. Sanity-checks the matrix
        // packing and the `[col0, col1, col3]` slicing.
        let viewport = crate::render::Viewport {
            width: 100.0,
            height: 200.0,
        };
        let vp = ortho_view_projection(viewport);
        let uniform = lighting_uniform(
            extent(100, 200),
            viewport,
            vp,
            lighting_with_point([0.0, 0.0, 0.0]),
            false,
        );
        // Reconstruct the 3×3 from the rows the renderer packed (drop the
        // `w` slot which is the per-axis scale, not part of the matrix).
        let inverse = Mat3::from_cols_array(&[
            uniform.matrix_r0[0],
            uniform.matrix_r1[0],
            uniform.matrix_r2[0],
            uniform.matrix_r0[1],
            uniform.matrix_r1[1],
            uniform.matrix_r2[1],
            uniform.matrix_r0[2],
            uniform.matrix_r1[2],
            uniform.matrix_r2[2],
        ]);
        // Bottom-right corner: user (100, 200, 0) → NDC (1, -1).
        // Inverse should send NDC (1, -1, 1) back to user (100, 200, 1)
        // (up to the homogeneous third component).
        let v = inverse * glam::Vec3::new(1.0, -1.0, 1.0);
        let xy = v.truncate() / v.z;
        assert!(
            (xy.x - 100.0).abs() < 1e-3 && (xy.y - 200.0).abs() < 1e-3,
            "inverse should round-trip the bottom-right corner: got {xy}",
        );
    }

    #[test]
    fn lighting_uniform_zero_viewport_falls_back_to_identity() {
        // A degenerate (zero-width) viewport must not divide by zero in
        // the texel-to-user-space scale calculation; the fallback is 1.0.
        let viewport = crate::render::Viewport {
            width: 0.0,
            height: 0.0,
        };
        let uniform = lighting_uniform(
            extent(100, 100),
            viewport,
            Mat4::IDENTITY,
            lighting_with_point([10.0, 20.0, 30.0]),
            false,
        );
        assert_eq!(uniform.light[0], 10.0);
        assert_eq!(uniform.light[1], 20.0);
        assert_eq!(uniform.light[2], 30.0);
        assert_eq!(uniform.matrix_r0[3], 1.0);
        assert_eq!(uniform.matrix_r1[3], 1.0);
    }

    #[test]
    fn lighting_uniform_encodes_specular_flag_and_light_kind_tag() {
        // `lighting.z` is the specular toggle (0 = diffuse, 1 = specular)
        // and `lighting.w` is the light-source kind tag the WGSL shader
        // dispatches on (0 = distant, 1 = point, 2 = spot). Pin them so a
        // future refactor doesn't quietly desync the Rust→WGSL ABI.
        let viewport = crate::render::Viewport {
            width: 64.0,
            height: 64.0,
        };
        let vp = ortho_view_projection(viewport);
        let point = lighting_uniform(
            extent(64, 64),
            viewport,
            vp,
            lighting_with_point([0.0, 0.0, 1.0]),
            true,
        );
        assert_eq!(point.lighting[2], 1.0); // specular = true
        assert_eq!(point.lighting[3], 1.0); // LIGHT_TYPE_POINT

        let spot = lighting_uniform(
            extent(64, 64),
            viewport,
            vp,
            Lighting {
                surface_scale: 1.0,
                constant: 1.0,
                specular_exponent: 1.0,
                lighting_color: [1.0; 4],
                lighting_color_uses_current: false,
                light: LightSource::Spot {
                    position: [10.0, 20.0, 30.0],
                    direction: [0.0, 0.0, 1.0],
                    cone_exponent: 2.0,
                    cos_limit: 0.5,
                },
            },
            false,
        );
        // Specular flag = 0 (diffuse), light kind tag = 2 (LIGHT_TYPE_SPOT).
        assert_eq!(spot.lighting[2], 0.0);
        assert_eq!(spot.lighting[3], 2.0);
        // Spot cos_limit and cone_exponent must reach the shader unchanged.
        assert_eq!(spot.light_dir[3], 0.5);
        assert_eq!(spot.extra[0], 2.0);
    }
}
