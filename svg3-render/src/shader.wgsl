// SDF-coverage shader for svg3 basic-shape geometry.
//
// Vertex positions arrive in SVG/world space; the vertex stage applies the
// configured view-projection matrix. Solid triangle geometry (`KIND_SOLID`)
// is painted at full coverage. The curved primitives arrive as bounding
// quads: the fragment stage evaluates a signed-distance function on the
// interpolated shape-local coordinate and derives anti-aliased coverage.
// Colours are linear RGBA; the `Rgba8UnormSrgb` target encodes them on store.

// Shape-kind tags. Must stay in sync with the `KIND_*` constants in shape.rs.
const KIND_SOLID: u32 = 0u;
const KIND_ELLIPSE: u32 = 1u;
const KIND_ROUND_BOX: u32 = 2u;

// Per-side bounding-quad inflation, in shape-local (user) units. Must match
// `SDF_PAD` in shape.rs. The coverage ramp below is clamped to this width so
// the anti-aliasing band never runs past the quad — outside it there are no
// fragments to shade.
const SDF_PAD: f32 = 1.0;

struct Transform {
    view_projection: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) params: vec4<f32>,
    @location(3) @interpolate(flat) kind: u32,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) local: vec2<f32>,
    @location(3) params: vec4<f32>,
    @location(4) kind: u32,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = transform.view_projection * vec4<f32>(position, 1.0);
    out.color = color;
    out.local = local;
    out.params = params;
    out.kind = kind;
    return out;
}

// Signed distance (user units, negative inside) to an axis-aligned ellipse
// centred at the origin with radii `ab`. A gradient-normalised approximation,
// exact for a circle (`ab.x == ab.y`). The origin is the deep interior.
fn sd_ellipse(p: vec2<f32>, ab: vec2<f32>) -> f32 {
    let k1 = length(p / ab);
    if (k1 < 1e-6) {
        return -min(ab.x, ab.y);
    }
    let k2 = length(p / (ab * ab));
    return (k1 - 1.0) * (k1 / k2);
}

// Signed distance to an axis-aligned box of half-extents `b`.
fn sd_box(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = abs(p) - b;
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0)));
}

// Signed distance to a box of half-extents `b` with independent elliptical
// corner radii `r` (an SVG `<rect>` corner is a quarter ellipse). `r` is
// clamped to `b` by the caller, so the corner-ellipse centre is non-negative.
fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: vec2<f32>) -> f32 {
    let q = abs(p);
    let corner = q - (b - r);
    if (corner.x > 0.0 && corner.y > 0.0) {
        return sd_ellipse(corner, r);
    }
    let d = q - b;
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0)));
}

// Anti-aliased coverage in `[0, 1]` from a signed distance. `local` is the
// linearly-interpolated shape-local coordinate; `fwidth(local)` measures user
// units per device pixel, so the coverage ramp is ~1px wide in any
// projection — including the perspective camera.
//
// The ramp half-width is clamped to `SDF_PAD`: the bounding quad is inflated
// only that far past the shape, so under heavy minification (more than two
// user units per device pixel) an unclamped ramp would extend beyond the
// quad and be clipped where there are no fragments. Clamping narrows the
// ramp instead, keeping the whole anti-aliased edge inside the quad.
fn coverage(dist: f32, local: vec2<f32>) -> f32 {
    let aa = min(max(fwidth(local.x), fwidth(local.y)) * 0.5, SDF_PAD);
    return 1.0 - smoothstep(-aa, aa, dist);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.kind == KIND_SOLID) {
        return in.color;
    }

    var dist: f32;
    if (in.kind == KIND_ELLIPSE) {
        dist = sd_ellipse(in.local, in.params.xy);
    } else if (in.kind == KIND_ROUND_BOX) {
        dist = sd_round_box(in.local, in.params.xy, in.params.zw);
    } else {
        dist = sd_box(in.local, in.params.xy);
    }

    // Straight (non-premultiplied) alpha: fold coverage into alpha only — the
    // pipeline's `ALPHA_BLENDING` multiplies rgb by alpha itself.
    return vec4<f32>(in.color.rgb, in.color.a * coverage(dist, in.local));
}
