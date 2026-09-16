use std::sync::Arc;

use crate::{Alignment, Rect, Size};

/// Decoded RGBA8 pixels, shared cheaply.
///
/// Lives here rather than in `vieww-paint` for the reason [`Path`](crate::Path)
/// does: a widget has to be able to name the thing it is describing, and
/// `vieww-widget` sits below the paint layer. `vieww-paint` re-exports it, so a
/// backend never has to know which crate the pixels came from.
///
/// # The buffer is an `Arc`, not an `Rc`
///
/// Everything else in this framework that shares cheaply uses `Rc`, because the
/// widget tree is single-threaded by construction and an atomic refcount would
/// be paying for a guarantee nothing needs. Pixels are the exception, and the
/// reason is where they come from: **an image is decoded on a worker thread**.
/// Every real application does this — a JPEG is milliseconds of work and the UI
/// thread has sixteen — and the decoded buffer then has to cross back.
///
/// With an `Rc` it cannot. `Rc<Vec<u8>>` is `!Send`, so the buffer has to be
/// handed over as a bare `Vec` and wrapped on arrival, and an application that
/// keeps its own copy in a model then pays a full memcpy per clone. One did:
/// 288x288 RGBA is 331 KB, a hundred of them cloned per frame during build is
/// 33 MB of `memcpy` before anything is drawn — and, worse, a freshly wrapped
/// buffer is a *different* `Arc`, so the identity comparison below reported
/// every image as changed and defeated damage tracking entirely.
///
/// An `Arc` costs one atomic increment per clone and lets
/// [`from_shared_rgba8`](Self::from_shared_rgba8) take the decoder's buffer
/// directly. That is the right trade for the one type in the tree that is
/// routinely produced somewhere else.
#[derive(Debug, Clone)]
pub struct Image {
    pixels: Arc<Vec<u8>>,
    width: u32,
    height: u32,
}

/// Equality is by *identity*, not by content.
///
/// Damage tracking compares last frame's commands against this frame's, so this
/// runs on every image on screen every frame. Comparing pixel buffers would make
/// that cost proportional to the number of pixels on screen, which defeats the
/// point of tracking damage at all. Cloning an `Image` shares the `Rc`, so the
/// common case — the same image surviving a rebuild — still compares equal.
///
/// Two separately decoded copies of identical pixels compare *unequal* and so
/// repaint needlessly. That is the safe direction to be wrong in — but it is
/// also why [`from_shared_rgba8`](Image::from_shared_rgba8) exists: an
/// application that rebuilds its `Image` from a `Vec` every frame gets a new
/// allocation every frame, compares unequal every frame, and repaints the whole
/// screen every frame while looking entirely reasonable at the call site.
impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}

impl Eq for Image {}

impl Image {
    /// Wrap tightly packed, non-premultiplied RGBA8 pixels.
    ///
    /// # Panics
    ///
    /// If `pixels` is not exactly `width * height * 4` bytes.
    #[must_use]
    pub fn from_rgba8(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        assert_eq!(
            pixels.len(),
            (width as usize) * (height as usize) * 4,
            "image data does not match {width}x{height} RGBA8"
        );
        Self {
            pixels: Arc::new(pixels),
            width,
            height,
        }
    }

    /// Wrap pixels somebody else already owns, without copying them.
    ///
    /// For the decode-on-a-worker-thread case: the buffer is produced off the
    /// UI thread, handed over as an `Arc`, and kept by the application as well
    /// as by the tree. Both hold the same allocation, so a rebuild is a
    /// refcount bump and the identity comparison in [`PartialEq`] keeps
    /// reporting the image as unchanged.
    ///
    /// [`from_rgba8`](Self::from_rgba8) is still the right call when the
    /// pixels are produced and forgotten in one place.
    ///
    /// # Panics
    ///
    /// If `pixels` is not exactly `width * height * 4` bytes.
    #[must_use]
    pub fn from_shared_rgba8(pixels: Arc<Vec<u8>>, width: u32, height: u32) -> Self {
        assert_eq!(
            pixels.len(),
            (width as usize) * (height as usize) * 4,
            "image data does not match {width}x{height} RGBA8"
        );
        Self {
            pixels,
            width,
            height,
        }
    }

