//! Clip resolution: a rect fast path plus rasterized path clips.
//!
//! Spec §7.3: "Rect clips map to scissor — zero cost... path clips use
//! stencil-then-cover geometry". This CPU reference has no scissor or
//! stencil hardware, so both collapse to the same operation — a coverage
//! mask multiplied into whatever is drawn — but the rect case is still kept
//! cheap: an all-rect clip stack never rasterizes anything, it only narrows
//! a bounding box.

use std::sync::Arc;

use crate::Clip;
use vieww_foundation::{PathVerb, Rect, Transform};

use super::geometry::fill::{rasterize, CoverageMask};

/// The device-space grid clip masks are rasterised on.
///
/// # Why a mask is not simply the size of the clip
///
/// A clip's bounding box is the *panel's*, and the thing being clipped is
/// usually a glyph. Rasterising the whole box to multiply it into a 20x14
/// piece of text is the second shape of the same mistake the cache fixed:
/// once per command became once per clip, but each of those was still a
/// panel-sized mask. Measured on the real Studio shell at 1440x900 — a
/// 1.3-megapixel window — the scene's shape clips came to **80 megapixels**
/// of mask.
///
/// So a mask is built only over the region that asks for it, snapped out to
/// this grid. The snapping is what keeps the cache working: a hundred glyph
/// runs across one line of code all round out to the same handful of tiles
/// and share them, where an exact per-consumer region would miss every time.
/// Bigger tiles mean more wasted mask; smaller tiles mean more entries and
/// more re-rasterisation at the seams. 128 covers a line of text in one or
/// two tiles and a panel edge in a few.
const TILE: f32 = 128.0;

/// `region` grown outwards to [`TILE`] boundaries.
fn snap_out(region: Rect) -> Rect {
    Rect::new(
        (region.left / TILE).floor() * TILE,
        (region.top / TILE).floor() * TILE,
        (region.right / TILE).ceil() * TILE,
        (region.bottom / TILE).ceil() * TILE,
    )
}

/// What makes two [`Clip`]s resolve to the same mask: the same bounding box
/// and the same shapes.
///
/// # Address first, contents second — and why both are needed
///
/// The fast comparison is the *address* of the `Arc<Vec<PathVerb>>` each
/// [`vieww_foundation::Path`] shares. `Path::shared_verbs`' own doc blesses
/// exactly this, "provided the entry holds the buffer too", which is why the
/// key owns the `Arc`s. Every command recorded under one `Canvas` clip state
/// carries a *clone* of that state, so in the ordinary case the clones all
/// point at one buffer and the pointer comparison settles it.
///
/// The ordinary case is not the only case. `Command::transformed` — which the
/// compositor runs whenever it moves a command between layers — rewrites the
/// clip through `Path::transformed`, and for a genuine (non-identity)
/// transform that has to build a new buffer. Every command in the moved run
/// then holds a *different* buffer containing the *same* numbers, and a
/// pointer-only key misses on all of them: exactly the collapse this cache
/// exists to prevent, reintroduced by the one code path that rewrites clips.
///
/// So a pointer miss falls through to comparing the verbs. That is a few
/// dozen floats for a rounded rectangle — thousands of times cheaper than the
/// rasterisation it avoids, and it makes the cache correct by *value*, which
/// is the property that actually matters.
///
/// (The key also carries the *region* the mask was built over, since a clip
/// resolved over one tile is not a mask for another — see [`TILE`].)
#[derive(Debug)]
struct ClipKey {
    bounds: [u32; 4],
    shapes: Vec<Arc<Vec<PathVerb>>>,
}

impl ClipKey {
    fn new(clip: &Clip, bounds: Rect) -> Self {
        Self {
            bounds: [
                bounds.left.to_bits(),
                bounds.top.to_bits(),
                bounds.right.to_bits(),
                bounds.bottom.to_bits(),
            ],
            shapes: clip
                .shapes()
                .iter()
                .map(|shape| Arc::clone(shape.shared_verbs()))
                .collect(),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.bounds == other.bounds
            && self.shapes.len() == other.shapes.len()
            && self
                .shapes
                .iter()
                .zip(&other.shapes)
                .all(|(a, b)| Arc::ptr_eq(a, b) || a == b)
    }
}

/// Resolved clip masks, kept across the commands of one frame.
///
/// # Why this is not an optimisation
///
/// `resolve_clip_cached` rasterises a clip's shapes over the clip's whole bounding
/// box. Without this cache that happened **once per drawing command**, so a
/// panel with one rounded corner cost every glyph, rule and icon inside it a
/// full-panel rasterisation of the same rounded rectangle. Measured on a
/// 1366x679 window: 1600 unclipped commands rendered in 24 ms and the same
/// 1600 under one window-sized rounded clip took 15.7 seconds — the clip mask,
/// re-derived 1600 times, was 99.8% of the frame. That is the difference
/// between a frame and a hang, not between a fast frame and a slow one.
#[derive(Debug, Default)]
pub(crate) struct ClipCache {
    entries: Vec<(ClipKey, CoverageMask)>,
    hits: usize,
    misses: usize,
}

/// How many distinct clip masks are kept at once.
///
/// Small on purpose: a mask is a `f32` per pixel of its bounds, so a
/// window-sized one is a few megabytes. Real trees nest clips a handful deep
/// and revisit them in runs, so the working set is tiny; this is a guard
/// against a pathological scene, not a tuning knob.
///
/// Larger than it looks like it needs to be because an entry is now one
/// *tile* of one clip rather than a whole clip: a screen of clipped text
/// touches a few dozen of them, and an entry is a 128x128 `f32` mask — 64 KB
/// — so the whole cache is a handful of megabytes at worst.
const CAPACITY: usize = 96;

impl ClipCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Hits and misses over this cache's lifetime — what the perf tests
    /// assert on, so "the clip was re-rasterised per command" can regress
    /// loudly instead of quietly.
    #[cfg(test)]
    pub(crate) fn stats(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }

