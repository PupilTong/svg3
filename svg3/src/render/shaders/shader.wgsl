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
const KIND_SEGMENT: u32 = 3u;

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

const PAINT_LINEAR_GRADIENT: u32 = 1u;
const PAINT_RADIAL_GRADIENT: u32 = 2u;
const PAINT_PATTERN: u32 = 3u;
const PAINT_SVG_TEXTURE: u32 = 4u;
const MAX_GRADIENT_STOPS: u32 = 8u;
const MAX_PATTERN_ITEMS: u32 = 8u;

struct PaintServer {
    // [kind, layer_or_count, mapping_kind_or_pattern_count, mapping_aux]
    //  - gradients use [kind, stop_count, 0, 0]
    //  - pattern uses  [kind, 0, pattern_item_count, 0]
    //  - svg texture   [kind, layer_index, mapping_kind, mapping_aux]
    header: vec4<u32>,
    // linear: [x1, y1, x2, y2], radial: [cx, cy, r, 0],
    // pattern: [x, y, width, height]
    geometry: vec4<f32>,
    colors: array<vec4<f32>, 8>,
    offsets: array<vec4<f32>, 2>,
    pattern_rects: array<vec4<f32>, 8>,
    pattern_colors: array<vec4<f32>, 8>,
}

@group(0) @binding(1)
var<storage, read> paints: array<PaintServer>;

// `PAINT_SVG_TEXTURE` paint servers sample this 2D-array texture; the
// `layer_index` field of the paint server's header picks the layer. When
// the document references no textures the renderer binds a 1×1 transparent
// dummy here, so this binding is always present regardless of content.
@group(1) @binding(0)
var svg_textures: texture_2d_array<f32>;
@group(1) @binding(1)
var svg_texture_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) params: vec4<f32>,
    @location(3) @interpolate(flat) kind: u32,
    @location(4) world_position: vec3<f32>,
    @location(5) @interpolate(flat) paint_id: u32,
    @location(6) uv: vec2<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) local: vec2<f32>,
    @location(3) params: vec4<f32>,
    @location(4) kind: u32,
    @location(5) paint_id: u32,
    @location(6) uv: vec2<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = transform.view_projection * vec4<f32>(position, 1.0);
    out.color = color;
    out.local = local;
    out.params = params;
    out.kind = kind;
    out.world_position = position;
    out.paint_id = paint_id;
    out.uv = uv;
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

fn stop_offset(paint: PaintServer, index: u32) -> f32 {
    if (index < 4u) {
        return paint.offsets[0][index];
    }
    return paint.offsets[1][index - 4u];
}

fn sample_gradient_stops(paint: PaintServer, t_unclamped: f32) -> vec4<f32> {
    let count = min(paint.header.y, MAX_GRADIENT_STOPS);
    if (count == 0u) {
        return vec4<f32>(0.0);
    }

    let t = clamp(t_unclamped, 0.0, 1.0);
    var previous_offset = stop_offset(paint, 0u);
    var previous_color = paint.colors[0];
    if (count == 1u || t <= previous_offset) {
        return previous_color;
    }

    for (var i = 1u; i < MAX_GRADIENT_STOPS; i = i + 1u) {
        if (i >= count) {
            break;
        }
        let next_offset = stop_offset(paint, i);
        let next_color = paint.colors[i];
        if (t <= next_offset) {
            let span = max(next_offset - previous_offset, 1e-6);
            return mix(previous_color, next_color, (t - previous_offset) / span);
        }
        previous_offset = next_offset;
        previous_color = next_color;
    }
    return previous_color;
}

fn source_over(src: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
    let out_a = src.a + dst.a * (1.0 - src.a);
    if (out_a <= 1e-6) {
        return vec4<f32>(0.0);
    }
    let out_rgb = (src.rgb * src.a + dst.rgb * dst.a * (1.0 - src.a)) / out_a;
    return vec4<f32>(out_rgb, out_a);
}

