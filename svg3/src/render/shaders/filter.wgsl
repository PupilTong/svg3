// Full-screen GPU post-processing for SVG filters.
//
// All filter passes share the same fullscreen-quad vertex stage and the same
// bind-group layout:
//   @binding(0) — input texture 1 (the current chain output)
//   @binding(1) — sampler
//   @binding(2) — input texture 2 (the immutable SourceGraphic; used by
//                 multi-input primitives such as feDisplacementMap and the
//                 drop-shadow composite step)
//   @binding(3) — FilterUniform
//
// Each filter primitive picks the fragment entry point it needs; the renderer
// fills in only the FilterUniform fields that entry point reads, leaving the
// rest zeroed.
//
// All offscreen filter targets use premultiplied RGBA in linear space. The
// blur kernel and convolution accumulate premultiplied colour; the final
// composite pass blends back onto the destination with premultiplied alpha.

struct FilterUniform {
    texel_size: vec2<f32>,
    direction: vec2<f32>,
    color: vec4<f32>,
    extra: vec4<f32>,
    light: vec4<f32>,
    light_dir: vec4<f32>,
    lighting: vec4<f32>,
    matrix_r0: vec4<f32>,
    matrix_r1: vec4<f32>,
    matrix_r2: vec4<f32>,
    matrix_r3: vec4<f32>,
    matrix_col4: vec4<f32>,
    transfer_r0: vec4<f32>,
    transfer_r1: vec4<f32>,
    transfer_g0: vec4<f32>,
    transfer_g1: vec4<f32>,
    transfer_b0: vec4<f32>,
    transfer_b1: vec4<f32>,
    transfer_a0: vec4<f32>,
    transfer_a1: vec4<f32>,
    transfer_kinds: vec4<u32>,
    transfer_counts: vec4<u32>,
    sigma: f32,
    radius: u32,
    mode: u32,
    flags: u32,
}

@group(0) @binding(0)
var source_texture: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;
@group(0) @binding(2)
var source_texture2: texture_2d<f32>;
@group(0) @binding(3)
var<uniform> filter_params: FilterUniform;

// Bindings 4-5 are used ONLY by `fs_composite`. The other filter
// pipelines bind a 4-entry layout; the composite pipeline binds a
// 6-entry layout that adds the source depth texture and a non-filtering
// depth sampler. Sampling them from a fragment shader on a 4-entry
// layout would be a validation error.
@group(0) @binding(4)
var source_depth_texture: texture_depth_2d;
@group(0) @binding(5)
var depth_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];

    var out: VertexOutput;
    out.clip_position = vec4<f32>(position, 0.0, 1.0);
    out.uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return out;
}

// ---- Helpers ---------------------------------------------------------------

fn sample_in1(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(source_texture, source_sampler, uv);
}

fn sample_in2(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(source_texture2, source_sampler, uv);
}

fn unpremultiply(color: vec4<f32>) -> vec4<f32> {
    if (color.a <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return vec4<f32>(color.rgb / color.a, color.a);
}

fn premultiply(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(color.rgb * color.a, color.a);
}

fn luminance(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2125, 0.7154, 0.0721));
}

fn channel_value(color: vec4<f32>, channel: u32) -> f32 {
    if (channel == 0u) { return color.r; }
    if (channel == 1u) { return color.g; }
    if (channel == 2u) { return color.b; }
    return color.a;
}

// ---- Gaussian blur (separable) --------------------------------------------

@fragment
fn fs_blur(in: VertexOutput) -> @location(0) vec4<f32> {
    if (filter_params.radius == 0u || filter_params.sigma <= 0.0) {
        return sample_in1(in.uv);
    }

    let two_sigma_sq = 2.0 * filter_params.sigma * filter_params.sigma;
    var weighted_sum = vec4<f32>(0.0);
    var weight_sum = 0.0;
    let radius = i32(filter_params.radius);

    for (var i = -radius; i <= radius; i = i + 1) {
        let offset = f32(i);
        let weight = exp(-(offset * offset) / two_sigma_sq);
        let uv = in.uv + filter_params.direction * filter_params.texel_size * offset;
        let sample = sample_in1(uv);
        weighted_sum += sample * weight;
        weight_sum += weight;
    }

    return weighted_sum / weight_sum;
}

// ---- Final composite ------------------------------------------------------

