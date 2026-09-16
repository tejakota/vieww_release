//! End-to-end placement for [`Positioned`] inside a [`Stack`], on the same
//! `FrameDriver`-to-`Scene` harness `aspect_ratio`'s tests use.
//!
//! ```console
//! cargo test -p vieww --test positioned
//! ```
//!
//! # What each test isolates
//!
//! `RenderStack::layout` has three passes — size the un-positioned children
//! (which alone decide the stack's size), then size the positioned ones
//! against that now-known size, then place every child. Each test below is
//! aimed at exactly one of the branches resolving a positioned child's edges,
//! plus the passes it depends on staying correct.
//!
//! # Why some tests wrap a colour in a `SizedBox`, and some don't
//!
//! `ColoredBox` takes its size entirely from its child (`docs` on the type
//! itself); with none, it reports `constraints.smallest()`. Under **tight**
//! constraints that is still the right size — `smallest() == largest()` — so
//! a bare `ColoredBox::new(colour)` is enough wherever `Positioned` pins both
//! axes tight (a size given, or two opposite edges). Wherever an axis is
//! left loose, a bare `ColoredBox` would report zero on it, so those tests
//! give it a `SizedBox` child instead, to assert a real chosen size rather
//! than the box collapsing to nothing.

use vieww::prelude::*;

/// One frame of `root` on a `surface`-sized driver.
///
/// The root is laid out tight to `surface` — confirmed by the same rule
/// `paint_to_pixels`'s tests rely on — which is what makes the first few
/// tests below able to skip an explicit "sizing" child.
fn drawn(surface: Size, root: impl Into<WidgetNode>) -> FrameDriver {
    let mut driver = FrameDriver::new(surface);
    driver.set_root(root);
    driver.draw_frame();
    driver
}

/// Where the box of `color` actually landed, in pixels.
///
/// A zero-size box may not be recorded at all, so a missing fill is a
/// failure of the thing under test rather than of this helper — the same
/// convention `aspect_ratio`'s tests use.
fn painted(driver: &FrameDriver, color: Color) -> Rect {
    driver
        .scene()
        .fills()
        .into_iter()
        .find(|(_, paint)| paint.color == color)
        .map(|(rect, _)| rect)
        .unwrap_or_else(|| panic!("nothing painted {color:?}"))
}

/// A solid box of an exact size — `ColoredBox` shrink-wrapped around a
/// `SizedBox`, so it reports `size` regardless of whether the constraints it
/// is laid out under are loose.
fn solid(color: Color, size: Size) -> ColoredBox {
    ColoredBox::new(color).child(SizedBox::from_size(size))
}

#[test]
fn an_edge_pair_beats_a_size_but_a_single_edge_does_not() {
    // The over-constrained case, decided rather than left to chance. With
    // `left` *and* `right` both given, the pair is the more specific request
    // and an explicit `width` is ignored — the classic rule, whose
    // reference implementations test the edge pair first and only reach
    // `width` in the `else if`. See `StackPosition::width_within`, and
    // `both_edges_beat_an_explicit_width` in `vieww-widget` for the same rule
    // asserted at the unit level.
    //
    // The vertical axis is the other branch: only `top` is given, so there is
    // no pair for `height` to lose to and it applies. Both axes end up tight,
    // so a bare `ColoredBox` is enough.
    let stack = Stack::new().children(children![Positioned::new()
        .left(0.0)
        .right(0.0)
        .width(30.0) // ignored — the edge pair already spans the width
        .top(0.0)
        .height(20.0)
        .child(ColoredBox::new(Color::BLUE))]);

    let painted = painted(&drawn(Size::new(200.0, 200.0), stack), Color::BLUE);
    assert_eq!(
        painted.right - painted.left,
        200.0,
        "left+right span the stack and beat an explicit width"
    );
    assert_eq!(
        painted.bottom - painted.top,
        20.0,
        "height applies: only one vertical edge was given"
    );
}

#[test]
fn two_opposite_edges_with_no_size_pin_the_space_between_them() {
    // Both axes tight again — the space left between the two edges given.
    let stack = Stack::new().children(children![Positioned::new()
        .left(10.0)
        .right(20.0) // 200 - 10 - 20 = 170
        .top(0.0)
        .bottom(0.0) // 200 - 0 - 0 = 200
        .child(ColoredBox::new(Color::BLUE))]);

    let painted = painted(&drawn(Size::new(200.0, 200.0), stack), Color::BLUE);
    assert_eq!(painted.left, 10.0);
    assert_eq!(painted.right, 180.0, "200 - 20");
    assert_eq!(painted.right - painted.left, 170.0);
    assert_eq!(painted.bottom - painted.top, 200.0);
}