    /// The buffer itself, for handing the same allocation somewhere else.
    #[must_use]
    pub fn shared(&self) -> &Arc<Vec<u8>> {
        &self.pixels
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The natural size, in logical pixels at scale 1.
    #[must_use]
    pub fn size(&self) -> Size {
        Size::new(self.width as f32, self.height as f32)
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// How a source of one shape is fitted into a box of another.
///
/// The standard `BoxFit` set, same names and same meanings, because the vocabulary is
/// the one every designer already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxFit {
    /// Stretch to fill the box exactly, distorting the aspect ratio.
    Fill,
    /// As large as possible while fitting entirely inside. Letterboxes.
    #[default]
    Contain,
    /// As small as possible while covering the whole box. Crops.
    Cover,
    /// Natural size, neither scaled up nor down.
    None,
    /// Like [`Contain`](BoxFit::Contain), but never scales *up*.
    ///
    /// The one to reach for with thumbnails: a small image stays crisp at its
    /// own size instead of being blown up to fill a box it was never made for.
    ScaleDown,
}

impl BoxFit {
    /// Where to draw a `source`-sized thing inside `bounds`.
    ///
    /// The returned rect covers the *whole* source; for [`Cover`](BoxFit::Cover)
    /// — and for [`None`](BoxFit::None) with an oversized source — it extends
    /// past `bounds`, and the caller is expected to clip. Returning the true
    /// rect and letting the caller clip is what keeps this pure geometry:
    /// cropping here would mean also returning which part of the source
    /// survived, and `Canvas::draw_image` has nowhere to put that.
    #[must_use]
    pub fn apply(self, source: Size, bounds: Rect, alignment: Alignment) -> Rect {
        let container = bounds.size();
        // A source with no area cannot be scaled into anything: every ratio
        // below would be a division by zero, and the honest answer is that
        // there is nothing to draw.
        if source.width <= 0.0 || source.height <= 0.0 {
            return Rect::new(bounds.left, bounds.top, bounds.left, bounds.top);
        }

        let wide = container.width / source.width;
        let tall = container.height / source.height;

        let scale = |by: f32| Size::new(source.width * by, source.height * by);
        let scaled = match self {
            Self::Fill => container,
            Self::Contain => scale(wide.min(tall)),
            Self::Cover => scale(wide.max(tall)),
            Self::None => source,
            Self::ScaleDown => scale(wide.min(tall).min(1.0)),
        };

        let offset = alignment.inscribe(scaled, container);
        Rect::new(
            bounds.left + offset.dx,
            bounds.top + offset.dy,
            bounds.left + offset.dx + scaled.width,
            bounds.top + offset.dy + scaled.height,
        )
    }

    /// `true` if [`apply`](Self::apply) can produce a rect escaping `bounds`, so
    /// a caller knows whether a clip is worth the save/restore pair.
    ///
    /// Answered from the geometry rather than from the variant: `Cover` on an
    /// image that happens to match the box's aspect ratio needs no clip, and a
    /// clip that is never exceeded still costs two commands on every frame.
    #[must_use]
    pub fn overflows(self, source: Size, bounds: Rect, alignment: Alignment) -> bool {
        let drawn = self.apply(source, bounds, alignment);
        drawn.left < bounds.left - f32::EPSILON
            || drawn.top < bounds.top - f32::EPSILON
            || drawn.right > bounds.right + f32::EPSILON
            || drawn.bottom > bounds.bottom + f32::EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole `Arc` change exists for: a shared buffer stays
    /// *the same* buffer, so damage tracking keeps reporting it unchanged.
    #[test]
    fn a_shared_buffer_survives_a_rebuild_as_the_same_image() {
        let pixels = Arc::new(vec![0u8; 2 * 2 * 4]);

        let first = Image::from_shared_rgba8(Arc::clone(&pixels), 2, 2);
        let rebuilt = Image::from_shared_rgba8(Arc::clone(&pixels), 2, 2);

        assert_eq!(first, rebuilt, "the same allocation must compare equal");
        assert_eq!(Arc::strong_count(&pixels), 3, "nothing was copied");
    }

    /// And the failure it replaces: wrapping the same *contents* twice is two
    /// allocations, which compare unequal and repaint. Asserted so the
    /// distinction is written down rather than rediscovered.
    #[test]
    fn two_copies_of_the_same_pixels_compare_unequal() {
        let a = Image::from_rgba8(vec![7u8; 2 * 2 * 4], 2, 2);
        let b = Image::from_rgba8(vec![7u8; 2 * 2 * 4], 2, 2);
        assert_ne!(a, b, "identity, not content — see the PartialEq note");
    }

    /// A buffer that is the wrong length is a bug at the call site, and a
    /// silently stretched image is the worst way to find out.
    #[test]
    #[should_panic(expected = "does not match")]
    fn a_mismatched_buffer_panics() {
        let _ = Image::from_shared_rgba8(Arc::new(vec![0u8; 3]), 2, 2);
    }

    fn image(width: u32, height: u32) -> Image {
        Image::from_rgba8(
            vec![0; (width as usize) * (height as usize) * 4],
            width,
            height,
        )
    }

    /// A 200x100 box, and a source whose shape the tests vary.
    fn bounds() -> Rect {
        Rect::new(0.0, 0.0, 200.0, 100.0)
    }

    #[test]
    fn a_cloned_image_compares_equal_but_a_re_decoded_one_does_not() {
        let first = image(2, 2);
        assert_eq!(first, first.clone());
        assert_ne!(first, image(2, 2), "same pixels, separately decoded");
    }

    #[test]
    #[should_panic(expected = "does not match")]
    fn mismatched_image_data_is_rejected_at_construction() {
        let _ = Image::from_rgba8(vec![0; 3], 2, 2);
    }

    #[test]
    fn an_images_natural_size_is_its_pixel_dimensions() {
        assert_eq!(image(640, 480).size(), Size::new(640.0, 480.0));
    }

    #[test]
    fn fill_takes_the_whole_box_and_distorts() {
        let drawn = BoxFit::Fill.apply(Size::square(10.0), bounds(), Alignment::CENTER);
        assert_eq!(drawn, bounds());
    }

    #[test]
    fn contain_fits_entirely_and_letterboxes() {
        // A square into a 200x100 box: height is the binding axis, so 100x100
        // centred leaves 50 either side.
        let drawn = BoxFit::Contain.apply(Size::square(10.0), bounds(), Alignment::CENTER);
        assert_eq!(drawn, Rect::new(50.0, 0.0, 150.0, 100.0));
    }

    #[test]
    fn cover_fills_the_box_and_spills_over() {
        // Width is the binding axis now: 200x200, which overflows a 100-tall box
        // by 50 top and bottom.
        let drawn = BoxFit::Cover.apply(Size::square(10.0), bounds(), Alignment::CENTER);
        assert_eq!(drawn, Rect::new(0.0, -50.0, 200.0, 150.0));
    }

    #[test]
    fn none_draws_at_natural_size_wherever_it_is_aligned() {
        let drawn = BoxFit::None.apply(Size::square(40.0), bounds(), Alignment::CENTER);
        assert_eq!(drawn, Rect::new(80.0, 30.0, 120.0, 70.0));
    }

    #[test]
    fn scale_down_shrinks_a_large_source_but_leaves_a_small_one_alone() {
        let large = BoxFit::ScaleDown.apply(Size::square(1000.0), bounds(), Alignment::CENTER);
        let contained = BoxFit::Contain.apply(Size::square(1000.0), bounds(), Alignment::CENTER);
        assert_eq!(large, contained, "too big to fit, so it behaves as Contain");

        let small = BoxFit::ScaleDown.apply(Size::square(20.0), bounds(), Alignment::CENTER);
        let natural = BoxFit::None.apply(Size::square(20.0), bounds(), Alignment::CENTER);
        assert_eq!(small, natural, "small enough already, so it is left alone");
    }

    #[test]
    fn alignment_places_what_does_not_fill_the_box() {
        let left = BoxFit::Contain.apply(Size::square(10.0), bounds(), Alignment::CENTER_LEFT);
        assert_eq!(left, Rect::new(0.0, 0.0, 100.0, 100.0));

        let right = BoxFit::Contain.apply(Size::square(10.0), bounds(), Alignment::CENTER_RIGHT);
        assert_eq!(right, Rect::new(100.0, 0.0, 200.0, 100.0));
    }

    #[test]
    fn the_rect_is_offset_by_where_the_box_actually_is() {
        let moved = Rect::new(1000.0, 500.0, 1200.0, 600.0);
        let drawn = BoxFit::Fill.apply(Size::square(10.0), moved, Alignment::CENTER);
        assert_eq!(
            drawn, moved,
            "fitting is relative to the box, not the origin"
        );
    }

    #[test]
    fn only_the_fits_that_can_escape_report_an_overflow() {
        let square = Size::square(10.0);
        assert!(!BoxFit::Contain.overflows(square, bounds(), Alignment::CENTER));
        assert!(!BoxFit::Fill.overflows(square, bounds(), Alignment::CENTER));
        assert!(BoxFit::Cover.overflows(square, bounds(), Alignment::CENTER));
    }

    #[test]
    fn cover_on_a_matching_aspect_ratio_needs_no_clip() {
        // The reason `overflows` measures rather than matching on the variant.
        let same_shape = Size::new(20.0, 10.0);
        assert!(!BoxFit::Cover.overflows(same_shape, bounds(), Alignment::CENTER));
    }

    #[test]
    fn a_natural_size_larger_than_the_box_overflows_too() {
        assert!(BoxFit::None.overflows(Size::square(500.0), bounds(), Alignment::CENTER));
    }

    #[test]
    fn a_source_with_no_area_draws_nothing_rather_than_dividing_by_zero() {
        let drawn = BoxFit::Cover.apply(Size::new(0.0, 10.0), bounds(), Alignment::CENTER);
        assert_eq!(drawn.width(), 0.0);
        assert_eq!(drawn.height(), 0.0);
        assert!(!BoxFit::Cover.overflows(Size::ZERO, bounds(), Alignment::CENTER));
    }
}
