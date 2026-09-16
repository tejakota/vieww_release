//! Layer filters: Gaussian (3-box) blur and the 5x4 colour matrix.
//!
//! Spec §8.1/§8.2: "the CPU kernels in vieww-effects are the definition of
//! correctness — the 3-box Gaussian blur_rgba, the premultiplied colour
//! matrix". Applied to a resolved layer [`super::target::Target`] in the
//! reference renderer's own layer-pop handling, in that order — blur first,
//! matrix after (spec
//! §7's `ImageFilter` docs: "blurring already-desaturated pixels and
//! desaturating already-blurred ones give the same answer for a linear
//! matrix", so the order is a convention rather than a constraint, and this
//! follows the one the doc states).

use vieww_foundation::filter::ColorMatrix;

use super::color::Premul;
use super::target::Target;

/// In-place 3-box approximation of a Gaussian blur, standard deviation
/// `sigma` device pixels, applied on premultiplied channels (blurring
/// premultiplied avoids a transparent neighbour tinting an opaque pixel,
/// which un-premultiplied blurring does not).
/// # Four channels at once, not four passes
///
/// This used to split the layer into four `f32` planes, blur each of them
/// separately, and interleave the result back — six full passes over the
/// buffer purely to move numbers around, and then twenty-four separable
/// passes over four separate allocations. It also read every sample through a
/// bounds-checked helper, including the overwhelming majority that are
/// nowhere near an edge, and its vertical pass walked *down columns* of a
/// row-major buffer, which is a cache miss per pixel.
///
/// A [`Premul`] is four floats side by side, so a running box sum over
/// `Premul` blurs all four channels together with the same number of
/// operations and a quarter of the memory traffic — no planes, no
/// deinterleave, one scratch buffer, and the vertical pass moving one whole
/// pixel at a time instead of one channel. The arithmetic is unchanged, so
/// the picture is too.
pub(crate) fn blur(target: &mut Target, sigma: f32) {
    if sigma <= 0.0 {
        return;
    }
    // See `shadow::box_blur_3`'s doc comment for why this is not `3 * sigma`.
    let diameter = (4.0 * sigma * sigma + 1.0).sqrt();
    let radius = (((diameter - 1.0) / 2.0).round() as i32).max(1);
    let (w, h) = (target.width, target.height);
    if w == 0 || h == 0 {
        return;
    }

    // **Blur the band the layer actually inked, not the whole buffer.**
    //
    // A layer's buffer is sized to its declared bounds — for a sheet or an
    // overlay that is usually the whole window — while what was drawn into it
    // may be a card in the middle. Every pixel outside the ink is transparent,
    // and a box blur of transparency is transparency, so the rows above and
    // below the ink (plus the kernel's reach, which is where the falloff lives)
    // cannot change. Running the six separable passes over them anyway is the
    // dominant cost of every blurred layer in the framework.
    //
    // Rows only, not columns: the horizontal pass walks whole rows and the
    // vertical pass walks whole columns, so narrowing the column range would
    // mean strided access for one of them and a bookkeeping mistake waiting to
    // happen. Rows are the axis where the saving is, because the buffer is
    // row-major and a band of rows is a contiguous slice.
    let reach = 3 * radius;
    let (top, bottom) = match target.ink() {
        Some((_, iy0, _, iy1)) => (
            (iy0 - reach).max(0) as u32,
            (iy1 + reach).min(h as i32).max(0) as u32,
        ),
        // Nothing was drawn: there is nothing to blur, and the whole buffer is
        // already transparent.
        None => {
            target.grow_ink(reach);
            return;
        }
    };
    if bottom <= top {
        target.grow_ink(reach);
        return;
    }
    let band = (bottom - top) as usize * w as usize;
    let start = top as usize * w as usize;
    let band_h = bottom - top;

    let rows = &mut target.pixels[start..start + band];

    // **The blur runs at full resolution, and a reduced-resolution version was
    // tried and removed.**
    //
    // Computing a wide blur on a half- or quarter-size copy and scaling the
    // result back up is what Core Animation, Skia and Chrome all do, and the
    // arithmetic is sound: past a certain width the output has no detail a
    // coarser grid cannot carry. It was implemented here in full — box
    // downsample, variance-corrected sigma, bilinear upsample — and then
    // deleted, because measuring it first would have saved the effort:
    //
    // * A callgrind profile of `examples/fixtures`' `23-editor-glass`, the
    //   most blur-heavy screen in the gallery, puts **7.5%** of the frame in
    //   this function. The other 92% is compositing and the output conversion.
    //   Sixteen-fold on 7.5% is worth 7%, and it changes what is drawn.
    // * Measured end to end, `00-blurs` moved from 8.15 ms to 7.39 ms and
    //   `23-editor-glass` from 32.21 ms to 32.46 ms — inside this machine's
    //   run-to-run spread, in both directions.
    // * The bilinear upsample called `f32::floor` twice per pixel, and on the
    //   baseline `x86-64` target that is a **call into `compiler-builtins`**
    //   rather than an instruction, which put 2.9% of the frame into `floorf`
    //   alone. The optimisation was paying most of its own winnings back.
    //
    // So: an approximation that changes output, for no measurable gain, on the
    // strength of an argument that is correct about everything except where the
    // time was going. Recorded because it is the obvious idea and someone will
    // have it again; the honest next step for blur-heavy frames is the
    // compositing loop, or a GPU.
    let mut scratch = vec![Premul::TRANSPARENT; band];
    for _ in 0..3 {
        horizontal(rows, &mut scratch, w, band_h, radius);
        vertical(&scratch, rows, w, band_h, radius);
    }
    // **A blur moves ink outwards, and the composite has to know how far.**
    //
    // Three box passes of half-width `radius` each reach `3 * radius` in every
    // direction; nothing beyond that can have become non-transparent, because
    // every sample feeding it was transparent. Declaring exactly that keeps
    // `Target::ink` an honest upper bound on what a blurred layer contains —
    // under-reporting it here would clip the soft edge off every glow and
    // every shadow in the framework, which is the one way this optimisation
    // could be visible in a picture.
    target.grow_ink(3 * radius);
}

