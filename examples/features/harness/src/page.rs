//! The page every example is mounted inside.
//!
//! # Why an example needs a wrapper at all
//!
//! Each example asks for a window big enough for what it draws. A window
//! manager is free to give it something smaller — a tiling one always does, a
//! scaled display often does, and a phone never asks — and a tree that does not
//! fit is simply cut off at the bottom with nothing to do about it. That is how
//! "the content below cannot be reached" happens to an example that looked
//! right on the machine it was written on.
//!
//! So the root goes inside a scrollable page: **it scrolls only when it has
//! to**, because the child is given the window's height as a *minimum*. An
//! example that fits is laid out exactly as it was before — a `Container` that
//! filled the window still fills it — and one that does not gets a wheel and a
//! drag instead of a crop.
//!
//! # Why the limit is a `Cell` and not a signal
//!
//! It is written from layout, by `on_extents`. Writing a signal there marks the
//! tree pending from inside the layout phase, which is a rebuild asking for
//! another layout: the loop that produces. Nothing *reads* the limit during a
//! build — only the drag handler does, at the moment a finger moves — so plain
//! interior mutability is both enough and the thing that terminates.

use std::cell::Cell;
use std::rc::Rc;

use vieww_element::Signal;
use vieww_foundation::{Constraints, DragDetails, Size};
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::ScrollExtents;

/// Mount `root` as a scroll-when-needed page.
///
/// Every example calls this instead of [`FrameDriver::set_root`].
pub fn set_page(driver: &mut FrameDriver, root: impl Into<WidgetNode>) {
    let offset = driver.elements().runtime().signal(0.0f32);
    driver.set_root(Page {
        child: root.into(),
        offset,
        limit: Rc::new(Cell::new(0.0)),
    });
}

#[derive(Debug)]
struct Page {
    child: WidgetNode,
    offset: Signal<f32>,
    /// How far the content can scroll: `content - viewport`, or zero when it
    /// fits. Learned from layout, so it is right for whatever size the window
    /// actually turned out to be.
    limit: Rc<Cell<f32>>,
}

impl Widget for Page {
    fn debug_name(&self) -> &'static str {
        "Page"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let offset = self.offset.get();

        let on_drag = {
            let held = self.offset.clone();
            let limit = Rc::clone(&self.limit);
            let handler: Rc<dyn Fn(DragDetails)> = Rc::new(move |drag: DragDetails| {
                // A wheel notch arrives here too — `Scrollable` reports one as a
                // drag with no velocity — so this is the whole input path.
                let next = (held.peek() - drag.delta.dy).clamp(0.0, limit.get());
                if next != held.peek() {
                    held.set(next);
                }
            });
            handler
        };

        let on_extents = {
            let limit = Rc::clone(&self.limit);
            let held = self.offset.clone();
            let handler: Rc<dyn Fn(ScrollExtents)> = Rc::new(move |extents: ScrollExtents| {
                let room = (extents.content - extents.viewport).max(0.0);
                limit.set(room);
                // A window that grew can leave the page scrolled past its own
                // end, which reads as a blank sheet with content above it.
                if held.peek() > room {
                    held.set(room);
                }
            });
            handler
        };

        let child = self.child.clone();
        LayoutBuilder::new(move |constraints: Constraints| {
            // The window's height as a *minimum*, not a maximum: inside a
            // viewport the main axis is unbounded, and without this every
            // example that fills its window would collapse to the height of its
            // contents and float in the top-left corner.
            let floor = if constraints.max_height.is_finite() {
                constraints.max_height
            } else {
                0.0
            };
            Scrollable::vertical(offset)
                .on_drag(Rc::clone(&on_drag))
                .on_extents(Rc::clone(&on_extents))
                .child(
                    Constrained::new(Constraints::new(
                        constraints.max_width,
                        constraints.max_width,
                        floor,
                        f32::INFINITY,
                    ))
                    .child(child.clone()),
                )
                .into()
        })
        .into()
    }
}

vieww_widget::widget_node_from!(Page);

/// The size an example asks for, clamped to something a laptop can show.
///
/// Some sheets are 900 points wide and 560 tall by design. That is fine on a
/// desktop and off the bottom of the screen on a scaled 13-inch display — and
/// since [`set_page`] makes every example scrollable, asking for less and
/// letting it scroll is strictly better than asking for more and being cropped
/// by the window manager.
#[must_use]
pub fn fits(size: Size) -> Size {
    const MAX: Size = Size::new(1280.0, 720.0);
    Size::new(size.width.min(MAX.width), size.height.min(MAX.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use vieww_foundation::{Color, Offset, ScrollEvent};
    use vieww_widget::{Container, Flex, SizedBox};

    /// A tree of `count` 100pt bands, tall enough to overflow a 300pt window
    /// when `count` is large and to fit when it is small.
    fn bands(count: usize) -> WidgetNode {
        Flex::column()
            .children(
                (0..count)
                    .map(|row| {
                        Container::new()
                            .color(if row % 2 == 0 {
                                Color::rgb(230, 235, 242)
                            } else {
                                Color::WHITE
                            })
                            .child(SizedBox::from_size(Size::new(200.0, 100.0)))
                            .into()
                    })
                    .collect::<Vec<WidgetNode>>(),
            )
            .into()
    }

    /// Wheel a page and report the ink pattern's vertical shift, by sampling
    /// where the first band boundary is.
    fn wheel(driver: &mut FrameDriver, by: f32) -> bool {
        let taken = driver.handle_scroll(&ScrollEvent {
            position: Offset::new(100.0, 150.0),
            delta: Offset::new(0.0, -by),
            timestamp: Duration::from_millis(16),
        });
        driver.draw_frame();
        taken
    }

    #[test]
    fn a_page_that_overflows_scrolls() {
        let mut driver = FrameDriver::new(Size::new(200.0, 300.0));
        set_page(&mut driver, bands(12));
        driver.draw_frame();
        let before = format!("{:?}", driver.scene().commands());

        assert!(
            wheel(&mut driver, 120.0),
            "the wheel reached the page rather than falling through it"
        );

        assert_ne!(
            before,
            format!("{:?}", driver.scene().commands()),
            "1200pt of content in a 300pt window must move when wheeled"
        );
    }

    /// The case that must **not** move: a sheet that fits has nothing below the
    /// fold, and a page that scrolled it anyway would drag the content off the
    /// top of every example in the set.
    #[test]
    fn a_page_that_fits_does_not_scroll() {
        let mut driver = FrameDriver::new(Size::new(200.0, 300.0));
        set_page(&mut driver, bands(2));
        driver.draw_frame();
        let before = format!("{:?}", driver.scene().commands());

        wheel(&mut driver, 400.0);

        assert_eq!(
            before,
            format!("{:?}", driver.scene().commands()),
            "nothing to scroll, so nothing moved"
        );
    }
}