    fn get_or_insert(
        &mut self,
        key: ClipKey,
        build: impl FnOnce() -> CoverageMask,
    ) -> &CoverageMask {
        if let Some(index) = self.entries.iter().position(|(k, _)| k.matches(&key)) {
            self.hits += 1;
            // Most-recently-used last, so the eviction below drops the
            // coldest entry rather than an entry a loop is alternating with.
            let entry = self.entries.remove(index);
            self.entries.push(entry);
        } else {
            self.misses += 1;
            if self.entries.len() >= CAPACITY {
                self.entries.remove(0);
            }
            let mask = build();
            self.entries.push((key, mask));
        }
        &self.entries.last().expect("just pushed").1
    }
}

/// A clip resolved against [`ClipCache`], borrowing the cached mask rather
/// than copying it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClipRef<'a> {
    mask: Option<&'a CoverageMask>,
}

impl ClipRef<'_> {
    #[must_use]
    pub(crate) fn coverage_at(&self, x: i32, y: i32) -> f32 {
        match self.mask {
            Some(mask) => mask.coverage_at(x, y),
            None => 1.0,
        }
    }
}

/// Resolve `clip` far enough to cover `need`, reusing an already-rasterised
/// mask wherever one covers the same tiles.
///
/// `need` is the device-space region the caller is actually about to paint —
/// the ink, not the clip. Everything outside it is never read, so nothing
/// outside it is rasterised. See [`TILE`] for why the region is snapped
/// rather than used exactly, and [`ClipCache`] for why any of this is
/// cached at all.
///
/// The returned [`ClipRef`] carries the mask alone. Callers that need the
/// clip's *bounds* — to narrow their own rasterisation before any of this runs
/// — ask [`clip_bounds`] for them directly, which is the cheap half and is
/// available before a mask exists. This used to also carry a `bounds` field
/// saying the same thing, which nothing read.
#[must_use]
pub(crate) fn resolve_clip_cached<'a>(
    cache: &'a mut ClipCache,
    clip: &Clip,
    transform: Transform,
    surface: Rect,
    need: Rect,
) -> ClipRef<'a> {
    let bounds = clip.bounds().map_or(surface, |b| b.intersect(surface));
    if clip.shapes().is_empty() {
        return ClipRef { mask: None };
    }

    // Only the part of the clip the caller can actually reach, snapped to the
    // tile grid so neighbouring callers share one mask.
    let region = snap_out(need.intersect(bounds));
    if region.is_empty() {
        return ClipRef { mask: None };
    }

    let key = ClipKey::new(clip, region);
    let clip = clip.clone();
    let mask = cache.get_or_insert(key, move || rasterize_clip_shapes(&clip, transform, region));
    ClipRef { mask: Some(mask) }
}

/// Flatten a clip's shapes and rasterize them into a coverage mask over
/// `bounds`. The one place a clip becomes pixels.
///
/// # Nested *shaped* clips intersect; they do not union
///
/// A `Clip`'s shape list accumulates — every `add_path` pushes another
/// shape, each one the record of a *nested* clip that was in force — so the
/// semantics the list carries are “inside shape 1 **and** inside shape 2”.
/// Flattening them all into one polyline list and rasterizing that in a
/// single nonzero-winding pass computes their **union**, which leaks
/// wherever the shapes only partially overlap: a circular clip inside a
/// rounded-rect clip painted the rounded rect's corners *outside* the
/// circle. That was the behaviour for the whole life of this file, and no
/// test caught it because the exercised nesting (panel in panel) always had
/// the inner shape fully contained in the outer one, where union and
/// intersection agree.
///
/// So: one shape (or zero) rasterizes as before — the single `Path`'s own
/// subpaths union among themselves, which is correct, that is what a
/// multi-subpath path means. Two or more *separate* shapes rasterize
/// separately and multiply, which is intersection in coverage arithmetic and
/// is exact at the antialiased edge rather than only in the interior.
pub(crate) fn rasterize_clip_shapes(
    clip: &Clip,
    transform: Transform,
    bounds: Rect,
) -> CoverageMask {
    // Clip shapes are already recorded in absolute (device) space by `Scene`
    // (spec §2.1: "every command carries its transform ... already resolved
    // to absolute coordinates at record time"), so they are rasterized with
    // an identity transform here — `transform` is accepted for symmetry with
    // every other geometry entry point and is intentionally unused on this
    // path.
    let _ = transform;
    let shapes = clip.shapes();
    if shapes.len() <= 1 {
        let mut polylines = Vec::new();
        for shape in shapes {
            polylines.extend(super::geometry::flatten::flatten_path(
                shape,
                Transform::IDENTITY,
            ));
        }
        return rasterize(&polylines, bounds);
    }

    let mut accumulated: Option<CoverageMask> = None;
    for shape in shapes {
        let polylines = super::geometry::flatten::flatten_path(shape, Transform::IDENTITY);
        let mask = rasterize(&polylines, bounds);
        accumulated = Some(match accumulated {
            None => mask,
            Some(existing) => intersect_masks(existing, &mask),
        });
    }
    accumulated.unwrap_or_else(CoverageMask::empty)
}

