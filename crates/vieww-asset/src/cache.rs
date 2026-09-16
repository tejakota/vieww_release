//! Decoded pixels, kept.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use vieww_foundation::Image;

use crate::{decode, AssetBundle, AssetError};

/// Decoded images, keyed by asset path.
///
/// # Why a cache is not optional
///
/// [`Image`] compares by identity rather than by pixels — two separately decoded
/// copies of the same file are *unequal*, which `vieww_foundation::image`
/// documents and relies on. So an application that decodes on every build hands
/// the render tree a different image each frame, and every frame reports damage
/// over the whole picture. The cache is what makes an unchanged image an
/// unchanged image.
///
/// # No eviction, on purpose
///
/// This holds everything it is given until [`clear`](Self::clear). An LRU needs
/// a budget, a budget needs a number, and the right number depends on the device
/// and on what the application shows — so a wrong default here would be a memory
/// leak on one phone and a thrashing cache on another. An application that loads
/// unbounded images knows it does; one loading a fixed set of icons wants
/// exactly this.
#[derive(Debug, Default)]
pub struct ImageCache {
    entries: RefCell<HashMap<String, Image>>,
}

impl ImageCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The decoded image for `path`, decoding it through `bundle` on first ask.
    ///
    /// The second call for the same path returns the *same* image — identical by
    /// [`Image`]'s own identity comparison, which is what keeps damage tracking
    /// quiet. That is the property this type exists for.
    ///
    /// # Errors
    ///
    /// Whatever the bundle or the decoder reports. A failure is **not** cached:
    /// a missing file may be a download in flight, and remembering the error
    /// would make the retry impossible.
    pub fn load(&self, bundle: &dyn AssetBundle, path: &str) -> Result<Image, AssetError> {
        if let Some(image) = self.entries.borrow().get(path) {
            return Ok(image.clone());
        }

        let (image, _) = decode(&bundle.open(path)?)?;
        self.entries
            .borrow_mut()
            .insert(path.to_owned(), image.clone());
        Ok(image)
    }

    /// The image for `path` if it has already been decoded.
    ///
    /// For a widget that wants to draw *something* this frame and start a
    /// background decode rather than block: a hit draws, a miss shows a
    /// placeholder and spawns the work.
    #[must_use]
    pub fn peek(&self, path: &str) -> Option<Image> {
        self.entries.borrow().get(path).cloned()
    }

    /// Put an image in under `path`, replacing anything there.
    ///
    /// What a background decode calls when it finishes — the work happened off
    /// the UI thread, and this is the UI thread taking delivery.
    pub fn insert(&self, path: impl Into<String>, image: Image) {
        self.entries.borrow_mut().insert(path.into(), image);
    }

    /// Forget one entry.
    pub fn forget(&self, path: &str) {
        self.entries.borrow_mut().remove(path);
    }

    /// Forget everything.
    ///
    /// The right response to a memory warning, and the only eviction there is.
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
    }

    /// How many images are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }
}

/// A cache several widgets can hold at once.
pub type SharedImageCache = Rc<ImageCache>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EmbeddedBundle;

    /// A 1x1 red PNG, encoded at test time so there is no fixture to lose.
    fn png() -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255])
            .expect("a 1x1 image")
            .write_to(&mut buffer, image::ImageFormat::Png)
            .expect("encode");
        buffer.into_inner()
    }

    fn bundle() -> EmbeddedBundle {
        // `Box::leak` because `EmbeddedBundle` holds `&'static [u8]`, which is
        // what `include_bytes!` produces in real use.
        EmbeddedBundle::new().with("a.png", Box::leak(png().into_boxed_slice()))
    }

    #[test]
    fn the_second_load_returns_the_same_image_rather_than_a_second_decode() {
        // The whole reason this type exists: `Image` compares by identity, so
        // a re-decode damages the whole picture every frame.
        let cache = ImageCache::new();
        let bundle = bundle();

        let first = cache.load(&bundle, "a.png").expect("load");
        let second = cache.load(&bundle, "a.png").expect("load");

        assert_eq!(
            first, second,
            "two decodes of one file are unequal by design, and that would \
             repaint the screen every frame"
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_failure_is_not_remembered() {
        // A missing file may be a download in flight.
        let cache = ImageCache::new();
        let empty = EmbeddedBundle::new();
        assert!(cache.load(&empty, "later.png").is_err());
        assert!(cache.is_empty(), "or the retry could never succeed");
    }

    #[test]
    fn peek_does_not_decode() {
        let cache = ImageCache::new();
        assert!(cache.peek("a.png").is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn a_background_decode_can_hand_its_result_in() {
        let cache = ImageCache::new();
        let (decoded, _) = decode(&png()).expect("decode");
        cache.insert("a.png", decoded.clone());
        assert_eq!(cache.peek("a.png"), Some(decoded));
    }

    #[test]
    fn clearing_frees_everything() {
        let cache = ImageCache::new();
        cache.load(&bundle(), "a.png").expect("load");
        cache.clear();
        assert!(cache.is_empty());
    }
}
