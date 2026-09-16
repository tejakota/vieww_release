// Solid-colour fill: the same shape `vieww-hal::vulkan`'s `MESH_SHADER_WGSL`
// already ships and this crate's `library` module now re-exports that exact
// source from (see `library::SOLID`) rather than duplicating it — a mesh of
// positions in logical-pixel space, painted one flat colour.

struct Uniforms {
    color: vec4<f32>,
    // .xy is the viewport size in the same units the incoming positions are
    // already in (logical pixels); .zw unused, kept so the whole struct is
    // one 16-byte-aligned vec4 pair with no manual padding to get wrong.
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> {
    // # The Y axis, and the flip that is already happening
    //
    // WGSL is authored in **WebGPU**'s clip space, where NDC +Y is *up*: y =
    // +1 is the top of the framebuffer. Vulkan's is the other way up. That
    // difference is not this shader's to resolve — `naga`'s SPIR-V writer
    // does it, via `WriterFlags::ADJUST_COORDINATE_SPACE`, which is on in the
    // `Options::default()` that `vieww_shaders::compiler` uses and which
    // flips `@builtin(position).y` on the way out.
    //
    // So the correct thing to write here is the WebGPU mapping, and letting
    // naga adjust it per backend is exactly what keeps one shader source
    // working for Metal and D3D12 too — each has its own convention and its
    // own naga backend to reconcile it.
    //
    // Writing the *Vulkan* mapping here instead — which this file did, under
    // a comment asserting that pixel space "maps directly onto Vulkan's clip
    // space (also y-down by default)" — produces a vertically mirrored
    // frame, because naga then flips the already-correct result. It survived
    // because the only test drawing through it used a square that was
    // symmetric about the horizontal centre line, where a vertical flip is
    // invisible. `vieww-hal`'s `vulkan_scene` suite compares against the CPU
    // rasterizer with deliberately asymmetric geometry, which is what caught
    // it.
    let normalized = position / u.viewport.xy;
    let ndc = vec2<f32>(normalized.x * 2.0 - 1.0, 1.0 - normalized.y * 2.0);
    return vec4<f32>(ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return u.color;
}
