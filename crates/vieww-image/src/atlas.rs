//! Texture atlas packing: laying out a batch of small images inside one big
//! one, and actually compositing the pixels there.
//!
//! # Shelf packing, not a general bin packer
//!
//! [`AtlasPacker::pack`] uses *shelf packing*: sort the requested rectangles
//! tallest-first, then lay them left to right along a "shelf" as wide as the
//! atlas, starting a new shelf below the previous one's tallest rectangle
//! whenever the current one runs out of width. This is a well-known,
//! decades-old heuristic (the same idea behind `stb_rect_pack`'s simplest
//! mode and most game-engine sprite packers) and it is **not** optimal — the
//! general rectangle bin-packing problem is NP-hard, and a shelf packer can
//! waste space below a shelf's tallest item when the rest of that shelf is
//! much shorter. It is adequate for what this crate packs: UI icon sets and
//! sprite sheets, where items are small relative to the atlas and packing
//! quality only has to be "does everything fit with reasonable waste", not
//! "provably minimal area". A guillotine or skyline packer would use less
//! wasted space; it would also be meaningfully more code for a gain that
//! does not show up until an atlas is packed close to capacity, which is
//! exactly when a caller should be reaching for a bigger atlas or fewer
//! sprites per page instead.
//!
//! # Reporting failure instead of guessing
//!
//! A request that cannot fit — because it is individually larger than the
//! atlas, or because everything that fit before it left no room — is
//! reported as `None` in [`PackResult::placements`] at its own index, never
//! silently dropped and never panicked on. Every other request is still
//! placed. A caller that gets any `None` back knows exactly which sprite(s)
//! need a second atlas page or a smaller source image, without losing the
//! placements it already has.

use vieww_foundation::Image;

/// Where one packed rectangle landed, in atlas pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The outcome of packing a batch of `(width, height)` requests.
#[derive(Debug, Clone)]
pub struct PackResult {
    /// One entry per input request, in the same order as given to
    /// [`AtlasPacker::pack`]. `None` marks a request that did not fit.
    pub placements: Vec<Option<AtlasRect>>,
}

impl PackResult {
    /// `true` if every request found a place.
    #[must_use]
    pub fn all_fit(&self) -> bool {
        self.placements.iter().all(Option::is_some)
    }

    /// How many requests did not fit.
    #[must_use]
    pub fn unfit_count(&self) -> usize {
        self.placements.iter().filter(|p| p.is_none()).count()
    }
}

/// A fixed-size atlas target, and the shelf-packing layout for it.
///
/// See the module docs for why shelf packing and not something fancier.
#[derive(Debug, Clone, Copy)]
pub struct AtlasPacker {
    width: u32,
    height: u32,
}

impl AtlasPacker {
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Lay out `requests` — one `(width, height)` per sprite, in the order a
    /// caller wants them addressable — inside this atlas.
    ///
    /// Requests are tried tallest-first (the standard shelf-packing
    /// heuristic: tall items define a shelf's height, so placing them first
    /// keeps later, shorter items from being stuck under a shelf far taller
    /// than they need). Ties keep their original relative order. The
    /// returned [`PackResult::placements`] is nonetheless in the *original*
    /// request order, not the sorted one, so index `i` of the result always
    /// answers for index `i` of `requests`.
    #[must_use]
    pub fn pack(&self, requests: &[(u32, u32)]) -> PackResult {
        let mut order: Vec<usize> = (0..requests.len()).collect();
        // Stable sort: equal heights keep their original relative order.
        order.sort_by(|&a, &b| requests[b].1.cmp(&requests[a].1));

        let mut placements = vec![None; requests.len()];
        let mut shelf_y: u32 = 0;
        let mut shelf_height: u32 = 0;
        let mut cursor_x: u32 = 0;

        for index in order {
            let (w, h) = requests[index];
            if w > self.width || h > self.height {
                // Can never fit this atlas, at any position.
                continue;
            }
            if cursor_x + w > self.width {
                // Out of room on this shelf: start a new one below it.
                shelf_y += shelf_height;
                cursor_x = 0;
                shelf_height = 0;
            }
            if shelf_y + h > self.height {
                // Out of room in the atlas entirely.
                continue;
            }
            placements[index] = Some(AtlasRect {
                x: cursor_x,
                y: shelf_y,
                width: w,
                height: h,
            });
            cursor_x += w;
            shelf_height = shelf_height.max(h);
        }

        PackResult { placements }
    }
}

