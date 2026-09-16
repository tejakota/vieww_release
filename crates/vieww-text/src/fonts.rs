//! Font identity and the cache that resolves it.
//!
//! # Why a cache and not just a path
//!
//! A [`FontHandle`] is an integer. Everything
//! downstream of shaping — glyph runs, the renderer's atlas, the layout
//! cache — stores handles rather than font data, because the alternative is
//! either cloning megabytes of face data per run or threading a lifetime
//! through every type that touches text.
//!
//! The cache is the one place that maps a handle back to a face. It is also
//! where *fallback* lives: a request for "Helvetica" on a machine without it
//! resolves to something, and the decision of what that something is belongs
//! in one place rather than at each call site.
//!
//! # Relationship to `FontStore`
//!
//! [`FontStore`](crate::FontStore) is the cosmic-text-backed store the working
//! layout path uses. `FontCache` is the equivalent for the in-progress
//! shaper-agnostic stack. They are deliberately separate while the second
//! stack is being built; the intent is that `FontCache` eventually becomes a
//! thin view over `FontStore` rather than a second source of truth.

use std::collections::HashMap;

use crate::shaping::FontHandle;

/// A font family name.
///
/// A newtype rather than a bare `String` so that a family name cannot be
/// passed where a PostScript name or a file path is expected — a mistake
/// that produces a fallback font rather than an error, and so goes unnoticed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontFamily(pub String);

impl FontFamily {
    /// A family by name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The family name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl From<&str> for FontFamily {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

/// Font weight on the CSS numeric scale (100–900).
///
/// Numeric rather than an enum of names because the scale is continuous in
/// variable fonts, and because "Semibold" means 600 in one family and 640 in
/// another — the number is the portable thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const THIN: Self = Self(100);
    pub const EXTRA_LIGHT: Self = Self(200);
    pub const LIGHT: Self = Self(300);
    pub const NORMAL: Self = Self(400);
    pub const MEDIUM: Self = Self(500);
    pub const SEMI_BOLD: Self = Self(600);
    pub const BOLD: Self = Self(700);
    pub const EXTRA_BOLD: Self = Self(800);
    pub const BLACK: Self = Self(900);

    /// Clamp an arbitrary number into the valid 100–900 range.
    #[must_use]
    pub fn clamped(value: u16) -> Self {
        Self(value.clamp(100, 900))
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// The key a face is looked up by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FaceKey {
    family: FontFamily,
    weight: FontWeight,
    italic: bool,
}

/// Maps font descriptions to handles, and handles back to face data.
///
/// # Fallback
///
/// [`resolve`](Self::resolve) never fails. A family the cache does not have
/// resolves to the fallback family, and if that is also absent, to whatever
/// was registered first. Text rendered in the wrong font is a cosmetic bug;
/// text that fails to render is a blank screen.
#[derive(Debug, Default)]
pub struct FontCache {
    /// Registered faces, in registration order. The index is the handle.
    faces: Vec<RegisteredFace>,
    /// Lookup from description to handle.
    by_key: HashMap<FaceKey, FontHandle>,
    /// The family to fall back to when a request cannot be satisfied.
    fallback: Option<FontFamily>,
}

/// One registered face.
#[derive(Debug, Clone)]
pub struct RegisteredFace {
    pub family: FontFamily,
    pub weight: FontWeight,
    pub italic: bool,
    /// The raw face bytes. Shared, because a variable font serves many
    /// weights from one file.
    pub data: std::sync::Arc<Vec<u8>>,
}

impl FontCache {
    /// An empty cache with no faces and no fallback.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a face, returning its handle.
    ///
    /// Registering the same description twice returns the original handle
    /// rather than shadowing it — handles already handed out must keep
    /// meaning the same face.
    pub fn register(
        &mut self,
        family: impl Into<FontFamily>,
        weight: FontWeight,
        italic: bool,
        data: Vec<u8>,
    ) -> FontHandle {
        let key = FaceKey {
            family: family.into(),
            weight,
            italic,
        };

        if let Some(existing) = self.by_key.get(&key) {
            return *existing;
        }

        let handle = FontHandle(self.faces.len() as u64);
        self.faces.push(RegisteredFace {
            family: key.family.clone(),
            weight,
            italic,
            data: std::sync::Arc::new(data),
        });
        self.by_key.insert(key, handle);

        // The first face registered becomes the fallback unless one was set.
        if self.fallback.is_none() {
            self.fallback = self.faces.first().map(|f| f.family.clone());
        }

        handle
    }

