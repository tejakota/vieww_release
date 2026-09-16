// Whole-scene geometry: one batched mesh, one pipeline, four materials.
//
// `vieww_gpu::scene` packs every fill, stroke, glyph, image and gradient in a
// frame into one vertex buffer. The material is a per-vertex value, so a
// panel, its gradient border, the photo on it and the label under the photo
// are one batch; the only thing that breaks a run is a scissor, which is
// dynamic state.
//
// # Output is premultiplied
//
// Every target this shader draws into — the frame and every offscreen layer —
// holds premultiplied RGBA, and the pipeline blends ONE / ONE_MINUS_SRC_ALPHA.
// That is `NativeRenderer`'s working representation, which is the point: a
// layer composited later must hold the numbers the CPU renderer's layer
// buffer would, or group opacity and the 28 blend modes cannot agree.
//
// # Nothing is sampled; everything is fetched
//
// Glyph coverage, clip masks, images and gradient ramps are read with
// `textureLoad` at integer texels. Glyphs and masks are placed at exact device
// pixels (one fragment, one texel). Images and ramps do their own bilinear /
// linear interpolation below, written out the way `native/image.rs` and
// `native/gradient.rs` write it — clamping inside the image's own patch, so
// an atlas neighbour can never bleed in, and matching the CPU renderer's
// texel-centre convention exactly.
//
// # The Y axis
//
// Authored in WebGPU clip space (+Y up); naga's SPIR-V writer flips it for
// Vulkan (`ADJUST_COORDINATE_SPACE`). `@builtin(position)` in the fragment
// stage is framebuffer pixels with a top-left origin on every backend, which
// is what the texel fetches use.

struct Uniforms {
    // .xy: the target size in device pixels. .zw unused (keeps a 16-byte vec4).
    viewport: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// R8 glyph coverage.
@group(0) @binding(1) var t_glyph: texture_2d<f32>;
// RGBA8 straight-alpha images.
@group(0) @binding(2) var t_image: texture_2d<f32>;
// R8 clip and shadow masks.
@group(0) @binding(3) var t_mask: texture_2d<f32>;
// RGBA32F premultiplied gradient ramps, 256 texels per row.
@group(0) @binding(4) var t_ramp: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) uv: vec2<f32>,
    @location(2) @interpolate(flat) kind: f32,
    @location(3) @interpolate(flat) params: vec4<f32>,
    @location(4) local: vec2<f32>,
    @location(5) @interpolate(flat) extra: vec4<f32>,
    @location(6) @interpolate(flat) mask: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) kind: f32,
    @location(4) params: vec4<f32>,
    @location(5) local: vec2<f32>,
    @location(6) extra: vec4<f32>,
    @location(7) mask: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    let normalized = position / u.viewport.xy;
    out.clip_position = vec4<f32>(normalized.x * 2.0 - 1.0, 1.0 - normalized.y * 2.0, 0.0, 1.0);
    out.color = color;
    out.uv = uv;
    out.kind = kind;
    out.params = params;
    out.local = local;
    out.extra = extra;
    out.mask = mask;
    return out;
}

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

// One premultiplied texel of the image patch `patch` (x, y, w, h), clamped
// to the patch — `native/image.rs`'s `texel` + its clamp.
fn image_texel(rect: vec4<f32>, ix: f32, iy: f32) -> vec4<f32> {
    let cx = clamp(ix, 0.0, rect.z - 1.0);
    let cy = clamp(iy, 0.0, rect.w - 1.0);
    let t = textureLoad(t_image, vec2<i32>(i32(rect.x + cx), i32(rect.y + cy)), 0);
    return premul(t);
}

// `native/image.rs`'s `bilinear`, at unit coordinate `uv`.
fn image_bilinear(rect: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    let sx = uv.x * rect.z - 0.5;
    let sy = uv.y * rect.w - 0.5;
    let x0 = floor(sx);
    let y0 = floor(sy);
    let fx = sx - x0;
    let fy = sy - y0;
    let c00 = image_texel(rect, x0, y0);
    let c10 = image_texel(rect, x0 + 1.0, y0);
    let c01 = image_texel(rect, x0, y0 + 1.0);
    let c11 = image_texel(rect, x0 + 1.0, y0 + 1.0);
    let top = c00 + (c10 - c00) * fx;
    let bottom = c01 + (c11 - c01) * fx;
    return top + (bottom - top) * fy;
}

const TAU: f32 = 6.283185307179586;

// `native/gradient.rs`'s `parameter`.
fn gradient_t(kind: f32, g: vec4<f32>, uv: vec2<f32>) -> f32 {
    if kind < 0.5 {
        let d = g.zw - g.xy;
        let len2 = dot(d, d);
        if len2 <= 1e-9 {
            return 0.0;
        }
        return dot(uv - g.xy, d) / len2;
    }
    if kind < 1.5 {
        if g.z <= 1e-6 {
            return 0.0;
        }
        return length(uv - g.xy) / g.z;
    }
    let span = g.w - g.z;
    if abs(span) <= 1e-6 {
        return 0.0;
    }
    let d = uv - g.xy;
    var angle = atan2(d.y, d.x);
    // The CPU loops `+= TAU` / `-= TAU`; the closed form lands in the same
    // half-open turn `[start, start + TAU)`.
    angle = angle - TAU * floor((angle - g.z) / TAU);
    return clamp((angle - g.z) / span, 0.0, 1.0);
}

fn ramp_at(row: f32, t: f32) -> vec4<f32> {
    let f = clamp(t, 0.0, 1.0) * 255.0;
    let i0 = min(floor(f), 254.0);
    let frac = f - i0;
    let r = i32(row);
    let a = textureLoad(t_ramp, vec2<i32>(i32(i0), r), 0);
    let b = textureLoad(t_ramp, vec2<i32>(i32(i0) + 1, r), 0);
    return a + (b - a) * frac;
}

const BAYER4 = array<f32, 16>(
    -0.46875, 0.03125, -0.34375, 0.15625,
    0.28125, -0.21875, 0.40625, -0.09375,
    -0.28125, 0.21875, -0.40625, 0.09375,
    0.46875, -0.03125, 0.34375, -0.15625,
);

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let px = vec2<i32>(floor(in.clip_position.xy));
    var out: vec4<f32>;
    if in.kind < 0.5 {
        out = premul(in.color);
    } else if in.kind < 1.5 {
        let texel = px - vec2<i32>(round(in.uv));
        let coverage = textureLoad(t_glyph, texel, 0).r;
        out = premul(in.color) * coverage;
    } else if in.kind < 2.5 {
        let uv = clamp(in.local, vec2<f32>(0.0), vec2<f32>(0.99999994));
        out = image_bilinear(in.params, uv);
        let frac = in.mask.w;
        if frac > 0.0 {
            let high = image_bilinear(in.extra, uv);
            out = out + (high - out) * frac;
        }
    } else {
        let t = gradient_t(in.extra.z, in.params, in.local);
        out = ramp_at(in.extra.x, t);
        if in.extra.y > 0.5 {
            let offset = BAYER4[((px.y & 3) * 4) + (px.x & 3)] / 255.0;
            out = clamp(out + vec4<f32>(offset), vec4<f32>(0.0), vec4<f32>(1.0));
        }
    }
    if in.mask.z > 0.5 {
        let texel = px - vec2<i32>(round(in.mask.xy));
        out = out * textureLoad(t_mask, texel, 0).r;
    }
    return out;
}