/// A running-sum box blur along each row.
///
/// Samples outside the buffer count as transparent — the same convention the
/// old bounds-checked reader had — which is what keeps a blurred layer's edge
/// fading out instead of smearing its border pixel outwards. The window is
/// advanced by adding the entering sample and subtracting the leaving one, so
/// the cost is per pixel rather than per pixel per radius; the edges are
/// handled by *not adding* out-of-range samples rather than by testing every
/// sample for being out of range.
fn horizontal(src: &[Premul], dst: &mut [Premul], w: u32, h: u32, radius: i32) {
    // Divided, not multiplied by a precomputed reciprocal: `x / w` and
    // `x * (1/w)` differ in the last bit, and this kernel is a correctness
    // oracle before it is a fast path. The fixture gallery caught the
    // difference as six changed pixels in a blurred panel.
    let window = (2 * radius + 1) as f32;
    let width = w as i32;
    for y in 0..h as usize {
        let row = y * w as usize;
        let src_row = &src[row..row + w as usize];
        let dst_row = &mut dst[row..row + w as usize];

        let mut sum = Premul::TRANSPARENT;
        for x in 0..=radius.min(width - 1) {
            sum = add(sum, src_row[x as usize]);
        }
        for x in 0..width {
            dst_row[x as usize] = divide(sum, window);
            // `sum += entering - leaving`, as one expression per channel
            // rather than an add followed by a subtract. Floating-point
            // addition is not associative, so the two spellings can differ in
            // the last bit — and this renderer is the oracle every other
            // backend is diffed against, so "the same picture, one bit
            // different" is a worse answer than it sounds. Measured on the
            // fixture gallery: splitting them moved 446 pixels of a blurred
            // panel by exactly 1/255.
            let entering = x + radius + 1;
            let leaving = x - radius;
            let a = if entering < width {
                src_row[entering as usize]
            } else {
                Premul::TRANSPARENT
            };
            let b = if leaving >= 0 {
                src_row[leaving as usize]
            } else {
                Premul::TRANSPARENT
            };
            sum = add(sum, sub(a, b));
        }
    }
}

/// How many columns the vertical pass carries at once — see [`vertical`].
///
/// Four `Premul`s are 64 bytes, which is one cache line on every target this
/// runs on. Eight is two lines per row per strip: enough that the loads are
/// whole lines rather than quarters, small enough that eight running sums plus
/// their addresses stay in registers.
const STRIP: usize = 8;

