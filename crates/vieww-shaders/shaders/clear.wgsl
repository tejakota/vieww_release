// Whole-viewport clear: a hardcoded fullscreen triangle (no vertex buffer
// needed — `vertex_index` alone selects one of three clip-space corners
// that together cover the whole viewport) painted one flat colour. Moved
// from `vieww-hal::vulkan`'s own `CLEAR_SHADER_WGSL` constant, which this
// crate's `library::CLEAR_WGSL` now re-exports so it has one home instead
// of living only inside that backend.
//
// Distinct from `solid.wgsl`: that shader paints an actual mesh
// (`vieww-gpu::tessellate`'s triangles) at real positions; this one paints
// every pixel the same colour with no input geometry at all — the M0/M1
// smoke test's "render `width x height` of `clear_color`, nothing else
// drawn" case, and `vieww-hal::vulkan::VulkanDevice::render_clear_to_pixels`'s
// shader.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}

struct Uniforms {
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return u.color;
}
