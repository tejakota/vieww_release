//! The application's mark, in one place, drawn at whatever size is asked for.
//!
//! # Why this is a module and not a picture
//!
//! Same argument `examples/icon.rs` makes for generating the icon rather than
//! committing a PNG: a picture is a thing nobody can edit, and a mark that
//! exists three times — as an `.icns`, as a splash, as a title-bar glyph —
//! drifts in two of them. So the shape lives here, as fractions of a side, and
//! everything that needs it asks for it at a size:
//!
//! * the title bar, at 18 points, where the three decorative traffic-light dots
//!   used to be;
//! * [`crate::ui::splash`], at a third of the window, with its three parts
//!   revealed on their own clocks;
//! * `examples/icon.rs`, which draws the same fractions per pixel because it
//!   has no widget tree to build into — the one copy, and the reason the
//!   colours below are stated as constants that must equal its.
//!
//! # The shape
//!
//! Two overlapping rounded panels on a rounded square: the editor and the
//! preview, which is what the application *is*. It reads at 16 pixels because
//! it is two shapes and one accent, which is all that survives that size.
//! Anything with a glyph in it would be a smudge.
//!
//! ```text
//! ground   0.00, 0.00, 1.00 x 1.00   radius 0.22
//! editor   0.16, 0.20, 0.40 x 0.60   radius 0.08
//! preview  0.44, 0.32, 0.40 x 0.48   radius 0.08
//! ```
//!
//! `examples/icon.rs`: ../../../../examples/icon.rs

use vieww_foundation::{Alignment, Color, Gradient, Offset, Rect, Shadow, Size, Sketchbook};
use vieww_widget::prelude::*;
use vieww_widget::{Container, Painting, Positioned, SizedBox, Stack, StackFit, Transformed};

/// The ground the panels sit on. `examples/icon.rs`'s `GROUND`.
pub const GROUND: Color = Color::rgba(0x14, 0x16, 0x1A, 0xFF);
/// The editor panel. `examples/icon.rs`'s `PANEL`.
///
/// Lifted from `#252A33`, which was six points of luminance above the ground
/// and therefore invisible at the size the mark is actually seen at: in the
/// title bar, at eighteen points, the "two panels" read as one accent
/// rectangle floating on a dark square. A mark whose shape only resolves in
/// the installer is a mark with one shape.
pub const PANEL: Color = Color::rgba(0x46, 0x4E, 0x5E, 0xFF);
/// The preview panel, and the one piece of colour.
///
/// Read from [`crate::theme::ACCENT`] rather than typed again, so the mark and
/// the interface change accent together. `examples/icon.rs` states the same two
/// colours as byte arrays because it has no widget tree to build into; its test
/// is that they still match.
pub const ACCENT: Color = crate::theme::ACCENT.dark_near;

/// The far end of the accent panel's ramp.
///
/// A mark is the one place in a product where a flat fill is a missed
/// opportunity rather than a discipline: it is seen at 16 pixels in a taskbar
/// and at 512 in an installer, and the thing that survives both is a shape with
/// light on it.
pub const ACCENT_FAR: Color = crate::theme::ACCENT.dark_far;

/// The editor panel's rectangle, as fractions of the side.
const EDITOR: (f32, f32, f32, f32) = (0.16, 0.20, 0.40, 0.60);
/// The preview panel's. Drawn second, so it sits on top — the relationship the
/// two panes actually have on screen.
const PREVIEW: (f32, f32, f32, f32) = (0.44, 0.32, 0.40, 0.48);

/// The mark, whole, `side` points square.
#[must_use]
pub fn mark(side: f32) -> WidgetNode {
    revealed(side, 1.0, 1.0)
}

/// The mark with its two panels part-way in, for an entrance.
///
/// `editor` and `preview` run `0..=1` and are each a fade and a scale about the
/// panel's own centre. The ground is always whole: it is the thing the panels
/// arrive *into*, and a ground that grows with them is a mark that pulses.
#[must_use]
pub fn revealed(side: f32, editor: f32, preview: f32) -> WidgetNode {
    let ground: WidgetNode = Painting::sized(Size::new(side, side), Ground { side }).into();

    Stack::new()
        .fit(StackFit::Loose)
        .alignment(Alignment::TOP_LEFT)
        .children(children![
            ground,
            panel(side, EDITOR, PANEL, editor, false),
            panel(side, PREVIEW, ACCENT, preview, true),
        ])
        .into()
}

