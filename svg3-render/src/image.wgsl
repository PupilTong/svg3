// Textured-rectangle shader for `<feImage>` filter sources.
//
// The CPU decodes embedded PNGs on demand, uploads them as sRGB textures, and
// this pass samples them into the same render target format used by the rest
// of the renderer. Vertex positions are generated from a uniform SVG-space
// rectangle so image filters share the same camera/projection path as shapes.

struct ImageUniform {
    view_projection: mat4x4<f32>,
    rect: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> image_params: ImageUniform;
@group(0) @binding(1)
var image_texture: texture_2d<f32>;
@group(0) @binding(2)
var image_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_image(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = corners[vertex_index];
    let position = image_params.rect.xy + uv * image_params.rect.zw;

    var out: VertexOutput;
    out.clip_position = image_params.view_projection * vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_image(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image_texture, image_sampler, in.uv);
}