struct CompositeOutput {
    @location(0) color: vec4<f32>,
    // Forwards the source pass's per-pixel NDC depth into the target so
    // subsequent 3D draws can spatially occlude or be occluded by the
    // filtered geometry (SPEC §7.3). Halo pixels — fragments inside the
    // filter region but outside the source geometry — get the NDC depth
    // of the world `z = 0` plane *at that screen position*, computed
    // per-fragment via the inverse projection encoded in `matrix_r0..r3`
    // and `matrix_col4`. A constant fallback (taken at the document
    // centre) was correct for the orthographic default but wrong for
    // perspective with any pitch or yaw, where the z = 0 plane crosses
    // a range of NDC depths across the screen — every fragment past the
    // centre's depth then failed the `LessEqual` test, slicing later
    // filters into half-moon wedges.
    @builtin(frag_depth) depth: f32,
}

/// NDC depth of the `world.z = 0` plane at the given UV under the
/// current projection. The uniform packs:
///   - `matrix_r0..r2.xyz`: rows of the inverse homography mapping
///     NDC `(nx, ny)` back to user-space `(X, Y)` on the z = 0 plane.
///   - `matrix_r3.xyz`: row 2 of the view-projection, columns
///     `(0, 1, 3)` — the coefficients producing `clip.z` from
///     `(X, Y, 0, 1)`.
///   - `matrix_col4.xyz`: row 3 at the same columns — the `clip.w`
///     coefficients.
/// Falls back to `1.0` (far plane) when either the inverse homography
/// or the forward `clip.w` is singular at this fragment, so a
/// degenerate uniform doesn't write garbage depth.
fn plane_ndc_depth(uv: vec2<f32>) -> f32 {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let v = vec3<f32>(ndc.x, ndc.y, 1.0);
    let xyw = vec3<f32>(
        dot(filter_params.matrix_r0.xyz, v),
        dot(filter_params.matrix_r1.xyz, v),
        dot(filter_params.matrix_r2.xyz, v),
    );
    if (abs(xyw.z) < 1e-6) {
        return 1.0;
    }
    let xy = xyw.xy / xyw.z;
    let v_xy1 = vec3<f32>(xy.x, xy.y, 1.0);
    let clip_z = dot(filter_params.matrix_r3.xyz, v_xy1);
    let clip_w = dot(filter_params.matrix_col4.xyz, v_xy1);
    if (abs(clip_w) < 1e-6) {
        return 1.0;
    }
    return clamp(clip_z / clip_w, 0.0, 1.0);
}

@fragment
fn fs_composite(in: VertexOutput) -> CompositeOutput {
    let color = sample_in1(in.uv);
    let src_depth = textureSample(source_depth_texture, depth_sampler, in.uv);
    // Discard fragments that are simultaneously outside the source
    // silhouette AND make zero colour contribution. Without this the
    // composite quad — which covers the whole filter region (≈ the
    // viewport) — writes the `z = 0` plane's NDC depth at every empty
    // pixel, and under a perspective + orbit camera that plane tilts
    // and slices through any 3D geometry whose back face sits behind
    // it. We still write `plane_ndc_depth` at non-source pixels that
    // *do* contribute colour (the filter's blur / drop-shadow / lit
    // flat-region halo), because subsequent 2D filters chain through
    // those pixels via the `LessEqual` tie at the plane's depth.
    // Inside the source silhouette we always pass through (even at
    // `alpha = 0`, e.g. the dark side of an `feSpecularLighting`
    // sphere) so the source geometry's depth still gets written.
    if (color.a <= 0.0 && (src_depth >= 1.0 || filter_params.flags == 1u)) {
        discard;
    }
    var depth: f32;
    if (src_depth < 1.0) {
        depth = src_depth;
    } else {
        depth = plane_ndc_depth(in.uv);
    }
    var out: CompositeOutput;
    out.color = color;
    out.depth = depth;
    return out;
}

// ---- feColorMatrix --------------------------------------------------------

@fragment
fn fs_color_matrix(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = sample_in1(in.uv);
    let straight = unpremultiply(src);
    let c = vec4<f32>(straight.r, straight.g, straight.b, straight.a);
    let bias = filter_params.matrix_col4;
    let result = vec4<f32>(
        dot(filter_params.matrix_r0, c) + bias.x,
        dot(filter_params.matrix_r1, c) + bias.y,
        dot(filter_params.matrix_r2, c) + bias.z,
        dot(filter_params.matrix_r3, c) + bias.w,
    );
    let clamped = clamp(result, vec4<f32>(0.0), vec4<f32>(1.0));
    return premultiply(clamped);
}

