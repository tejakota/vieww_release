// Offscreen compositing: every per-pixel operation a `vieww_gpu::Step` needs
// that is not geometry.
//
// One full-screen triangle, scissored by the backend to the region the step
// touches, and one fragment program selected by `pc.args.x`. Each mode is a
// line-for-line port of the `NativeRenderer` function named beside it, so the
// GPU/CPU parity suite compares two executions of one definition:
//
//   0 COPY     src                         Target::snapshot_from
//   1 BOX_H    horizontal box, radius r    shadow::box_blur_horizontal / effects::blur
//   2 BOX_V    vertical box, radius r      … _vertical
//   3 MATRIX   4×5 colour matrix           effects::color_matrix
//   4 MASK     src × mask                  PopLayer's shaped-clip multiply
//   5 BLEND    blend(mode, src×α, dst)     Target::composite_layer, 28 modes
//   6 OVER     src × α (pipeline blends)   composite_layer, Normal
//   7 SEED     mask coverage               shadow caster rasterisation
//   8 INSET    (1 − src.a) × mask          render_shadow's complement
//   9 TINT     colour × src.a [× clip]     render_shadow's tint + clip multiply
//
// Targets are premultiplied RGBA16F, fetched with `textureLoad`. Pixels
// outside `pc.region` read as transparent for the blur passes — "outside the
// buffer is transparent", which is what makes a shadow fade at its own edge.

struct Post {
    // x0, y0, x1, y1 in device pixels.
    region: vec4<f32>,
    // mode, radius, blend mode, alpha.
    args: vec4<f32>,
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
    p4: vec4<f32>,
    p5: vec4<f32>,
};

var<immediate> pc: Post;

@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var t_aux: texture_2d<f32>;
@group(0) @binding(2) var t_mask: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // A triangle covering the whole target; the scissor narrows it.
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn src_at(p: vec2<i32>) -> vec4<f32> {
    if f32(p.x) < pc.region.x || f32(p.y) < pc.region.y || f32(p.x) >= pc.region.z || f32(p.y) >= pc.region.w {
        return vec4<f32>(0.0);
    }
    return textureLoad(t_src, p, 0);
}

fn mask_at(p: vec2<i32>, offset: vec2<f32>) -> f32 {
    return textureLoad(t_mask, p - vec2<i32>(round(offset)), 0).r;
}

fn unpremul(c: vec4<f32>) -> vec3<f32> {
    if c.a <= 1e-6 {
        return vec3<f32>(0.0);
    }
    return c.rgb / c.a;
}

fn over(s: vec4<f32>, d: vec4<f32>) -> vec4<f32> {
    return s + d * (1.0 - s.a);
}

fn hard_light(a: f32, b: f32) -> f32 {
    if a <= 0.5 {
        return 2.0 * a * b;
    }
    return 1.0 - 2.0 * (1.0 - a) * (1.0 - b);
}

fn soft_light(cs: f32, cb: f32) -> f32 {
    if cs <= 0.5 {
        return cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb);
    }
    var d: f32;
    if cb <= 0.25 {
        d = ((16.0 * cb - 12.0) * cb + 4.0) * cb;
    } else {
        d = sqrt(cb);
    }
    return cb + (2.0 * cs - 1.0) * (d - cb);
}

fn separable_channel(mode: i32, cs: f32, cb: f32) -> f32 {
    switch mode {
        case 13: { return cs * cb; }
        case 14: { return cs + cb - cs * cb; }
        case 15: { return hard_light(cb, cs); }
        case 16: { return min(cs, cb); }
        case 17: { return max(cs, cb); }
        case 18: {
            if cb == 0.0 { return 0.0; }
            if cs >= 1.0 { return 1.0; }
            return min(cb / (1.0 - cs), 1.0);
        }
        case 19: {
            if cb >= 1.0 { return 1.0; }
            if cs <= 0.0 { return 0.0; }
            return 1.0 - min((1.0 - cb) / cs, 1.0);
        }
        case 20: { return hard_light(cs, cb); }
        case 21: { return soft_light(cs, cb); }
        case 22: { return abs(cs - cb); }
        case 23: { return cs + cb - 2.0 * cs * cb; }
        default: { return cs; }
    }
}

