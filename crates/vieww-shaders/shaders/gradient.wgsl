// Two-stop linear gradient, interpolated in the fragment shader along a
// caller-supplied axis — `vieww_paint::Paint`'s `LinearGradient` is exactly
// two colours plus a start/end point, which is all this shader needs to
// stay generic over: everything about *which* two colours and *which* axis
// is data (the uniform), not shader variants.

struct Uniforms {
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    // .xy is the viewport size (logical pixels, same convention as
    // `solid.wgsl`); .zw unused (padding to keep the struct's alignment
    // explicit rather than implicit).
    viewport: vec4<f32>,
    // The gradient axis in the same logical-pixel space as `position`:
    // .xy is the start point, .zw is the end point.
    axis: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    let ndc = (position / u.viewport.xy) * 2.0 - vec2<f32>(1.0, 1.0);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.world_position = position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let start = u.axis.xy;
    let end = u.axis.zw;
    let axis_vector = end - start;
    // A degenerate (zero-length) axis would divide by zero; fall back to
    // the gradient's start colour rather than propagating NaN into the
    // framebuffer — the same "never worse than the input" rule
    // `vieww_paint`'s CPU gradient rasterizer already follows.
    let axis_length_sq = dot(axis_vector, axis_vector);
    var t: f32 = 0.0;
    if axis_length_sq > 0.00001 {
        t = clamp(dot(in.world_position - start, axis_vector) / axis_length_sq, 0.0, 1.0);
    }
    return mix(u.color_start, u.color_end, t);
}
