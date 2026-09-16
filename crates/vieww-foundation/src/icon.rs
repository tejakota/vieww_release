//! What an icon *is*: a filled shape and the box it was drawn in.
//!
//! The shape is a [`Path`], which lives here for exactly this reason — a widget
//! that could not name a path could not describe an icon, and the widget layer
//! may not depend on the paint layer (`docs/DESIGN.md` §7).
//!
//! # Why a viewbox and not just a path
//!
//! An icon's meaning includes its margins. A tick drawn to fill its box edge to
//! edge and a tick drawn with room around it are different icons, and scaling by
//! the path's own *bounds* would erase the difference — every icon would end up
//! flush against its box, and a set of them would visually jump around as each
//! one's ink happened to extend further than the last. Scaling by the declared
//! viewbox keeps a set consistent, which is the whole point of having one.

use std::rc::Rc;

use crate::{Path, Rect};

/// A filled shape, and the coordinate box it was designed in.
///
/// Cheap to clone: the path is shared, because an icon is described once and
/// then rebuilt into a widget tree on every frame.
#[derive(Debug, Clone, PartialEq)]
pub struct IconData {
    path: Rc<Path>,
    viewbox: Rect,
}

impl IconData {
    /// An icon whose `path` is drawn within `viewbox`.
    #[must_use]
    pub fn new(path: Path, viewbox: Rect) -> Self {
        Self {
            path: Rc::new(path),
            viewbox,
        }
    }

    /// An icon drawn on the 24x24 grid the built-in set uses.
    #[must_use]
    pub fn square24(path: Path) -> Self {
        Self::new(path, Rect::new(0.0, 0.0, 24.0, 24.0))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn viewbox(&self) -> Rect {
        self.viewbox
    }

    /// This icon's shape, scaled to sit inside `bounds`.
    ///
    /// Uniform and centred — see [`Path::fitted`].
    #[must_use]
    pub fn fitted(&self, bounds: Rect) -> Path {
        self.path.fitted(self.viewbox, bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Offset;

    /// A shape that deliberately does not fill its viewbox.
    fn inset_square() -> IconData {
        IconData::square24(Path::rect(Rect::new(6.0, 6.0, 18.0, 18.0)))
    }

    #[test]
    fn an_icons_margins_survive_being_scaled() {
        let fitted = inset_square().fitted(Rect::new(0.0, 0.0, 48.0, 48.0));
        let bounds = fitted.bounds();

        // 6/24 of the way in, at twice the size: 12, not 0.
        assert!((bounds.left - 12.0).abs() < 1e-4, "{bounds:?}");
        assert!((bounds.width() - 24.0).abs() < 1e-4, "{bounds:?}");
    }

    #[test]
    fn two_icons_with_different_ink_still_agree_on_scale() {
        let big = IconData::square24(Path::rect(Rect::new(0.0, 0.0, 24.0, 24.0)));
        let small = inset_square();
        let into = Rect::new(0.0, 0.0, 24.0, 24.0);

        assert_eq!(big.fitted(into).bounds(), into);
        assert_eq!(
            small.fitted(into).bounds(),
            Rect::new(6.0, 6.0, 18.0, 18.0),
            "scaling by ink rather than viewbox would make these identical"
        );
    }

    #[test]
    fn cloning_shares_the_shape_rather_than_copying_it() {
        let icon = inset_square();
        let clone = icon.clone();
        assert!(Rc::ptr_eq(&icon.path, &clone.path));
        assert_eq!(icon.path().bounds().origin(), Offset::new(6.0, 6.0));
    }
}