// ---- feTurbulence ---------------------------------------------------------
//
// Value-noise approximation: we sample a hash on the integer lattice and
// bilinearly interpolate inside each cell, then sum octaves. This is not the
// SVG 1.1 reference Perlin noise but produces visually similar fractal output
// and is fully on-GPU. The `fractalNoise` type maps the result to [0, 1]; the
// classic `turbulence` type maps it to [-1, 1] then `abs()`.

fn hash21(p: vec2<f32>, seed: f32) -> f32 {
    let q = vec2<f32>(p.x * 127.1 + seed, p.y * 311.7 - seed);
    let s = sin(dot(q, vec2<f32>(12.9898, 78.233)));
    return fract(s * 43758.5453);
}

fn value_noise(p: vec2<f32>, seed: f32) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i + vec2<f32>(0.0, 0.0), seed);
    let b = hash21(i + vec2<f32>(1.0, 0.0), seed);
    let c = hash21(i + vec2<f32>(0.0, 1.0), seed);
    let d = hash21(i + vec2<f32>(1.0, 1.0), seed);
    let x = mix(a, b, u.x);
    let y = mix(c, d, u.x);
    return mix(x, y, u.y);
}

@fragment
fn fs_turbulence(in: VertexOutput) -> @location(0) vec4<f32> {
    let octaves = max(filter_params.radius, 1u);
    let seed = filter_params.sigma;
    let base = filter_params.extra.xy;
    let canvas = vec2<f32>(1.0 / max(filter_params.texel_size.x, 1e-6),
                           1.0 / max(filter_params.texel_size.y, 1e-6));
    var amp = 1.0;
    var sum = 0.0;
    var norm = 0.0;
    var freq = base;
    let fractal = filter_params.flags == 1u;
    for (var i = 0u; i < octaves; i = i + 1u) {
        let p = in.uv * canvas * freq;
        var n = value_noise(p, seed + f32(i) * 13.37);
        if (!fractal) {
            n = abs(n * 2.0 - 1.0);
        }
        sum += amp * n;
        norm += amp;
        amp = amp * 0.5;
        freq = freq * 2.0;
    }
    let value = sum / max(norm, 1e-6);
    return vec4<f32>(value, value, value, 1.0);
}

// ---- feSpecularLighting / feDiffuseLighting -------------------------------
//
// All lighting math is evaluated in *user-space* `(X, Y, Z)` — the units
// SVG 1.1 §15.21.2 assigns to `<fePointLight>` / `<feSpotLight>` positions.
// Recovering user-space `(X, Y)` from a texel's UV needs the inverse of
// the current projection restricted to the world `z = 0` plane, which the
// renderer packs into `matrix_r0..2` as a 3×3 homography. Earlier code
// approximated the conversion with a single `extent / viewport` scale,
// which only matches user-space under orthographic projection — under a
// moved perspective camera the scale is wrong and varies across the
// texture, so the spot cone shifted off the lit region and clipped the
// sphere with a hard black edge.

/// User-space `(X, Y)` of the texel at `uv`, recovered via the inverse
/// homography of the projection's `z = 0` plane.
fn surface_user_xy(uv: vec2<f32>) -> vec2<f32> {
    // UV → NDC. The composite path flips Y so SVG `y = 0` (top) maps to
    // UV `y = 0`; here we undo that for the homography input.
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let v = vec3<f32>(ndc.x, ndc.y, 1.0);
    // `matrix_r0..2.xyz` are the rows of the 3×3 inverse homography.
    let xyw = vec3<f32>(
        dot(filter_params.matrix_r0.xyz, v),
        dot(filter_params.matrix_r1.xyz, v),
        dot(filter_params.matrix_r2.xyz, v),
    );
    if (abs(xyw.z) < 1e-6) {
        return vec2<f32>(0.0, 0.0);
    }
    return xyw.xy / xyw.z;
}

fn surface_normal(uv: vec2<f32>) -> vec3<f32> {
    let dx = filter_params.texel_size.x;
    let dy = filter_params.texel_size.y;
    let surface_scale = filter_params.lighting.x;
    // Sobel filter on the alpha channel; the height field is `alpha * surface_scale`.
    let tl = sample_in1(uv + vec2<f32>(-dx, -dy)).a;
    let tc = sample_in1(uv + vec2<f32>(0.0, -dy)).a;
    let tr = sample_in1(uv + vec2<f32>(dx, -dy)).a;
    let ml = sample_in1(uv + vec2<f32>(-dx, 0.0)).a;
    let mr = sample_in1(uv + vec2<f32>(dx, 0.0)).a;
    let bl = sample_in1(uv + vec2<f32>(-dx, dy)).a;
    let bc = sample_in1(uv + vec2<f32>(0.0, dy)).a;
    let br = sample_in1(uv + vec2<f32>(dx, dy)).a;
    let sx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
    let sy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
    // Sobel returns `d(alpha)/d(texel)`. Convert to user-space gradient
    // via the per-axis texel-to-user scale stored in `matrix_rN.w` so the
    // normal direction is correct for non-square `tex/viewport` ratios
    // (e.g. a 440×140 doc rendered into a 1500×1200 window texture).
    let scale_x = filter_params.matrix_r0.w;
    let scale_y = filter_params.matrix_r1.w;
    let nx = -sx * 0.25 * surface_scale * scale_x;
    let ny = -sy * 0.25 * surface_scale * scale_y;
    return normalize(vec3<f32>(nx, ny, 1.0));
}