fn lum(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn clip_color(c_in: vec3<f32>) -> vec3<f32> {
    var c = c_in;
    let l = lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    if n < 0.0 {
        c = vec3<f32>(l) + (c - vec3<f32>(l)) * l / max(l - n, 1e-6);
    }
    if x > 1.0 {
        c = vec3<f32>(l) + (c - vec3<f32>(l)) * (1.0 - l) / max(x - l, 1e-6);
    }
    return c;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let d = l - lum(c);
    return clip_color(c + vec3<f32>(d));
}

fn sat(c: vec3<f32>) -> f32 {
    return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    // A stable sort of three indices by value, as `native/color.rs` sorts.
    var i0 = 0;
    var i1 = 1;
    var i2 = 2;
    if c[i1] < c[i0] { let t = i0; i0 = i1; i1 = t; }
    if c[i2] < c[i1] {
        let t = i1; i1 = i2; i2 = t;
        if c[i1] < c[i0] { let u = i0; i0 = i1; i1 = u; }
    }
    var out = vec3<f32>(0.0);
    if c[i2] > c[i0] {
        out[i1] = (c[i1] - c[i0]) * s / (c[i2] - c[i0]);
        out[i2] = s;
    }
    out[i0] = 0.0;
    return out;
}

// `native/color.rs`'s `blend`, on premultiplied colours.
fn blend(mode: i32, s: vec4<f32>, d: vec4<f32>) -> vec4<f32> {
    switch mode {
        case 0: { return over(s, d); }
        case 1: { return vec4<f32>(0.0); }
        case 2: { return s; }
        case 3: { return d; }
        case 4: { return over(d, s); }
        case 5: { return s * d.a; }
        case 6: { return d * s.a; }
        case 7: { return s * (1.0 - d.a); }
        case 8: { return d * (1.0 - s.a); }
        case 9: { return s * d.a + d * (1.0 - s.a); }
        case 10: { return s * (1.0 - d.a) + d * s.a; }
        case 11: { return s * (1.0 - d.a) + d * (1.0 - s.a); }
        case 12: { return min(s + d, vec4<f32>(1.0)); }
        default: {}
    }
    let cs = unpremul(s);
    let cb = unpremul(d);
    var blended: vec3<f32>;
    if mode <= 23 {
        blended = vec3<f32>(
            separable_channel(mode, cs.r, cb.r),
            separable_channel(mode, cs.g, cb.g),
            separable_channel(mode, cs.b, cb.b),
        );
    } else {
        switch mode {
            case 24: { blended = set_lum(set_sat(cs, sat(cb)), lum(cb)); }
            case 25: { blended = set_lum(set_sat(cb, sat(cs)), lum(cb)); }
            case 26: { blended = set_lum(cs, lum(cb)); }
            case 27: { blended = set_lum(cb, lum(cs)); }
            default: { blended = cs; }
        }
    }
    let mixed = (1.0 - d.a) * cs + d.a * blended;
    return over(vec4<f32>(mixed * s.a, s.a), d);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(frag.xy));
    let mode = i32(round(pc.args.x));
    switch mode {
        case 0: {
            return src_at(p);
        }
        case 1, 2: {
            let r = i32(round(pc.args.y));
            var step = vec2<i32>(1, 0);
            if mode == 2 {
                step = vec2<i32>(0, 1);
            }
            var sum = vec4<f32>(0.0);
            for (var k = -r; k <= r; k = k + 1) {
                sum = sum + src_at(p + step * k);
            }
            return sum / f32(2 * r + 1);
        }
        case 3: {
            let c = src_at(p);
            let a = max(c.a, 1e-6);
            let v = vec4<f32>(c.rgb / a, c.a);
            let nr = clamp(dot(pc.p0, v) + pc.p4.x, 0.0, 1.0);
            let ng = clamp(dot(pc.p1, v) + pc.p4.y, 0.0, 1.0);
            let nb = clamp(dot(pc.p2, v) + pc.p4.z, 0.0, 1.0);
            let na = clamp(dot(pc.p3, v) + pc.p4.w, 0.0, 1.0);
            return vec4<f32>(nr * na, ng * na, nb * na, na);
        }
        case 4: {
            return src_at(p) * mask_at(p, pc.p0.xy);
        }
        case 5: {
            let s = src_at(p) * pc.args.w;
            let d = textureLoad(t_aux, p, 0);
            return blend(i32(round(pc.args.z)), s, d);
        }
        case 6: {
            return src_at(p) * pc.args.w;
        }
        case 7: {
            return vec4<f32>(mask_at(p, pc.p0.xy));
        }
        case 8: {
            let blurred = src_at(p).a;
            return vec4<f32>(clamp(1.0 - blurred, 0.0, 1.0) * mask_at(p, pc.p0.xy));
        }
        case 9: {
            var a = src_at(p).a;
            if pc.p1.z > 0.5 {
                a = a * mask_at(p, pc.p1.xy);
            }
            return pc.p2 * a;
        }
        default: {
            return vec4<f32>(0.0);
        }
    }
}
