// Separable Gaussian blur, one pass. Two draws — one with `direction =
// (1, 0)`, one with `direction = (0, 1)` on the first pass's output — turn
// this into a full 2D blur at O(n) samples instead of O(n²), the standard
// technique behind every GPU blur (and the one `vieww-effects`' CPU
// `BackdropBlur` approximates with a box blur for speed; this is the real
// separable-Gaussian version a GPU can afford).
//
// `sigma` is in texels; the loop's radius is derived from it (`3 * sigma`
// captures >99% of a Gaussian's mass), capped at `MAX_RADIUS` so a runaway
// uniform can't turn one draw into thousands of texture fetches.

const MAX_RADIUS: i32 = 32;
const PI: f32 = 3.14159265359;

struct Uniforms {
    // .xy: blur direction as a texel-space unit vector, e.g. (1, 0) for the
    // horizontal pass; .z: sigma in texels; .w unused.
    direction_sigma: vec4<f32>,
    // .xy: source texture size in texels, used to convert `direction` (a
    // unit vector) into a per-tap UV offset.
    texture_size: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;
@group(0) @binding(1)
var t_source: texture_2d<f32>;
@group(0) @binding(2)
var s_source: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    // Blur passes draw a full-viewport triangle in already-normalized
    // device coordinates — there is no viewport-relative geometry to place,
    // unlike `solid`/`gradient`/`image`, so `position` is passed straight
    // through as clip space.
    var out: VertexOutput;
    out.clip_position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn gaussian_weight(x: f32, sigma: f32) -> f32 {
    let sigma_sq = max(sigma * sigma, 0.0001);
    return exp(-(x * x) / (2.0 * sigma_sq)) / sqrt(2.0 * PI * sigma_sq);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let sigma = max(u.direction_sigma.z, 0.0001);
    let radius = min(i32(ceil(sigma * 3.0)), MAX_RADIUS);
    let texel = u.direction_sigma.xy / u.texture_size.xy;

    var accumulated = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var weight_sum = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let weight = gaussian_weight(f32(i), sigma);
        let sample_uv = in.uv + texel * f32(i);
        accumulated += textureSample(t_source, s_source, sample_uv) * weight;
        weight_sum += weight;
    }
    return accumulated / max(weight_sum, 0.0001);
}
