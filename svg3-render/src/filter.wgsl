// Full-screen GPU post-processing for SVG filters.
//
// Filtered geometry is first drawn into an offscreen texture by the regular
// shape pipeline. Because that pass uses alpha blending over transparent, the
// texture stores premultiplied RGB. The blur pass therefore accumulates and
// writes premultiplied colour, and the composite pass uses premultiplied alpha
// blending when it lands on the final target.
//
// The sampler clamps at the render-target edge. Shapes blurred against the
// canvas boundary therefore smear their edge texels instead of sampling an SVG
// filter region expanded with transparent pixels; bbox-derived filter regions
// are a future renderer milestone.

struct FilterUniform {
    texel_size: vec2<f32>,
    direction: vec2<f32>,
    sigma: f32,
    radius: u32,
    _pad: vec2<u32>,
}

@group(0) @binding(0)
var source_texture: texture_2d<f32>;
@group(0) @binding(1)
var source_sampler: sampler;
@group(0) @binding(2)
var<uniform> filter_params: FilterUniform;

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

@fragment
fn fs_blur(in: VertexOutput) -> @location(0) vec4<f32> {
    if (filter_params.radius == 0u || filter_params.sigma <= 0.0) {
        return textureSample(source_texture, source_sampler, in.uv);
    }

    let two_sigma_sq = 2.0 * filter_params.sigma * filter_params.sigma;
    var weighted_sum = vec4<f32>(0.0);
    var weight_sum = 0.0;
    let radius = i32(filter_params.radius);

    for (var i = -radius; i <= radius; i = i + 1) {
        let offset = f32(i);
        let weight = exp(-(offset * offset) / two_sigma_sq);
        let uv = in.uv + filter_params.direction * filter_params.texel_size * offset;
        let sample = textureSample(source_texture, source_sampler, uv);
        weighted_sum += sample * weight;
        weight_sum += weight;
    }

    return weighted_sum / weight_sum;
}

@fragment
fn fs_composite(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source_texture, source_sampler, in.uv);
}