    /// Set the fallback family used when a request cannot be satisfied.
    pub fn set_fallback(&mut self, family: impl Into<FontFamily>) {
        self.fallback = Some(family.into());
    }

    /// Resolve a font description to a handle.
    ///
    /// Returns `None` only when the cache is completely empty — there is no
    /// face to fall back to.
    #[must_use]
    pub fn resolve(
        &self,
        family: &FontFamily,
        weight: FontWeight,
        italic: bool,
    ) -> Option<FontHandle> {
        let exact = FaceKey {
            family: family.clone(),
            weight,
            italic,
        };
        if let Some(handle) = self.by_key.get(&exact) {
            return Some(*handle);
        }

        // Same family, nearest weight. Preferring the same family over the
        // exact weight is deliberate: a bold-ish Helvetica reads as Helvetica,
        // a bold Times does not.
        let nearest_in_family = self
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.family == *family && f.italic == italic)
            .min_by_key(|(_, f)| f.weight.0.abs_diff(weight.0))
            .map(|(i, _)| FontHandle(i as u64));

        if nearest_in_family.is_some() {
            return nearest_in_family;
        }

        // The fallback family, then anything at all.
        if let Some(fallback) = &self.fallback {
            if fallback != family {
                if let Some(handle) = self.resolve(fallback, weight, italic) {
                    return Some(handle);
                }
            }
        }

        (!self.faces.is_empty()).then_some(FontHandle(0))
    }

    /// The face behind a handle.
    #[must_use]
    pub fn face(&self, handle: FontHandle) -> Option<&RegisteredFace> {
        self.faces.get(handle.0 as usize)
    }

    /// How many faces are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// `true` if no faces are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_with_helvetica() -> FontCache {
        let mut cache = FontCache::new();
        cache.register("Helvetica", FontWeight::NORMAL, false, vec![0u8; 4]);
        cache
    }

    #[test]
    fn an_empty_cache_resolves_nothing() {
        let cache = FontCache::new();
        assert_eq!(
            cache.resolve(&"Helvetica".into(), FontWeight::NORMAL, false),
            None
        );
    }

    #[test]
    fn an_exact_match_resolves_to_itself() {
        let cache = cache_with_helvetica();
        let handle = cache
            .resolve(&"Helvetica".into(), FontWeight::NORMAL, false)
            .expect("registered");
        assert_eq!(cache.face(handle).unwrap().family.name(), "Helvetica");
    }

    #[test]
    fn registering_twice_returns_the_same_handle() {
        let mut cache = FontCache::new();
        let first = cache.register("Helvetica", FontWeight::NORMAL, false, vec![0u8; 4]);
        let second = cache.register("Helvetica", FontWeight::NORMAL, false, vec![9u8; 4]);
        assert_eq!(first, second, "a handle must keep meaning one face");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_missing_weight_falls_back_within_the_family() {
        let cache = cache_with_helvetica();
        // Bold Helvetica is not registered; normal Helvetica is closer than
        // nothing, and staying in the family matters more than the weight.
        let handle = cache
            .resolve(&"Helvetica".into(), FontWeight::BOLD, false)
            .expect("falls back");
        assert_eq!(cache.face(handle).unwrap().family.name(), "Helvetica");
    }

    #[test]
    fn a_missing_family_falls_back_rather_than_failing() {
        let cache = cache_with_helvetica();
        let handle = cache.resolve(&"Nonexistent".into(), FontWeight::NORMAL, false);
        assert!(
            handle.is_some(),
            "resolution must never fail while any face exists"
        );
    }

    #[test]
    fn weights_clamp_into_range() {
        assert_eq!(FontWeight::clamped(0), FontWeight(100));
        assert_eq!(FontWeight::clamped(5000), FontWeight(900));
        assert_eq!(FontWeight::clamped(550), FontWeight(550));
    }
}