// `lighting.w` light-source tag: keep in sync with `LIGHT_TYPE_*` constants
// in `renderer.rs`'s `lighting_uniform` builder.
const LIGHT_TYPE_DISTANT: f32 = 0.0;
const LIGHT_TYPE_POINT: f32 = 1.0;
const LIGHT_TYPE_SPOT: f32 = 2.0;

fn light_vector(uv: vec2<f32>, surface_z: f32) -> vec3<f32> {
    let kind = filter_params.lighting.w;
    if (kind >= LIGHT_TYPE_POINT - 0.5) {
        // Point and spot lights carry user-space `(x, y, z)`; recover the
        // surface point in matching user-space coordinates and take the
        // diff so the resulting unit vector is independent of texture
        // size and projection.
        let surface_xy = surface_user_xy(uv);
        let surface_point = vec3<f32>(surface_xy.x, surface_xy.y, surface_z);
        let diff = filter_params.light.xyz - surface_point;
        return normalize(diff);
    }
    return normalize(filter_params.light.xyz);
}

/// Cone-falloff factor for `<feSpotLight>`. Returns 1.0 for non-spot lights
/// so the regular diffuse/specular paths are unaffected. Per SVG 1.1 §15.21.1,
/// the cone factor is `max(-dot(L, axis), 0)^specularExponent`, zero outside
/// `limitingConeAngle`.
///
/// Evaluated in user-space so the cone angle matches the author's intent
/// regardless of projection or texture size.
fn spot_cone_factor(uv: vec2<f32>, surface_z: f32) -> f32 {
    if (filter_params.lighting.w < LIGHT_TYPE_SPOT - 0.5) {
        return 1.0;
    }
    let axis = normalize(filter_params.light_dir.xyz);
    let surface_xy = surface_user_xy(uv);
    let surface_point = vec3<f32>(surface_xy.x, surface_xy.y, surface_z);
    let light_point = filter_params.light.xyz;
    let diff = surface_point - light_point;
    if (dot(diff, diff) <= 1e-6) {
        // Surface coincides with the light origin — fully lit.
        return 1.0;
    }
    let to_surface = normalize(diff);
    let cos_angle = dot(to_surface, axis);
    if (cos_angle <= 0.0) {
        return 0.0;
    }
    let cos_limit = filter_params.light_dir.w;
    // `cos_limit < 0` encodes "no limiting cone".
    if (cos_limit >= 0.0 && cos_angle < cos_limit) {
        return 0.0;
    }
    let exponent = filter_params.extra.x;
    return pow(cos_angle, exponent);
}

@fragment
fn fs_lighting(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = surface_normal(in.uv);
    let surface_z = sample_in1(in.uv).a * filter_params.lighting.x;
    let light_dir = light_vector(in.uv, surface_z);
    let cone = spot_cone_factor(in.uv, surface_z);
    let lighting_color = filter_params.color;
    let constant = filter_params.lighting.y;
    let specular = filter_params.lighting.z > 0.5;

    if (specular) {
        // Halfway vector between the view direction (looking straight down at
        // the surface, `(0, 0, 1)`) and the light. SVG 1.1 §15.22.
        let view = vec3<f32>(0.0, 0.0, 1.0);
        let half_vec = normalize(light_dir + view);
        let n_dot_h = max(dot(normal, half_vec), 0.0);
        let exponent = filter_params.light.w;
        let intensity = constant * pow(n_dot_h, exponent) * cone;
        let rgb = lighting_color.rgb * intensity;
        // Specular alpha = max(R, G, B), per SVG 1.1 §15.22.
        let alpha = clamp(max(rgb.r, max(rgb.g, rgb.b)), 0.0, 1.0);
        return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * alpha, alpha);
    } else {
        let n_dot_l = max(dot(normal, light_dir), 0.0);
        let intensity = constant * n_dot_l * cone;
        let rgb = clamp(lighting_color.rgb * intensity, vec3<f32>(0.0), vec3<f32>(1.0));
        // Diffuse output alpha is opaque per SVG 1.1 §15.21.
        return vec4<f32>(rgb, 1.0);
    }
}

