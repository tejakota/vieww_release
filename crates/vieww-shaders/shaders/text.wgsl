// GPU glyph rendering: sample a single-channel coverage atlas (the standard
// glyph-atlas representation — `vieww-text`'s rasterized glyphs are exactly
// this, one alpha value per texel) and use it as the alpha of a solid text
// colour. This is the "coverage mask times colour" technique every
// GPU-accelerated text renderer uses, deliberately not full RGBA glyphs, so
// one atlas texel format serves every text colour a UI ever draws with it.

struct Uniforms {
    color: vec4<f32>,
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;
@group(0) @binding(1)
var t_atlas: texture_2d<f32>;
@group(0) @binding(2)
var s_atlas: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) atlas_uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) atlas_uv: vec2<f32>) -> VertexOutput {
    // # The Y axis
    //
    // This line used to read `(position / u.viewport.xy) * 2.0 - vec2(1.0)`,
    // which maps y = 0 to NDC -1. In WebGPU's clip space — the space WGSL is
    // authored in, and the one `naga`'s SPIR-V writer adjusts *from* via
    // `WriterFlags::ADJUST_COORDINATE_SPACE` — NDC +Y is up, so that put the
    // top of the layout at the bottom of the framebuffer: every frame through
    // this shader was vertically mirrored.
    //
    // It is the same defect `solid.wgsl` and `scene.wgsl` carry a long comment
    // about having had, and it survived here for the reason that makes this
    // class of bug worth naming: **nothing ever drew through this shader**, so
    // no test could disagree with it. The whole-scene path (`vieww_gpu::scene`
    // and `scene.wgsl`) is what actually renders text, and its parity tests use
    // asymmetric geometry precisely so a flip cannot pass. This file is kept
    // for a future single-run text pass, and is corrected so that pass does not
    // inherit a mirrored frame the day it is written.
    let normalized = position / u.viewport.xy;
    let ndc = vec2<f32>(normalized.x * 2.0 - 1.0, 1.0 - normalized.y * 2.0);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.atlas_uv = atlas_uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The atlas is single-channel coverage stored in the red channel (the
    // convention `vieww-text`'s glyph rasterizer already writes, matching
    // e.g. FreeType/cosmic-text's own greyscale-atlas output). Coverage
    // scales the alpha channel only, leaving `color.rgb` un-premultiplied —
    // straight alpha, the same convention every other shader in this
    // library returns and `vieww-hal`'s render targets read back as.
    let coverage = textureSample(t_atlas, s_atlas, in.atlas_uv).r;
    return vec4<f32>(u.color.rgb, u.color.a * coverage);
}
