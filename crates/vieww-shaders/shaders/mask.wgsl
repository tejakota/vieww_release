// Alpha masking: multiply a content texture's alpha by a separate mask
// texture's coverage (its red channel, the same single-channel convention
// `text.wgsl`/`shadow.wgsl` use) — the GPU equivalent of
// `vieww_paint::Canvas::push_masked_layer`'s clip-by-arbitrary-shape, where
// the mask is rendered to its own texture once and then reused as a
// multiplier rather than re-rasterized per composite.

struct Uniforms {
    // Multiplies the final alpha in addition to the mask, so a fading-out
    // masked layer (opacity animation over a mask) doesn't need a second
    // pass; 1.0 for "mask alone decides".
    opacity: f32,
    _padding: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;
@group(0) @binding(1)
var t_content: texture_2d<f32>;
@group(0) @binding(2)
var s_content: sampler;
@group(0) @binding(3)
var t_mask: texture_2d<f32>;
@group(0) @binding(4)
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
    let content = textureSample(t_content, s_content, in.uv);
    let mask_coverage = textureSample(t_mask, s_mask, in.uv).r;
    return vec4<f32>(content.rgb, content.a * mask_coverage * u.opacity);
}