fn sample_pattern(paint: PaintServer, p: vec2<f32>) -> vec4<f32> {
    let tile = paint.geometry;
    if (tile.z <= 0.0 || tile.w <= 0.0) {
        return vec4<f32>(0.0);
    }

    let item_count = min(paint.header.z, MAX_PATTERN_ITEMS);
    let tile_size = tile.zw;
    let relative = p - tile.xy;
    let tile_p = relative - floor(relative / tile_size) * tile_size;
    var out = vec4<f32>(0.0);
    for (var i = 0u; i < MAX_PATTERN_ITEMS; i = i + 1u) {
        if (i >= item_count) {
            break;
        }
        let rect = paint.pattern_rects[i];
        if (
            tile_p.x >= rect.x && tile_p.y >= rect.y &&
            tile_p.x < rect.x + rect.z && tile_p.y < rect.y + rect.w
        ) {
            out = source_over(paint.pattern_colors[i], out);
        }
    }
    return out;
}

fn evaluate_paint(in: VertexOutput) -> vec4<f32> {
    if (in.paint_id == 0u) {
        return in.color;
    }

    let paint = paints[in.paint_id];
    let p = in.world_position.xy;
    if (paint.header.x == PAINT_LINEAR_GRADIENT) {
        let a = paint.geometry.xy;
        let b = paint.geometry.zw;
        let axis = b - a;
        let len2 = dot(axis, axis);
        let t = select(0.0, dot(p - a, axis) / len2, len2 > 1e-6);
        return sample_gradient_stops(paint, t);
    }
    if (paint.header.x == PAINT_RADIAL_GRADIENT) {
        let radius = paint.geometry.z;
        let t = select(0.0, distance(p, paint.geometry.xy) / radius, radius > 1e-6);
        return sample_gradient_stops(paint, t);
    }
    if (paint.header.x == PAINT_PATTERN) {
        return sample_pattern(paint, p);
    }
    if (paint.header.x == PAINT_SVG_TEXTURE) {
        // UV is in `[0, 1]²` by tessellation; the per-primitive remap
        // (e.g. cube-cross slot offsets) is baked into the vertex UV at
        // mesh-build time, so the shader sample is a straight lookup. The
        // layer index lives in `header.y` (the renderer rewrites the
        // node-id placeholder to the array layer before buffer upload).
        return textureSampleLevel(
            svg_textures,
            svg_texture_sampler,
            in.uv,
            i32(paint.header.y),
            0.0,
        );
    }
    return in.color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let paint_color = evaluate_paint(in);
    // Fully transparent paint should not occupy the shared depth buffer.
    // Otherwise an invisible 2D shape with `fill-opacity="0"` would still
    // occlude later 3D content behind it.
    if (paint_color.a <= 0.0) {
        discard;
    }

    if (in.kind == KIND_SOLID) {
        return paint_color;
    }

    var dist: f32;
    if (in.kind == KIND_ELLIPSE) {
        dist = sd_ellipse(in.local, in.params.xy);
    } else if (in.kind == KIND_ROUND_BOX) {
        dist = sd_round_box(in.local, in.params.xy, in.params.zw);
    } else {
        dist = sd_box(in.local, in.params.xy);
    }

    // Discard SDF fragments fully outside the analytic shape so the depth
    // buffer doesn't pick up the bounding quad's transparent corners. The
    // anti-aliasing band sits within `SDF_PAD` user units of the shape edge
    // (see `coverage`), so any fragment whose distance exceeds that band
    // contributes no colour — and must not contribute depth either.
    if (dist >= SDF_PAD) {
        discard;
    }

    // Straight (non-premultiplied) alpha: fold coverage into alpha only — the
    // pipeline's `ALPHA_BLENDING` multiplies rgb by alpha itself.
    return vec4<f32>(paint_color.rgb, paint_color.a * coverage(dist, in.local));
}
