// Textured mesh: sample a source image at each fragment's UV, tinted by a
// uniform multiply colour (white/`vec4(1.0)` for "no tint" — the common
// case, and how `vieww_paint::Paint::Image` with no colour filter maps
// here) — the same role `vieww-image`'s decoded textures play once uploaded
// through `vieww-gpu`'s resource heap.

struct Uniforms {
    // Multiplied into the sampled texel; (1, 1, 1, 1) for an untinted draw.
    tint: vec4<f32>,
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;
@group(0) @binding(1)
var t_image: texture_2d<f32>;
@group(0) @binding(2)
var s_image: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    let ndc = (position / u.viewport.xy) * 2.0 - vec2<f32>(1.0, 1.0);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_image, s_image, in.uv) * u.tint;
}