/// The same, down each column — but eight columns at a time.
///
/// # Why a strip, and why this is not an approximation
///
/// Column-major access over a row-major buffer is the one genuinely
/// cache-hostile step in a separable blur. Walking a single column touches one
/// `Premul` — 16 bytes — out of every 64-byte line the hardware fetches, so
/// three quarters of the memory traffic of the vertical pass was fetching
/// neighbours it was about to throw away and then come back for. A blurred
/// full-window layer runs this three times.
///
/// Carrying `STRIP` adjacent columns down the buffer together uses the whole
/// line while it is resident. Each column keeps its own running sum, exactly as
/// before, and the sums never interact — so this is the same arithmetic in the
/// same order *per column*, only interleaved across columns. The output is
/// bit-for-bit what the single-column loop produced, which matters here more
/// than usual: this kernel is a correctness reference, and `horizontal`'s own
/// comment records a previous change to it that moved 446 pixels by 1/255.
///
/// # What it is worth, and a warning about how it was nearly measured
///
/// `examples/blur_bench.rs`, one full-window blurred layer at 1366x768, best of
/// twelve renders per run, six runs: **43.3 ms with the strip against 58.6 ms
/// without** — about 26%.
///
/// Those six runs are not ceremony. A single run of each said 55.4 ms for the
/// strip and 53.8 ms without it, which is the *opposite* conclusion, and it was
/// nearly acted on: the change was reverted on that number before a repeat run
/// showed the two distributions barely overlap. This machine's spread on the
/// same binary is wider than the effect being measured, so a before/after taken
/// one sample each is not evidence of anything, in either direction.
fn vertical(src: &[Premul], dst: &mut [Premul], w: u32, h: u32, radius: i32) {
    // Divided rather than multiplied by a reciprocal — see `horizontal`.
    let window = (2 * radius + 1) as f32;
    let (width, height) = (w as usize, h as i32);

    let mut x0 = 0usize;
    while x0 < width {
        let strip = STRIP.min(width - x0);
        let mut sums = [Premul::TRANSPARENT; STRIP];

        for y in 0..=radius.min(height - 1) {
            let row = y as usize * width + x0;
            for (i, sum) in sums.iter_mut().take(strip).enumerate() {
                *sum = add(*sum, src[row + i]);
            }
        }

        for y in 0..height {
            let out_row = y as usize * width + x0;
            for (i, sum) in sums.iter_mut().take(strip).enumerate() {
                dst[out_row + i] = divide(*sum, window);
            }
            // One expression per column, for the reason `horizontal` gives.
            let entering = y + radius + 1;
            let leaving = y - radius;
            for (i, sum) in sums.iter_mut().take(strip).enumerate() {
                let a = if entering < height {
                    src[entering as usize * width + x0 + i]
                } else {
                    Premul::TRANSPARENT
                };
                let b = if leaving >= 0 {
                    src[leaving as usize * width + x0 + i]
                } else {
                    Premul::TRANSPARENT
                };
                *sum = add(*sum, sub(a, b));
            }
        }
        x0 += strip;
    }
}

fn add(a: Premul, b: Premul) -> Premul {
    Premul {
        r: a.r + b.r,
        g: a.g + b.g,
        b: a.b + b.b,
        a: a.a + b.a,
    }
}

fn sub(a: Premul, b: Premul) -> Premul {
    Premul {
        r: a.r - b.r,
        g: a.g - b.g,
        b: a.b - b.b,
        a: a.a - b.a,
    }
}

fn divide(p: Premul, k: f32) -> Premul {
    Premul {
        r: p.r / k,
        g: p.g / k,
        b: p.b / k,
        a: p.a / k,
    }
}

/// Apply a 5x4 colour matrix to every pixel, in **straight** alpha (spec
/// §8.2: "applies to straight-alpha colours per the reference kernel's
/// premultiplied-to-straight round trip").
pub(crate) fn color_matrix(target: &mut Target, matrix: &ColorMatrix) {
    // A matrix has offsets in its fifth column, so a fully transparent pixel
    // can come out visible. Every pixel is therefore potentially ink.
    target.mark_all_ink();
    for p in &mut target.pixels {
        let a = p.a.max(1e-6);
        let (r, g, b, alpha) = (p.r / a, p.g / a, p.b / a, p.a);
        let apply = |row: usize| -> f32 {
            matrix[row * 5] * r
                + matrix[row * 5 + 1] * g
                + matrix[row * 5 + 2] * b
                + matrix[row * 5 + 3] * alpha
                + matrix[row * 5 + 4]
        };
        let nr = apply(0).clamp(0.0, 1.0);
        let ng = apply(1).clamp(0.0, 1.0);
        let nb = apply(2).clamp(0.0, 1.0);
        let na = apply(3).clamp(0.0, 1.0);
        *p = Premul {
            r: nr * na,
            g: ng * na,
            b: nb * na,
            a: na,
        };
    }
}