// ---- feMorphology ---------------------------------------------------------

@fragment
fn fs_morphology(in: VertexOutput) -> @location(0) vec4<f32> {
    let radius = filter_params.radius;
    if (radius == 0u) {
        return sample_in1(in.uv);
    }
    let dilate = filter_params.flags == 1u;
    let axis = filter_params.direction;
    var acc: vec4<f32>;
    if (dilate) {
        acc = vec4<f32>(0.0);
    } else {
        acc = vec4<f32>(1.0);
    }
    let r = i32(radius);
    for (var i = -r; i <= r; i = i + 1) {
        let uv = in.uv + axis * filter_params.texel_size * f32(i);
        let s = sample_in1(uv);
        if (dilate) {
            acc = max(acc, s);
        } else {
            acc = min(acc, s);
        }
    }
    return acc;
}

// ---- feFlood --------------------------------------------------------------

@fragment
fn fs_flood(in: VertexOutput) -> @location(0) vec4<f32> {
    // The flood region is the entire filter primitive area, which in this
    // renderer is the full offscreen target. `in.uv` is unused here.
    _ = in.uv;
    return premultiply(filter_params.color);
}

// ---- feDropShadow ---------------------------------------------------------
//
// Step 1 (`fs_drop_shadow_alpha`): sample the source at uv - offset and emit
// `color * source.a` premultiplied, isolating the alpha-coloured silhouette.
// Steps 2/3: the standard separable Gaussian blur (`fs_blur`).
// Step 4 (`fs_drop_shadow_composite`): "source over shadow" between
// `source_texture2` (SourceGraphic) and `source_texture` (the blurred shadow).

@fragment
fn fs_drop_shadow_alpha(in: VertexOutput) -> @location(0) vec4<f32> {
    let offset_uv = filter_params.direction;
    let s = sample_in1(in.uv - offset_uv);
    let alpha = s.a * filter_params.color.a;
    return vec4<f32>(filter_params.color.rgb * alpha, alpha);
}

@fragment
fn fs_drop_shadow_composite(in: VertexOutput) -> @location(0) vec4<f32> {
    // `source_texture` carries the blurred shadow; `source_texture2` carries
    // the original SourceGraphic. SourceGraphic-over-shadow, both premultiplied.
    let shadow = sample_in1(in.uv);
    let source = sample_in2(in.uv);
    return source + shadow * (1.0 - source.a);
}

// ---- feDisplacementMap ----------------------------------------------------

@fragment
fn fs_displacement(in: VertexOutput) -> @location(0) vec4<f32> {
    // The map lives on `source_texture2` (SourceGraphic by default in this
    // renderer); the displaced graphic lives on `source_texture` (in1).
    let map = sample_in2(in.uv);
    let x_channel = filter_params.transfer_kinds.x;
    let y_channel = filter_params.transfer_kinds.y;
    let scale = filter_params.extra.x;
    let dx = (channel_value(map, x_channel) - 0.5) * scale;
    let dy = (channel_value(map, y_channel) - 0.5) * scale;
    let displaced = in.uv + vec2<f32>(dx, dy) * filter_params.texel_size;
    return sample_in1(displaced);
}

// ---- feConvolveMatrix -----------------------------------------------------

// SVG 1.1 §15.10 `edgeMode` — keep in sync with `EDGE_MODE_*` in `filters.rs`.
const EDGE_MODE_DUPLICATE: u32 = 0u;
const EDGE_MODE_WRAP: u32 = 1u;
const EDGE_MODE_NONE: u32 = 2u;

/// Sample the source texture with the requested `edgeMode` handling. The
/// renderer's sampler is `ClampToEdge`, which already implements
/// "duplicate"; the other two modes need an explicit out-of-bounds check.
fn sample_with_edge_mode(uv: vec2<f32>, mode: u32) -> vec4<f32> {
    if (mode == EDGE_MODE_NONE) {
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            return vec4<f32>(0.0);
        }
        return sample_in1(uv);
    }
    if (mode == EDGE_MODE_WRAP) {
        let wrapped = fract(uv - floor(uv));
        return sample_in1(wrapped);
    }
    // EDGE_MODE_DUPLICATE: the ClampToEdge sampler does the right thing.
    return sample_in1(uv);
}

