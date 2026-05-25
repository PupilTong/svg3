// One-draw GPU frustum culling for a prepared GpuScene.
//
// The CPU owns camera/model updates, but visibility is decided on the GPU by
// testing the scene-local bounding box against clip-space planes. The compute
// pass writes only the indirect draw's instance count: 1 for visible, 0 for
// culled.

struct Transform {
    view_projection: mat4x4<f32>,
    model: mat4x4<f32>,
}

struct Bounds {
    min_corner: vec4<f32>,
    max_corner: vec4<f32>,
}

struct DrawIndexedIndirect {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

@group(0) @binding(0)
var<uniform> transform: Transform;
@group(0) @binding(1)
var<uniform> bounds: Bounds;
@group(0) @binding(2)
var<storage, read_write> indirect: DrawIndexedIndirect;

fn corner(index: u32) -> vec4<f32> {
    let x = select(bounds.min_corner.x, bounds.max_corner.x, (index & 1u) != 0u);
    let y = select(bounds.min_corner.y, bounds.max_corner.y, (index & 2u) != 0u);
    let z = select(bounds.min_corner.z, bounds.max_corner.z, (index & 4u) != 0u);
    return transform.view_projection * transform.model * vec4<f32>(x, y, z, 1.0);
}

@compute @workgroup_size(1)
fn cs_cull() {
    var outside_left = true;
    var outside_right = true;
    var outside_bottom = true;
    var outside_top = true;
    var outside_near = true;
    var outside_far = true;

    for (var i = 0u; i < 8u; i = i + 1u) {
        let p = corner(i);
        outside_left = outside_left && p.x < -p.w;
        outside_right = outside_right && p.x > p.w;
        outside_bottom = outside_bottom && p.y < -p.w;
        outside_top = outside_top && p.y > p.w;
        outside_near = outside_near && p.z < 0.0;
        outside_far = outside_far && p.z > p.w;
    }

    let culled =
        outside_left ||
        outside_right ||
        outside_bottom ||
        outside_top ||
        outside_near ||
        outside_far;
    indirect.instance_count = select(1u, 0u, culled);
}
