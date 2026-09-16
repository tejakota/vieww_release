//! A render object vieww has never heard of, drawn by vieww.
//!
//! ```console
//! cargo test -p vieww --test third_party_render_widget
//! ```
//!
//! # What this is for
//!
//! `docs/DESIGN.md` and the factory's own documentation both claim that a third
//! party can add a render widget without editing this repository: implement
//! [`RenderObject`], register the widget type, done. **Nothing tested that
//! claim from outside**, and it was false in the way a claim is worst — the
//! mechanism was built, public and unit-tested, and there was no route to it.
//!
//! `RenderFactory::register` and `RenderOwner::register` were both public. But
//! `FrameDriver::owner` hands out `&RenderOwner`, `App::run` constructs the
//! driver itself, and an application therefore held nothing it could register
//! *through*. The gap was one accessor, and it survived an extensibility audit
//! because everything the audit looked at was present and correct.
//!
//! So this test stands where an application stands: it uses only what
//! `vieww` re-exports, builds the driver the way `App` builds it, and asserts
//! that a widget defined *here* — in a test crate, with no entry anywhere in
//! `vieww-render` — lays out and paints.
//!
//! # Why it asserts pixels rather than "no panic"
//!
//! An unregistered render widget is not an error. `RenderFactory::create`
//! returns `None`, the element realises no render object, and the tree draws
//! **nothing at all** — quietly, and looking exactly like a widget that decided
//! it had nothing to show. A test that only checked for a panic would pass
//! against the broken version. So the assertion is the fill: the shape, the
//! colour and the size that only this render object knows how to produce.

use vieww::foundation::{Color, Constraints, Size};
use vieww::render::{LayoutCtx, PaintCtx};
use vieww::{FrameDriver, RenderObject, Widget, WidgetKind};

const SURFACE: Size = Size {
    width: 200.0,
    height: 200.0,
};
/// Deliberately not a size any built-in would choose, so a fill of these
/// dimensions can only have come from the render object below.
const BADGE: Size = Size {
    width: 37.0,
    height: 19.0,
};
const INK: Color = Color::rgb(11, 222, 33);

/// A widget from outside the framework.
///
/// `RenderLeaf`, so it owns a render object and has no children — the kind that
/// is impossible to write without the registry, since a `Composed` widget only
/// ever assembles built-ins and needs nothing from this seam.
#[derive(Debug)]
struct DemoBadge {
    ink: Color,
}

impl Widget for DemoBadge {
    fn debug_name(&self) -> &'static str {
        "DemoBadge"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
}

vieww::widget::widget_node_from!(DemoBadge);

/// Its render object, which vieww has no way of knowing about.
#[derive(Debug)]
struct RenderDemoBadge {
    ink: Color,
}

impl RenderObject for RenderDemoBadge {
    fn debug_name(&self) -> &'static str {
        "RenderDemoBadge"
    }

    /// A fixed size, constrained — so the assertion below is about this object's
    /// choice rather than about whatever the parent handed down.
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(BADGE)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        ctx.canvas().fill_rect(bounds, self.ink.into());
    }
}

/// The driver an application gets: constructed exactly as `App::run` does it,
/// with the built-in factory and nothing else.
fn driver() -> FrameDriver {
    FrameDriver::new(SURFACE)
}

#[test]
fn a_registered_third_party_render_widget_lays_out_and_paints() {
    use vieww::prelude::*;

    let mut driver = driver();

    // The one call this test exists for. Before `FrameDriver::register` there
    // was no way to make it from here.
    driver.register::<DemoBadge, RenderDemoBadge>(|badge| RenderDemoBadge { ink: badge.ink });

    // **Centred, and that is not decoration.** `FrameDriver::new` lays the root
    // out under `Constraints::tight(surface)` — a window is a fixed size and a
    // root filling it is what makes a full-screen background work — so a bare
    // `DemoBadge` root is stretched to 200×200 and `layout`'s own answer is
    // invisible. `Center` hands its child loose constraints, which is what lets
    // the assertion below be about this render object's choice rather than
    // about the window's.
    driver.set_root(Center::new().child(DemoBadge { ink: INK }));
    driver.draw_frame();

    let fills = driver.scene().fills();
    assert_eq!(
        fills.len(),
        1,
        "one widget, one fill — got {} of them",
        fills.len()
    );

    let (shape, paint) = &fills[0];
    assert_eq!(
        paint.color, INK,
        "the colour came out of the widget, through the constructor, into the \
         render object, and onto the screen"
    );
    assert_eq!(
        shape.size(),
        BADGE,
        "and the size is the one `RenderDemoBadge::layout` chose, which nothing in \
         vieww could have produced"
    );
}

#[test]
fn without_the_registration_it_draws_nothing_at_all() {
    // **The other half, and the reason the test above asserts pixels.** This is
    // the behaviour every application had before `FrameDriver::register`
    // existed: not a panic, not an error placeholder — silence. A widget that
    // draws nothing looks identical to a widget that had nothing to draw, which
    // is why the gap survived an audit.
    let mut driver = driver();
    driver.set_root(DemoBadge { ink: INK });
    driver.draw_frame();

    assert!(
        driver.scene().fills().is_empty(),
        "an unregistered render widget realises no render object, so there is \
         nothing to paint — and nothing to notice"
    );
}

#[test]
fn a_third_party_render_widget_composes_with_built_ins() {
    // Registration is not a mode. A tree that mixes a foreign render object in
    // among built-in widgets has to lay out as one tree, or the seam is only
    // good for a root.
    use vieww::prelude::*;

    let mut driver = driver();
    driver.register::<DemoBadge, RenderDemoBadge>(|badge| RenderDemoBadge { ink: badge.ink });

    driver.set_root(Flex::column().children(children![
        ColoredBox::new(Color::rgb(9, 9, 9)).child(SizedBox::square(10.0)),
        DemoBadge { ink: INK },
    ]));
    driver.draw_frame();

    let fills = driver.scene().fills();
    assert_eq!(fills.len(), 2, "the built-in and the foreign one");

    let badge = fills
        .iter()
        .find(|(_, paint)| paint.color == INK)
        .expect("the foreign render object painted");
    assert_eq!(
        badge.0.size(),
        BADGE,
        "at its own size, inside a built-in layout that never heard of it"
    );
    assert!(
        badge.0.top > 0.0,
        "and below the built-in, so the column laid both out as one tree \
         rather than stacking them at the origin"
    );
}