/// The rounded square the two panels sit on.
///
/// A painting rather than a `Container`, for the reason the accent ramp exists:
/// the mark is the product's face and a flat charcoal square with two
/// rectangles on it is a wireframe of a logo. Here it has a ramp across it, a
/// rim of light along its top edge, and — at the sizes where it is a real
/// object rather than a glyph — a shadow under it.
#[derive(Debug)]
struct Ground {
    side: f32,
}

impl vieww_widget::Painter for Ground {
    fn paint(&self, book: &mut Sketchbook, size: Size) {
        let radius = self.side * 0.22;
        let body = Rect::new(0.0, 0.0, size.width, size.height);
        // Below about twenty points the shadow is a smudge under a glyph
        // rather than depth under an object, and the glyph is what a title bar
        // wants.
        if self.side >= 20.0 {
            book.shadow(
                body,
                radius,
                Shadow::new(
                    Color::rgba(0, 0, 0, 130),
                    Offset::new(0.0, self.side * 0.05),
                    self.side * 0.14,
                ),
            );
        }
        book.rrect(
            body,
            radius,
            Gradient::vertical().between(Color::rgba(0x24, 0x27, 0x2E, 0xFF), GROUND),
        );
        // The rim, as a filled ring rather than a stroke: a one-point stroke
        // of a rounded rectangle is the exact shape class GPU stroke
        // rasterisers are least reliable on, and a hairline that smears is
        // more visible than one that is a hair crisper. The band a stroke of
        // the inset path would cover and the ring cover are the same one
        // point of ink inside the body's edge.
        book.fill(
            vieww_foundation::Path::rounded_ring(body, radius, 1.0),
            Color::rgba(255, 255, 255, 26),
        );
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// One panel, placed by its fractions and revealed by `t`.
///
/// The scale is about the panel's own centre, which takes two transforms rather
/// than one: [`Transformed::scale`] scales about the top-left, so the shrink is
/// undone by translating back half of what it took away. An earlier version of
/// the splash tried to place panels with a container's *padding* instead, which
/// paints the background across the padding as well — a "panel" reaching from
/// the mark's corner to the panel's centre.
fn panel(side: f32, rect: (f32, f32, f32, f32), color: Color, t: f32, accent: bool) -> WidgetNode {
    let (fx, fy, fw, fh) = rect;
    let (width, height) = (side * fw, side * fh);
    let t = t.clamp(0.0, 1.0);
    let inset = Offset::new((1.0 - t) * width / 2.0, (1.0 - t) * height / 2.0);

    // The accent panel is the one piece of colour, so it is the one that gets
    // a ramp; the editor panel behind it stays flat, because two ramps in a
    // mark this size read as a gradient rather than as two panels.
    let mut block = Container::new()
        .color(color)
        .radius(side * 0.08)
        .child(SizedBox::from_size(Size::new(width, height)));
    if accent {
        block = block
            .gradient(Gradient::vertical().between(ACCENT_FAR, color))
            .shadow(Shadow::new(
                Color::rgba(0, 0, 0, 90),
                Offset::new(0.0, side * 0.02),
                side * 0.07,
            ));
    }

    let at = Positioned::new().left(side * fx).top(side * fy);

    // **A finished panel is a plain rectangle again.** An identity opacity and
    // two identity transforms draw the same pixels and cost three render
    // objects apiece, on a widget that is in the tree for the whole session —
    // and they show up in every tree dump, where `tests/shell.rs` reads the
    // shell looking for an `Opacity` that would mean the *preview* was being
    // faded for staleness. The animation is the exception; whole is the rule.
    if t >= 1.0 {
        return at.child(block).into();
    }

    at.child(
        Opacity::new(t)
            .child(Transformed::translate(inset).child(Transformed::scale(t, t).child(block))),
    )
    .into()
}