#[test]
fn one_edge_alone_places_that_side_and_sizes_loosely_from_it() {
    // Only `left` and `top` given, one edge per axis: both axes are loose,
    // so the child's own 30x30 has to come from a `SizedBox`, not the tight
    // constraints a bare `ColoredBox` would otherwise collapse to zero under.
    let stack = Stack::new().children(children![Positioned::new()
        .left(50.0)
        .top(0.0)
        .child(solid(Color::BLUE, Size::new(30.0, 30.0)))]);

    let painted = painted(&drawn(Size::new(200.0, 200.0), stack), Color::BLUE);
    assert_eq!(painted.left, 50.0);
    assert_eq!(painted.top, 0.0);
    assert_eq!(painted.right - painted.left, 30.0, "not stretched to fill");
}

#[test]
fn an_axis_with_neither_edge_falls_back_to_the_stacks_alignment() {
    // Only `top` given: horizontally this child behaves like an ordinary,
    // un-positioned child under the stack's own alignment — centred here.
    // Both axes are loose (one edge each, or none), so a `SizedBox` child.
    let stack = Stack::new()
        .alignment(Alignment::CENTER)
        .children(children![Positioned::new()
            .top(0.0)
            .child(solid(Color::BLUE, Size::new(40.0, 20.0))),]);

    let painted = painted(&drawn(Size::new(200.0, 200.0), stack), Color::BLUE);
    assert_eq!(painted.top, 0.0, "the edge that was given");
    assert_eq!(
        painted.left,
        (200.0 - 40.0) / 2.0,
        "centred horizontally, same as an un-positioned child would be"
    );
}

#[test]
fn right_and_bottom_measure_from_the_far_edge() {
    // One edge per axis again: loose both ways, so a `SizedBox` child.
    let stack = Stack::new().children(children![Positioned::new()
        .right(10.0)
        .bottom(20.0)
        .child(solid(Color::BLUE, Size::new(30.0, 15.0)))]);

    let painted = painted(&drawn(Size::new(200.0, 200.0), stack), Color::BLUE);
    assert_eq!(painted.right, 190.0, "200 - 10");
    assert_eq!(painted.bottom, 180.0, "200 - 20");
}

#[test]
fn fill_stretches_to_every_edge() {
    // `fill()` sets all four edges: both axes tight to the full stack size,
    // so a bare `ColoredBox` reports it correctly.
    let stack = Stack::new().children(children![
        Positioned::fill().child(ColoredBox::new(Color::BLUE))
    ]);

    let painted = painted(&drawn(Size::new(120.0, 80.0), stack), Color::BLUE);
    assert_eq!(painted, Rect::new(0.0, 0.0, 120.0, 80.0));
}

#[test]
fn a_positioned_child_does_not_affect_the_stacks_own_size() {
    // A 300-wide positioned child must not make a 100-wide stack grow to fit
    // it — only the un-positioned child decides the stack's size, which is
    // then what the positioned child is placed (and here, clipped) against.
    let stack = Stack::new().children(children![
        SizedBox::from_size(Size::new(100.0, 100.0)),
        Positioned::new()
            .left(0.0)
            .top(0.0)
            .width(300.0)
            .child(ColoredBox::new(Color::BLUE)),
    ]);

    // Measured through a marker that shrink-wraps whatever the stack settles
    // on. The outer `Stack` gives the inner one loose rather than tight
    // constraints — the same reason the collapse test below nests one too —
    // so its size actually comes from the sizing pass rather than being
    // forced to whatever the root happens to be.
    let ground = Stack::new().children(children![ColoredBox::new(Color::RED).child(stack)]);
    let driver = drawn(Size::new(500.0, 500.0), ground);
    let red = painted(&driver, Color::RED);
    assert_eq!(
        (red.right - red.left, red.bottom - red.top),
        (100.0, 100.0),
        "the stack sized itself from the un-positioned child alone"
    );
}

#[test]
fn a_stack_of_only_positioned_children_takes_the_largest_size_allowed() {
    let stack = Stack::new().children(children![Positioned::new()
        .right(0.0)
        .bottom(0.0)
        .width(50.0)
        .height(50.0)
        .child(ColoredBox::new(Color::BLUE))]);

    // Wrapped so the stack itself is under loose rather than tight
    // constraints — otherwise every stack in this file would report the
    // root's tight size regardless, and this test would pass without the
    // no-unpositioned-children rule doing anything.
    let ground = Center::new().child(stack);
    let driver = drawn(Size::new(500.0, 500.0), ground);

    // With nothing un-positioned to measure against, the stack takes the
    // largest its constraints allow rather than collapsing — 500x500 here.
    // A child pinned to the far corner therefore lands against the far
    // corner of the surface, which is what makes the size observable at all.
    // (An *unbounded* axis is the one case that falls back to the minimum
    // instead; see `RenderStack::size_without_unpositioned`.)
    let blue = painted(&driver, Color::BLUE);
    assert_eq!(
        (blue.left, blue.top, blue.right, blue.bottom),
        (450.0, 450.0, 500.0, 500.0)
    );
}