@fragment
fn fs_convolve(in: VertexOutput) -> @location(0) vec4<f32> {
    let row0 = filter_params.matrix_r0;
    let row1 = filter_params.matrix_r1;
    let row2 = filter_params.matrix_r2;
    let divisor = filter_params.matrix_col4.x;
    let bias = filter_params.matrix_col4.y;
    let preserve_alpha = filter_params.flags == 1u;
    let edge_mode = filter_params.mode;
    let tx = filter_params.texel_size.x;
    let ty = filter_params.texel_size.y;

    // 3×3 kernel sampled with the source target's bilinear sampler.
    var acc = vec4<f32>(0.0);
    let offsets = array<vec2<f32>, 9>(
        vec2<f32>(-tx, -ty), vec2<f32>(0.0, -ty), vec2<f32>(tx, -ty),
        vec2<f32>(-tx, 0.0), vec2<f32>(0.0, 0.0), vec2<f32>(tx, 0.0),
        vec2<f32>(-tx, ty), vec2<f32>(0.0, ty), vec2<f32>(tx, ty),
    );
    let weights = array<f32, 9>(
        row0.x, row0.y, row0.z,
        row1.x, row1.y, row1.z,
        row2.x, row2.y, row2.z,
    );
    for (var i = 0u; i < 9u; i = i + 1u) {
        acc += sample_with_edge_mode(in.uv + offsets[i], edge_mode) * weights[i];
    }
    var result = acc / divisor + vec4<f32>(bias);
    if (preserve_alpha) {
        result.a = sample_in1(in.uv).a;
    }
    return clamp(result, vec4<f32>(0.0), vec4<f32>(1.0));
}

// ---- feComponentTransfer --------------------------------------------------

// Index up to eight floats stored in two vec4s, returning zero out of range.
fn table_pick(p0: vec4<f32>, p1: vec4<f32>, idx: i32) -> f32 {
    if (idx == 0) { return p0.x; }
    if (idx == 1) { return p0.y; }
    if (idx == 2) { return p0.z; }
    if (idx == 3) { return p0.w; }
    if (idx == 4) { return p1.x; }
    if (idx == 5) { return p1.y; }
    if (idx == 6) { return p1.z; }
    if (idx == 7) { return p1.w; }
    return 0.0;
}

fn transfer_apply(value: f32, kind: u32, p0: vec4<f32>, p1: vec4<f32>, count: u32) -> f32 {
    if (kind == 0u) {
        // Identity.
        return value;
    }
    if (kind == 3u) {
        // Linear: y = slope * C + intercept (params in p0.x / p0.y).
        return p0.x * value + p0.y;
    }
    if (kind == 4u) {
        // Gamma: y = amplitude * C^exponent + offset (params in p0.x / p0.y / p0.z).
        return p0.x * pow(max(value, 0.0), p0.y) + p0.z;
    }
    // Table / discrete share the same up-to-eight-entry storage.
    let n = i32(count);
    if (n < 2) {
        return value;
    }
    let v = clamp(value, 0.0, 1.0);
    if (kind == 1u) {
        // Table: piecewise-linear over N control points spanning [0, 1].
        // N points define N - 1 segments.
        let segments = f32(n - 1);
        let scaled = v * segments;
        let i0 = clamp(i32(floor(scaled)), 0, n - 2);
        let i1 = i0 + 1;
        let t = clamp(scaled - f32(i0), 0.0, 1.0);
        let a = table_pick(p0, p1, i0);
        let b = table_pick(p0, p1, i1);
        return mix(a, b, t);
    }
    if (kind == 2u) {
        // Discrete: piecewise-constant over N equal buckets.
        let idx = clamp(i32(floor(v * f32(n))), 0, n - 1);
        return table_pick(p0, p1, idx);
    }
    return value;
}

@fragment
fn fs_component_transfer(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = sample_in1(in.uv);
    let straight = unpremultiply(src);
    let kinds = filter_params.transfer_kinds;
    let counts = filter_params.transfer_counts;
    let r = transfer_apply(
        straight.r, kinds.x,
        filter_params.transfer_r0, filter_params.transfer_r1,
        counts.x,
    );
    let g = transfer_apply(
        straight.g, kinds.y,
        filter_params.transfer_g0, filter_params.transfer_g1,
        counts.y,
    );
    let b = transfer_apply(
        straight.b, kinds.z,
        filter_params.transfer_b0, filter_params.transfer_b1,
        counts.z,
    );
    let a = transfer_apply(
        straight.a, kinds.w,
        filter_params.transfer_a0, filter_params.transfer_a1,
        counts.w,
    );
    let result = clamp(vec4<f32>(r, g, b, a), vec4<f32>(0.0), vec4<f32>(1.0));
    return premultiply(result);
}