/// Two coverage masks, multiplied — the coverage arithmetic for “inside
/// both”.
///
/// `rasterize` returns a mask whose extent is the *ink* it covered, not the
/// region it was asked to cover, so two shapes of one clip produce two masks
/// with different origins and extents. Rather than re-anchoring either, the
/// accumulator walks its own pixels and asks the other mask for coverage *at
/// that pixel's absolute position* — `coverage_at` answers zero outside the
/// other mask, which is exactly the intersection semantics.
///
/// The product of two antialiased coverages is the standard approximation for
/// intersected soft edges (the same one stencil-then-cover would produce for
/// two clip layers), and it errs on the conservative side — slightly darker
/// edge than the exact-area answer, never ink outside either shape.
fn intersect_masks(mut into: CoverageMask, by: &CoverageMask) -> CoverageMask {
    let stride = into.width as usize;
    for (index, accumulator) in into.data.iter_mut().enumerate() {
        let x = into.x0 + (index % stride) as i32;
        let y = into.y0 + (index / stride) as i32;
        *accumulator *= by.coverage_at(x, y);
    }
    into
}

/// The rectangle a clip lets through, before any shape is rasterised.
///
/// Callers need this *first* — to know how much of their own geometry is
/// worth rasterising at all — and only then know which region of the clip
/// they will read, which is what [`resolve_clip_cached`] wants. Splitting the
/// cheap half out is what makes that order possible.
#[must_use]
pub(crate) fn clip_bounds(clip: &Clip, surface: Rect) -> Rect {
    clip.bounds().map_or(surface, |b| b.intersect(surface))
}

impl ClipRef<'_> {
    /// Whether this clip actually carries a coverage mask.
    ///
    /// A clip that does not — no clip at all, or a plain rectangle, which
    /// callers have already confined their own geometry to — leaves every
    /// pixel at full coverage, so a caller can skip the whole
    /// multiply-by-coverage pass instead of multiplying by one.
    #[must_use]
    pub(crate) fn has_mask(&self) -> bool {
        self.mask.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Canvas, Scene};
    use vieww_foundation::{Color, Path};

    /// Two nested *shaped* clips that only partially overlap must intersect,
    /// not union — the leak this file's own module doc records.
    ///
    /// A square clip and a circular clip over the same square fill, each
    /// covering only part of the other: the union answer inks the square's
    /// corners outside the circle, the intersection answer does not. The
    /// pixel checked is deep in a corner the circle never covers, far from
    /// any antialiased edge, so the assertion needs no tolerance.
    #[test]
    fn partially_overlapping_shaped_clips_intersect_not_union() {
        let mut scene = Scene::new();
        scene.save();
        scene.clip_path(&Path::rect(Rect::new(0.0, 0.0, 100.0, 100.0)));
        // A circle centred at the square's centre, radius 25: covers the
        // middle, touches no corner.
        let mut circle = Path::new();
        circle.move_to(vieww_foundation::Offset::new(50.0, 25.0));
        for step in 0..16 {
            let angle = step as f32 * std::f32::consts::FRAC_PI_8;
            let point =
                vieww_foundation::Offset::new(50.0 + 25.0 * angle.cos(), 50.0 + 25.0 * angle.sin());
            circle.line_to(point);
        }
        circle.close();
        scene.clip_path(&circle);
        scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED.into());
        scene.restore();

        let mut renderer = super::super::reference::NativeRenderer::new();
        let (pixels, _) = renderer
            .render_to_pixels(&scene, 100, 100, Color::WHITE)
            .expect("render");
        // A corner of the square, well outside the circle: white under
        // intersection, red under the union this test exists to keep dead.
        assert_eq!(
            pixels.pixel(5, 5),
            Color::WHITE,
            "a square clip intersected with a circle clip must not ink the \
             square's corners outside the circle"
        );
        // And the centre, inside both, is red — proving the intersection did
        // not eat the whole clip.
        assert_eq!(
            pixels.pixel(50, 50),
            Color::RED,
            "inside both clips, the fill still lands"
        );
    }
}
