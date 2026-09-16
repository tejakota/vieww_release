// Drop shadow: sample a shape's coverage mask (typically the output of
// `blur.wgsl` run over the shape's own alpha, per `vieww_foundation::Shadow`
// — offset, blur radius, spread, colour), shift it by an offset, and tint
// it with the shadow's colour. This is deliberately a separate, simpler
// shader from `blur.wgsl` rather than folding the offset into it: a shadow
// needs one solid-colour tint of a pre-blurred mask, not another blur pass.

struct Uniforms {
    color: vec4<f32>,
    // .xy: shadow offset in UV space (already normalized by the caller to
    // the source texture's size, the same way `direction_sigma` in
    // `blur.wgsl` is pre-divided by `texture_size`); .zw unused.
    offset: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;
@group(0) @binding(1)
var t_mask: texture_2d<f32>;
@group(0) @binding(2)
var s_mask: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let shifted_uv = in.uv - u.offset.xy;
    // Outside [0, 1] the mask contributes no shadow — a shadow should never
    // wrap or clamp-smear at the source shape's edges.
    if any(shifted_uv < vec2<f32>(0.0, 0.0)) || any(shifted_uv > vec2<f32>(1.0, 1.0)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let coverage = textureSample(t_mask, s_mask, shifted_uv).r;
    return vec4<f32>(u.color.rgb, u.color.a * coverage);
}