/// Copy each source image's pixels into an atlas-sized buffer at its rect.
///
/// # Fill policy
///
/// Every texel not covered by a placement is fully transparent
/// (`0, 0, 0, 0`) straight-alpha — the same "nothing here" value
/// [`vieww_foundation::Color::TRANSPARENT`] uses, and the safe one for a
/// texture that will be sampled with filtering: a transparent gap composites
/// as nothing rather than as a stray opaque colour bleeding in at a sprite's
/// edge under bilinear sampling.
///
/// # Panics
///
/// If a placement's rectangle does not exactly match its image's dimensions,
/// or falls outside the `width` x `height` output — both are programmer
/// errors (an [`AtlasRect`] not actually produced by [`AtlasPacker::pack`]
/// for that image), not something a caller should need a `Result` to
/// recover from.
#[must_use]
pub fn composite(width: u32, height: u32, placements: &[(&Image, AtlasRect)]) -> Image {
    let mut buffer = vec![0u8; width as usize * height as usize * 4];
    for (image, rect) in placements {
        assert_eq!(
            image.width(),
            rect.width,
            "placement width does not match the image it names"
        );
        assert_eq!(
            image.height(),
            rect.height,
            "placement height does not match the image it names"
        );
        assert!(
            rect.x + rect.width <= width && rect.y + rect.height <= height,
            "placement ({rect:?}) falls outside a {width}x{height} atlas"
        );

        let src = image.pixels();
        for row in 0..rect.height {
            let src_start = (row * rect.width * 4) as usize;
            let src_end = src_start + (rect.width * 4) as usize;
            let dest_row = rect.y + row;
            let dest_start = ((dest_row * width + rect.x) * 4) as usize;
            let dest_end = dest_start + (rect.width * 4) as usize;
            buffer[dest_start..dest_end].copy_from_slice(&src[src_start..src_end]);
        }
    }
    Image::from_rgba8(buffer, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> Image {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.copy_from_slice(&color);
        }
        Image::from_rgba8(pixels, width, height)
    }

    #[test]
    fn taller_requests_are_placed_first_and_shorter_ones_share_their_shelf() {
        let packer = AtlasPacker::new(8, 8);
        // Index 0 is shorter than index 1: sorted tallest-first, index 1
        // goes at the shelf origin and index 0 shares its shelf beside it.
        let result = packer.pack(&[(2, 2), (3, 4)]);
        assert_eq!(
            result.placements,
            vec![
                Some(AtlasRect {
                    x: 3,
                    y: 0,
                    width: 2,
                    height: 2
                }),
                Some(AtlasRect {
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 4
                }),
            ]
        );
    }

    #[test]
    fn equal_heights_keep_their_original_relative_order() {
        let packer = AtlasPacker::new(8, 8);
        let result = packer.pack(&[(2, 2), (2, 2)]);
        assert_eq!(
            result.placements,
            vec![
                Some(AtlasRect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2
                }),
                Some(AtlasRect {
                    x: 2,
                    y: 0,
                    width: 2,
                    height: 2
                }),
            ]
        );
    }

    #[test]
    fn a_full_shelf_wraps_to_a_new_row_below_the_tallest_item_so_far() {
        let packer = AtlasPacker::new(4, 8);
        // Two 3x2s cannot share a width-4 shelf (3+3=6 > 4): the second
        // wraps to y=2, directly below the first.
        let result = packer.pack(&[(3, 2), (3, 2)]);
        assert_eq!(
            result.placements,
            vec![
                Some(AtlasRect {
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 2
                }),
                Some(AtlasRect {
                    x: 0,
                    y: 2,
                    width: 3,
                    height: 2
                }),
            ]
        );
    }

    #[test]
    fn a_request_larger_than_the_whole_atlas_reports_none_rather_than_panicking() {
        let packer = AtlasPacker::new(4, 4);
        let result = packer.pack(&[(5, 5), (2, 2)]);
        assert_eq!(
            result.placements[0], None,
            "too big for the atlas at any position"
        );
        assert!(
            result.placements[1].is_some(),
            "the request that does fit is unaffected"
        );
        assert!(!result.all_fit());
        assert_eq!(result.unfit_count(), 1);
    }

    #[test]
    fn running_out_of_vertical_room_across_shelves_reports_none_too() {
        // Atlas is 3 wide, 7 tall: two 3x3 shelves fit (6 <= 7), a third
        // would need y=6..9, which overflows the 7-tall atlas.
        let packer = AtlasPacker::new(3, 7);
        let result = packer.pack(&[(3, 3), (3, 3), (3, 3)]);
        assert_eq!(result.unfit_count(), 1);
        assert!(result.placements.iter().filter(|p| p.is_some()).count() == 2);
    }

    #[test]
    fn packing_zero_requests_produces_an_empty_result() {
        let result = AtlasPacker::new(16, 16).pack(&[]);
        assert!(result.placements.is_empty());
        assert!(result.all_fit(), "vacuously true — nothing failed to fit");
    }

    #[test]
    fn compositing_copies_exact_pixels_to_exact_offsets_and_leaves_the_rest_transparent() {
        // A 2x2 red block and a 1x2 blue strip, packed into a 4x2 atlas with
        // one leftover column.
        let red = solid(2, 2, [255, 0, 0, 255]);
        let blue = solid(1, 2, [0, 0, 255, 255]);
        let packer = AtlasPacker::new(4, 2);
        let result = packer.pack(&[(2, 2), (1, 2)]);
        let rect_a = result.placements[0].expect("fits");
        let rect_b = result.placements[1].expect("fits");
        assert_eq!(
            rect_a,
            AtlasRect {
                x: 0,
                y: 0,
                width: 2,
                height: 2
            }
        );
        assert_eq!(
            rect_b,
            AtlasRect {
                x: 2,
                y: 0,
                width: 1,
                height: 2
            }
        );

        let atlas = composite(4, 2, &[(&red, rect_a), (&blue, rect_b)]);
        let pixel = |x: u32, y: u32| {
            let i = ((y * 4 + x) * 4) as usize;
            [
                atlas.pixels()[i],
                atlas.pixels()[i + 1],
                atlas.pixels()[i + 2],
                atlas.pixels()[i + 3],
            ]
        };

        assert_eq!(pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(1, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(0, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(2, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(2, 1), [0, 0, 255, 255]);
        assert_eq!(
            pixel(3, 0),
            [0, 0, 0, 0],
            "leftover column is transparent, not red or blue"
        );
        assert_eq!(pixel(3, 1), [0, 0, 0, 0]);
    }

    #[test]
    #[should_panic(expected = "does not match")]
    fn compositing_a_mismatched_rect_panics_rather_than_corrupting_the_atlas() {
        let red = solid(2, 2, [255, 0, 0, 255]);
        let wrong_rect = AtlasRect {
            x: 0,
            y: 0,
            width: 3,
            height: 3,
        };
        let _ = composite(4, 4, &[(&red, wrong_rect)]);
    }
}
