// Minimal 2D pass-through shader for filled `<rect>` geometry.
//
// Vertex positions arrive already in clip space — the default-surface
// orthographic projection (`RenderConfig::projection`) is applied on the
// CPU — so the vertex stage only forwards position and colour. Colours are
// linear RGBA; the `Rgba8UnormSrgb` target encodes them to sRGB on store.

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
    out.clip_position = vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
