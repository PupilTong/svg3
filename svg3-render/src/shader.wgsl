// Minimal shader for filled basic-shape geometry.
//
// Vertex positions arrive in SVG/world space. The vertex stage applies the
// configured view-projection matrix, then forwards linear RGBA colours to the
// fragment stage. The `Rgba8UnormSrgb` target encodes them to sRGB on store.

struct Transform {
    view_projection: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = transform.view_projection * vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