// ---- feOffset -------------------------------------------------------------
//
// SVG 1.1 §15.16: shift the input by `(dx, dy)` filter pixels. `direction`
// carries the *UV-space* offset (already converted from filter-pixel space
// by the renderer-side uniform builder). Samples outside the source are
// clamped to the edge by the filter sampler, which would smear the source's
// edge across the offset region — undesirable. We replicate the spec's
// "transparent black outside source" by explicitly returning transparent
// outside the [0,1] UV box.

@fragment
fn fs_offset(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv - filter_params.direction;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return vec4<f32>(0.0);
    }
    return sample_in1(uv);
}

// ---- feBlend --------------------------------------------------------------
//
// SVG 1.1 §15.7: `result = src.rgb * (1 - dst.a) + dst.rgb * (1 - src.a) + f(src, dst, mode)`.
// The blend-mode-specific term lives in `blend_color`. Both inputs are
// premultiplied; the spec describes the math on straight-alpha values, so we
// unpremultiply first, then re-premultiply the result.

fn blend_normal(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return s;
}

fn blend_multiply(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return s * d;
}

fn blend_screen(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return s + d - s * d;
}

fn blend_darken(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return min(s, d);
}

fn blend_lighten(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return max(s, d);
}

@fragment
fn fs_blend(in: VertexOutput) -> @location(0) vec4<f32> {
    // Filter targets are premultiplied; the blend math operates on straight
    // RGB but the linear-mixing terms below need to keep the result
    // premultiplied so downstream primitives and the final composite see a
    // valid premultiplied texel.
    let src = sample_in1(in.uv);
    let dst = sample_in2(in.uv);
    let s = unpremultiply(src);
    let d = unpremultiply(dst);

    var blended: vec3<f32>;
    let mode = filter_params.mode;
    if (mode == 1u) {
        blended = blend_multiply(s.rgb, d.rgb);
    } else if (mode == 2u) {
        blended = blend_screen(s.rgb, d.rgb);
    } else if (mode == 3u) {
        blended = blend_darken(s.rgb, d.rgb);
    } else if (mode == 4u) {
        blended = blend_lighten(s.rgb, d.rgb);
    } else {
        blended = blend_normal(s.rgb, d.rgb);
    }

    // SVG 1.1 §15.7, written on premultiplied colour so the output stays
    // premultiplied:
    //   Co = (1 - αb) * Ca + (1 - αa) * Cb + αa * αb * B(Ca/αa, Cb/αb)
    //   αo = αa + αb - αa * αb
    // The blend function `B` operates on straight RGB (`blended` above);
    // the linear terms use `src.rgb` and `dst.rgb` (already premultiplied).
    let rgb = (1.0 - dst.a) * src.rgb + (1.0 - src.a) * dst.rgb + src.a * dst.a * blended;
    let a = src.a + dst.a - src.a * dst.a;
    return clamp(vec4<f32>(rgb, a), vec4<f32>(0.0), vec4<f32>(1.0));
}

// ---- feComposite ----------------------------------------------------------
//
// SVG 1.1 §15.6: Porter-Duff `over | in | out | atop | xor`, plus the
// arithmetic mode `k1*src*dst + k2*src + k3*dst + k4`. The Porter-Duff math
// is written on premultiplied colour; arithmetic uses straight colour.

@fragment
fn fs_fe_composite(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = sample_in1(in.uv);
    let dst = sample_in2(in.uv);
    let op = filter_params.mode;
    var result: vec4<f32>;
    if (op == 1u) {
        // in: src * dst.a
        result = src * dst.a;
    } else if (op == 2u) {
        // out: src * (1 - dst.a)
        result = src * (1.0 - dst.a);
    } else if (op == 3u) {
        // atop: src * dst.a + dst * (1 - src.a)
        result = src * dst.a + dst * (1.0 - src.a);
    } else if (op == 4u) {
        // xor: src * (1 - dst.a) + dst * (1 - src.a)
        result = src * (1.0 - dst.a) + dst * (1.0 - src.a);
    } else if (op == 5u) {
        // arithmetic: result = k1*src*dst + k2*src + k3*dst + k4 (on straight RGB)
        let s = unpremultiply(src);
        let d = unpremultiply(dst);
        let k1 = filter_params.extra.x;
        let k2 = filter_params.extra.y;
        let k3 = filter_params.extra.z;
        let k4 = filter_params.extra.w;
        let rgb = k1 * s.rgb * d.rgb + k2 * s.rgb + k3 * d.rgb + vec3<f32>(k4);
        let alpha = clamp(k1 * s.a * d.a + k2 * s.a + k3 * d.a + k4, 0.0, 1.0);
        result = vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * alpha, alpha);
    } else {
        // over: src + dst * (1 - src.a)
        result = src + dst * (1.0 - src.a);
    }
    return clamp(result, vec4<f32>(0.0), vec4<f32>(1.0));
}

