//! Arbitrary vector content, drawn by a [`Painter`].

use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Key, Size, Sketchbook};

use crate::{widget_node_from, Widget, WidgetKind};

/// Draws vector content into a [`Sketchbook`], given the box it was laid out at.
///
/// The size arrives at paint time rather than at construction, which is the
/// difference between this and [`CustomPaint`](crate::CustomPaint): a painter
/// can fill whatever room it was given, so a background wash, a divider glow or
/// a device bezel does not have to be told its own dimensions by its parent.
///
/// ```
/// use vieww_foundation::{Color, Rect, Size, Sketchbook};
/// use vieww_widget::{Painter, Painting};
///
/// #[derive(Debug)]
/// struct Underline(Color);
///
/// impl Painter for Underline {
///     fn paint(&self, book: &mut Sketchbook, size: Size) {
///         book.rrect(
///             Rect::new(0.0, size.height - 2.0, size.width, size.height),
///             1.0,
///             self.0,
///         );
///     }
///
///     // For `should_repaint`'s downcast. Always `self`.
///     fn as_any(&self) -> &dyn std::any::Any {
///         self
///     }
/// }
///
/// let widget = Painting::new(Underline(Color::rgb(90, 140, 255)));
/// ```
pub trait Painter: fmt::Debug + 'static {
    /// Record the drawing. `size` is the box this painter was laid out at, with
    /// the origin at its top-left corner.
    fn paint(&self, book: &mut Sketchbook, size: Size);

    /// Whether a repaint is needed now that `previous` has been replaced by
    /// `self`.
    ///
    /// The default is `true`, which is always correct and sometimes wasteful.
    /// Override it when a painter carries values it can compare — the pattern
    /// is a downcast through [`Painter::as_any`], returning `true` when the
    /// downcast fails, because a painter of a different type is by definition
    /// a different drawing.
    fn should_repaint(&self, _previous: &dyn Painter) -> bool {
        true
    }

    /// For `should_repaint`'s downcast. Implement it as `self`, always.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A leaf that fills its box with whatever a [`Painter`] draws.
///
/// # Sizing
///
/// With no [`size`](Self::size) it takes the largest box its constraints allow,
/// the way a background does — so it is normally the child of something that
/// bounds it, or a layer in a [`Stack`](crate::Stack). With a size it asks for
/// that and takes what the constraints allow of it.
///
/// # Why not just write a render object
///
/// You can: [`FrameDriver::register`](../../vieww_render/struct.FrameDriver.html#method.register)
/// has always been the way to add one, and it hands you the real canvas. It
/// also means an application carrying a `vieww-render` dependency, a widget and
/// an object for every shape it wants to draw, and a registration call it will
/// forget once — an unregistered render widget draws *nothing*, silently. This
/// is the same power through one trait and no registration.
#[derive(Clone)]
pub struct Painting {
    painter: Rc<dyn Painter>,
    size: Option<Size>,
    key: Option<Key>,
}

impl fmt::Debug for Painting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Painting")
            .field("painter", &self.painter)
            .field("size", &self.size)
            .finish()
    }
}

impl Painting {
    /// A painting that fills the box it is given.
    #[must_use]
    pub fn new(painter: impl Painter) -> Self {
        Self {
            painter: Rc::new(painter),
            size: None,
            key: None,
        }
    }

    /// A painting that asks for `size`.
    #[must_use]
    pub fn sized(size: Size, painter: impl Painter) -> Self {
        Self {
            painter: Rc::new(painter),
            size: Some(size),
            key: None,
        }
    }

    /// Ask for a size rather than filling the box.
    #[must_use]
    pub const fn size(mut self, size: Size) -> Self {
        self.size = Some(size);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub fn painter(&self) -> &Rc<dyn Painter> {
        &self.painter
    }

    #[must_use]
    pub const fn requested_size(&self) -> Option<Size> {
        self.size
    }

    /// Record this painting at `size`, without a renderer.
    ///
    /// What the render object does, exposed so a test can assert on the
    /// drawing rather than on pixels.
    #[must_use]
    pub fn record(&self, size: Size) -> Sketchbook {
        let mut book = Sketchbook::new();
        self.painter.paint(&mut book, size);
        book
    }
}

impl Widget for Painting {
    fn debug_name(&self) -> &'static str {
        "Painting"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }
}

widget_node_from!(Painting);

/// A painter from a closure, for a drawing with nothing to remember.
///
/// `should_repaint` is `true` — a closure cannot be compared, so the honest
/// answer is that it may have changed. Write a named painter when the repaint
/// cost matters.
pub struct PaintWith<F: Fn(&mut Sketchbook, Size) + 'static> {
    draw: F,
}

impl<F: Fn(&mut Sketchbook, Size) + 'static> PaintWith<F> {
    pub const fn new(draw: F) -> Self {
        Self { draw }
    }
}

impl<F: Fn(&mut Sketchbook, Size) + 'static> fmt::Debug for PaintWith<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PaintWith")
    }
}

impl<F: Fn(&mut Sketchbook, Size) + 'static> Painter for PaintWith<F> {
    fn paint(&self, book: &mut Sketchbook, size: Size) {
        (self.draw)(book, size);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::{Color, Rect};

    #[derive(Debug)]
    struct Wash(Color);

    impl Painter for Wash {
        fn paint(&self, book: &mut Sketchbook, size: Size) {
            book.rect(Rect::new(0.0, 0.0, size.width, size.height), self.0);
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn a_painter_is_handed_the_box_it_was_laid_out_at() {
        let book = Painting::new(Wash(Color::RED)).record(Size::new(40.0, 12.0));
        let vieww_foundation::Sketch::Fill { path, .. } = &book.items()[0] else {
            panic!("a fill")
        };
        assert_eq!(path.bounds(), Rect::new(0.0, 0.0, 40.0, 12.0));
    }

    #[test]
    fn a_closure_painter_records_the_same_way() {
        let widget = Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
            book.circle(
                vieww_foundation::Offset::new(size.width / 2.0, size.height / 2.0),
                4.0,
                Color::BLUE,
            );
        }));
        assert_eq!(widget.record(Size::square(20.0)).len(), 1);
    }

    #[test]
    fn a_size_is_a_request_and_none_means_fill() {
        assert!(Painting::new(Wash(Color::RED)).requested_size().is_none());
        assert_eq!(
            Painting::sized(Size::square(16.0), Wash(Color::RED)).requested_size(),
            Some(Size::square(16.0))
        );
    }
}
