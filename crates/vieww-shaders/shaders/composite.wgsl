// Layer compositing: blend a source texture over a destination texture
// using one of a handful of `vieww_foundation::BlendMode`'s separable modes,
// selected at draw time by an integer uniform rather than by shader
// variant — the same "data, not a shader per mode" choice `gradient.wgsl`
// makes for its two colours.
//
// # Scope: which `BlendMode` variants this covers, and why not all of them
//
// `Normal` (Porter-Duff source-over) plus five separable modes — `Multiply`,
// `Screen`, `Darken`, `Lighten`, `Difference` — are implemented below. The
// rest of `BlendMode` (the remaining Porter-Duff operators, the harder
// separable curves like `Overlay`/`ColorDodge`/`ColorBurn`/`SoftLight`, and
// the non-separable HSL family `Hue`/`Saturation`/`Color`/`Luminosity`) is
// not: each is a mechanical addition of one more `blend_mode` branch
// following this exact pattern, not a new architectural piece, so adding
// them is follow-on work rather than something this shader's structure
// needs to change to support — tracked in `TRACKER.md`.
//
// # Simplification: opaque-backdrop compositing
//
// The exact W3C/PDF compositing formula accounts for the destination's own
// alpha (a blend mode over a *transparent* backdrop is not the same as over
// an opaque one). This shader uses the common real-time approximation —
// `mix(destination, blend(destination, source), source.alpha)` — which is
// exact when the destination is opaque (the overwhelmingly common case for
// UI compositing: blending onto an already-opaque layer beneath it) and a
// close approximation otherwise. Getting the fully general formula right
// for a semi-transparent destination is real, separate work, not a subset
// of this shader's scope.

struct Uniforms {
    // 0 = Normal, 1 = Multiply, 2 = Screen, 3 = Darken, 4 = Lighten,
    // 5 = Difference. Stored as f32 (not a WGSL integer uniform) purely so
    // this struct's layout stays one plain `vec4<f32>`-aligned block, like
    // every other shader in this library — converted with `i32()` below.
    mode: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;
@group(0) @binding(1)
var t_dst: texture_2d<f32>;
@group(0) @binding(2)
var s_dst: sampler;
@group(0) @binding(3)
var t_src: texture_2d<f32>;
@group(0) @binding(4)
var s_src: sampler;

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

fn blend_channels(dst: vec3<f32>, src: vec3<f32>, mode: i32) -> vec3<f32> {
    switch mode {
        case 1: {
            return dst * src;
        }
        case 2: {
            return vec3<f32>(1.0, 1.0, 1.0) - (vec3<f32>(1.0, 1.0, 1.0) - dst) * (vec3<f32>(1.0, 1.0, 1.0) - src);
        }
        case 3: {
            return min(dst, src);
        }
        case 4: {
            return max(dst, src);
        }
        case 5: {
            return abs(dst - src);
        }
        default: {
            return src;
        }
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dst = textureSample(t_dst, s_dst, in.uv);
    let src = textureSample(t_src, s_src, in.uv);
    let mode = i32(u.mode.x);
    let blended_rgb = blend_channels(dst.rgb, src.rgb, mode);
    let out_rgb = mix(dst.rgb, blended_rgb, src.a);
    let out_a = src.a + dst.a * (1.0 - src.a);
    return vec4<f32>(out_rgb, out_a);
}