// ---- feMerge --------------------------------------------------------------
//
// The renderer encodes `<feMerge>` as one source-over composite per child
// node; this entry point composites `src` ON TOP OF `dst` and the renderer
// drives it once per node. Same math as `fs_composite` op=over but with
// inputs labelled by accumulation role.

@fragment
fn fs_merge_step(in: VertexOutput) -> @location(0) vec4<f32> {
    let above = sample_in1(in.uv);
    let below = sample_in2(in.uv);
    let result = above + below * (1.0 - above.a);
    return clamp(result, vec4<f32>(0.0), vec4<f32>(1.0));
}

// ---- clipPath / mask ------------------------------------------------------
//
// The first input is a premultiplied source texture. The second input is a
// rendered clip-path or mask texture. `mode = 0` uses mask alpha directly
// (clipPath and `mask-type="alpha"`); `mode = 1` uses luminance * alpha
// (SVG mask default).

@fragment
fn fs_alpha_mask(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = sample_in1(in.uv);
    let mask = sample_in2(in.uv);
    var factor: f32;
    if (filter_params.mode == 1u) {
        factor = luminance(unpremultiply(mask).rgb) * mask.a;
    } else {
        factor = mask.a;
    }
    return src * clamp(factor, 0.0, 1.0);
}

// ---- Primitive subregion clip (SVG 1.1 §15.5) -----------------------------
//
// Run as a post-pass on a primitive's output when the primitive carries an
// authored `x`/`y`/`width`/`height` subregion. `extra.xy` carries the
// subregion's top-left in UV space and `extra.zw` carries its size. Pixels
// outside the subregion are transparent black; inside they pass through.

@fragment
fn fs_subregion_clip(in: VertexOutput) -> @location(0) vec4<f32> {
    let origin = filter_params.extra.xy;
    let size = filter_params.extra.zw;
    let inside = all(in.uv >= origin) && all(in.uv <= origin + size);
    if (!inside) {
        return vec4<f32>(0.0);
    }
    return sample_in1(in.uv);
}

// ---- Nested <svg> viewport clip ------------------------------------------
//
// Normal 2D SVG mode gives every nested <svg> its own viewport and clips
// overflowing child content by default. `extra.xy` carries the root viewport
// dimensions, `extra.zw` carries the nested viewport's top-left in the
// parent user coordinate system, `lighting.xy` carries its size, and
// `matrix_r0/r1.xyz` carry the inverse parent->root affine transform rows.

@fragment
fn fs_svg_viewport_clip(in: VertexOutput) -> @location(0) vec4<f32> {
    let root_p = vec2<f32>(
        in.uv.x * filter_params.extra.x,
        in.uv.y * filter_params.extra.y,
    );
    let v = vec3<f32>(root_p.x, root_p.y, 1.0);
    let local = vec2<f32>(
        dot(filter_params.matrix_r0.xyz, v),
        dot(filter_params.matrix_r1.xyz, v),
    );
    let origin = filter_params.extra.zw;
    let size = filter_params.lighting.xy;
    let inside = all(local >= origin) && all(local <= origin + size);
    if (!inside) {
        return vec4<f32>(0.0);
    }
    return sample_in1(in.uv);
}

// ---- feTile ---------------------------------------------------------------
//
// SVG 1.1 §15.27: tile the input subregion across the filter region. The
// uniform's `extra.xy` carries the source rect's top-left in UV space and
// `extra.zw` carries its size; both are computed by the renderer from the
// upstream primitive's subregion (default = SourceGraphic = full target).
// `fract()` would suffice for a 0..1 source, but a subregion-based tile must
// wrap inside the rectangle.

@fragment
fn fs_tile(in: VertexOutput) -> @location(0) vec4<f32> {
    let origin = filter_params.extra.xy;
    let size = filter_params.extra.zw;
    let safe_size = max(size, vec2<f32>(1e-6));
    let local = (in.uv - origin) / safe_size;
    let wrapped = fract(local - floor(local));
    let uv = origin + wrapped * safe_size;
    return sample_in1(uv);
}
