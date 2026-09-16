//! Filters, all the way to pixels.
//!
//! ```console
//! cargo test -p vieww --features native --test filter_to_pixels
//! ```
//!
//! The gap this closes had a very specific shape: `vieww-effects` shipped
//! correct, tested blur and colour-matrix kernels, and widget wrappers that
//! never called them. `BackdropBlur` painted a flat tint; `FilterChain` painted
//! `Opacity(0.7)` whatever was in the chain. Both were honestly documented as
//! degraded paths, and both compiled, so every test passed.
//!
//! A unit test on the kernels could not have caught that, because the kernels
//! were never wrong. **Only a test that reads back real pixels can**, which is
//! what all of these do.
//!
//! This used to be split from a `filter_to_pixels_gpu.rs` that cross-checked a
//! second, independently-implemented backend against this one. That backend
//! (vello's) is gone — vieww's own rasterizer is this crate's one and only
//! renderer now, so there is nothing left to cross-check against — but four of
//! that file's tests asserted something this file did not: that the filter
//! path actually runs (`filtered_layers`), that content after a filtered layer
//! stacks correctly, and that a filtered layer's own opacity composites right.
//! Those four moved here rather than being lost with the file.

#![cfg(feature = "native")]

use vieww::foundation::{Color, Size};
use vieww::paint::native::{NativeRenderer, SceneReport};
use vieww::prelude::*;
use vieww_render::FrameDriver;

const W: f32 = 120.0;
const H: f32 = 120.0;

/// Render a tree and hand back straight RGBA8 for the whole surface.
fn pixels(root: WidgetNode) -> Vec<u8> {
    render(root).0
}

/// [`pixels`], plus the report — for the tests that need to know what the
/// renderer actually did, not just what it drew.
fn render(root: WidgetNode) -> (Vec<u8>, SceneReport) {
    let mut driver = FrameDriver::new(Size::new(W, H));
    let mut renderer = NativeRenderer::new();
    driver.elements().set_root(root);
    driver.draw_frame();
    let (pixels, report) = renderer
        .render_to_pixels(driver.scene(), W as u32, H as u32, Color::WHITE)
        .expect("rasterising");
    (pixels.data().to_vec(), report)
}

fn at(buffer: &[u8], x: u32, y: u32) -> (u8, u8, u8, u8) {
    let index = ((y * W as u32 + x) * 4) as usize;
    (
        buffer[index],
        buffer[index + 1],
        buffer[index + 2],
        buffer[index + 3],
    )
}

/// A hard-edged square, centred, on the white page.
fn square(color: Color) -> WidgetNode {
    Container::new()
        .color(Color::WHITE)
        .padding(EdgeInsets::all(30.0))
        .child(
            Container::new()
                .color(color)
                .child(SizedBox::from_size(Size::new(60.0, 60.0))),
        )
        .into()
}

/// The headline claim. A blurred edge must not be a step function.
///
/// Sampled just outside the square's boundary: unblurred that pixel is pure
/// page white, blurred it has picked up some of the square.
#[test]
fn a_blur_softens_an_edge_that_was_hard() {
    let ink = Color::rgb(0, 0, 0);

    let sharp = pixels(square(ink));
    let blurred = pixels(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(30.0))
            .child(
                Filtered::blur(4.0).child(
                    Container::new()
                        .color(ink)
                        .child(SizedBox::from_size(Size::new(60.0, 60.0))),
                ),
            )
            .into(),
    );

    // Four pixels outside the square's left edge, on its centre line.
    let (r, _, _, _) = at(&sharp, 26, 60);
    assert_eq!(r, 255, "unblurred, this pixel is page white");

    let (r, g, b, _) = at(&blurred, 26, 60);
    assert!(
        r < 250 && g < 250 && b < 250,
        "the blur must have reached outside the square, got ({r},{g},{b})"
    );

    // And the centre must still be dark — a blur spreads, it does not erase.
    let (r, _, _, _) = at(&blurred, 60, 60);
    assert!(r < 80, "the middle of a blurred square is still dark: {r}");
}

/// A blur has to *fall off*. A uniform grey wash over the region would pass the
/// test above and be nothing like a blur.
#[test]
fn a_blur_falls_off_with_distance() {
    let blurred = pixels(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(30.0))
            .child(
                Filtered::blur(5.0).child(
                    Container::new()
                        .color(Color::rgb(0, 0, 0))
                        .child(SizedBox::from_size(Size::new(60.0, 60.0))),
                ),
            )
            .into(),
    );

    let near = at(&blurred, 27, 60).0;
    let far = at(&blurred, 20, 60).0;
    assert!(
        near < far,
        "nearer the edge must be darker: {near} at x=27 vs {far} at x=20"
    );
}

/// The `FilterChain(grayscale)` case, which used to render a washed-out but
/// still perfectly colourful square.
#[test]
fn a_grayscale_filter_actually_removes_the_colour() {
    let vivid = Color::rgb(220, 40, 40);

    let unfiltered = pixels(square(vivid));
    let (r, g, _, _) = at(&unfiltered, 60, 60);
    assert!(r > 200 && g < 60, "the source really is red: ({r},{g})");

    let filtered = pixels(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(30.0))
            .child(
                Filtered::new().grayscale().child(
                    Container::new()
                        .color(vivid)
                        .child(SizedBox::from_size(Size::new(60.0, 60.0))),
                ),
            )
            .into(),
    );

    let (r, g, b, _) = at(&filtered, 60, 60);
    assert!(
        r.abs_diff(g) <= 2 && g.abs_diff(b) <= 2,
        "grayscale must equalise the channels, got ({r},{g},{b})"
    );
    // And it must be luminance-weighted rather than a flat mean: the mean of
    // (220,40,40) is 100, the Rec.709 luminance is about 76.
    assert!(r < 95, "a flat-mean grey rather than luminance: {r}");
}

/// Both at once, on one layer — the frosted-glass case.
#[test]
fn a_blur_and_a_colour_matrix_compose_on_one_layer() {
    let vivid = Color::rgb(40, 90, 220);
    let frosted = pixels(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(30.0))
            .child(
                Filtered::blur(3.0).grayscale().child(
                    Container::new()
                        .color(vivid)
                        .child(SizedBox::from_size(Size::new(60.0, 60.0))),
                ),
            )
            .into(),
    );

    let (r, g, b, _) = at(&frosted, 60, 60);
    assert!(
        r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3,
        "the colour matrix ran: ({r},{g},{b})"
    );
    let (edge, _, _, _) = at(&frosted, 27, 60);
    assert!(edge < 250, "and so did the blur: {edge}");
}

/// The regression guard for the whole class of bug. A no-op filter must leave
/// the pixels **exactly** where an unfiltered tree put them — otherwise every
/// `Opacity` in the tree starts paying for an offscreen buffer it does not use.
#[test]
fn a_noop_filter_changes_nothing_at_all() {
    let plain = pixels(square(Color::rgb(30, 120, 60)));
    let wrapped = pixels(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(30.0))
            .child(
                Filtered::new().child(
                    Container::new()
                        .color(Color::rgb(30, 120, 60))
                        .child(SizedBox::from_size(Size::new(60.0, 60.0))),
                ),
            )
            .into(),
    );

    assert_eq!(plain, wrapped, "a no-op filter must be bit-identical");
}

/// The blur has to reach *outside* the layer's own box, or a blurred panel is
/// cut off square at its edge — the most recognisable way a blur is wrong.
#[test]
fn a_blur_is_not_clipped_to_the_box_it_came_from() {
    let blurred = pixels(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(30.0))
            .child(
                Filtered::blur(6.0).child(
                    Container::new()
                        .color(Color::rgb(0, 0, 0))
                        .child(SizedBox::from_size(Size::new(60.0, 60.0))),
                ),
            )
            .into(),
    );

    // The square occupies x = 30..90. A blur clipped to that box would leave
    // x = 22 at pure white; an unclipped one reaches it.
    let (r, _, _, _) = at(&blurred, 22, 60);
    assert!(
        r < 255,
        "the blur was clipped to its own bounds — it must expand by 3 sigma"
    );
}

/// The two widgets from `vieww-effects` that used to be stubs, now going
/// through the same path.
#[test]
fn the_effects_crate_widgets_reach_the_pixels_now() {
    let vivid = Color::rgb(220, 40, 40);

    let grey = pixels(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(30.0))
            .child(
                vieww::effects::FilterChain::new()
                    .filter(vieww::effects::Filter::grayscale())
                    .child(
                        Container::new()
                            .color(vivid)
                            .child(SizedBox::from_size(Size::new(60.0, 60.0))),
                    ),
            )
            .into(),
    );

    let (r, g, b, _) = at(&grey, 60, 60);
    assert!(
        r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3,
        "FilterChain(grayscale) must be grey, not a 0.7 opacity red: ({r},{g},{b})"
    );

    // # Why the backdrop probe needs something behind it
    //
    // This assertion used to sit on a white page with a black square inside
    // the blur, and check that a pixel *outside* the square was no longer
    // pure white. It could never hold, for a reason worth writing down: a
    // backdrop filter's subject is the destination already painted beneath
    // it, that destination was uniform white, and blurring white gives white.
    // The square is not part of the backdrop — it paints *after* the blur, by
    // design, which is the whole difference between a backdrop filter and an
    // ordinary one.
    //
    // So the test proved nothing and failed anyway. What discriminates is a
    // hard edge *behind* the layer: a real backdrop blur softens it into a
    // ramp, and every degraded stand-in — a flat tint, a plain opacity, a
    // dropped layer — leaves it a step.
    let softened = pixels(
        Stack::new()
            .fit(StackFit::Expand)
            .children(children![
                Positioned::new()
                    .left(0.0)
                    .top(0.0)
                    .bottom(0.0)
                    .width(60.0)
                    .child(Container::new().color(Color::rgb(0, 0, 0))),
                Positioned::new()
                    .left(20.0)
                    .top(20.0)
                    .right(20.0)
                    .bottom(20.0)
                    .child(vieww::effects::BackdropBlur::new(
                        vieww::effects::BackdropFilter::frosted(6.0),
                    )),
            ])
            .into(),
    );

    // Across the step at x = 60. A sharp edge reads 0 then 255; a blurred one
    // climbs through the middle.
    let (left, _, _, _) = at(&softened, 50, 60);
    let (middle, _, _, _) = at(&softened, 60, 60);
    let (right, _, _, _) = at(&softened, 70, 60);
    assert!(
        left > 0 && left < middle && middle < right && right < 255,
        "BackdropBlur must soften what is behind it, not leave a step: \
         50px={left}, 60px={middle}, 70px={right}"
    );

    // And the panel must exist at all when it has no children. A backdrop
    // filter draws nothing of its own — that is what it is — so the
    // empty-group rule that drops a layer nothing painted into used to delete
    // exactly the case people write most often.
    let empty_panel_worked = at(&softened, 30, 60).0 > 0;
    assert!(
        empty_panel_worked,
        "a BackdropBlur with no child must still filter its backdrop"
    );
}

/// The regression guard, moved from the old GPU-vs-CPU parity file: an
/// unfiltered frame must not touch the filter path at all — one extra
/// offscreen pass and composite per frame, on every screen in every
/// application, would be a real cost for a feature most frames never use.
#[test]
fn an_unfiltered_frame_does_not_enter_the_filter_path() {
    let plain: WidgetNode = Container::new()
        .color(Color::WHITE)
        .padding(EdgeInsets::all(32.0))
        .child(
            Container::new()
                .color(Color::rgb(30, 120, 60))
                .child(SizedBox::from_size(Size::new(64.0, 64.0))),
        )
        .into();

    let (_, report) = render(plain);
    assert_eq!(report.filtered_layers, 0);
}

/// Segmentation must not reorder anything. Content drawn *after* a filtered
/// layer has to land on top of it — the most likely way a segmented compositor
/// goes wrong, and invisible in any test with only one thing on screen.
#[test]
fn content_after_a_filtered_layer_draws_over_it() {
    let tree: WidgetNode = Container::new()
        .color(Color::WHITE)
        .child(
            Stack::new()
                .push(
                    Filtered::blur(6.0).child(
                        Container::new()
                            .color(Color::rgb(0, 0, 0))
                            .child(SizedBox::from_size(Size::new(64.0, 64.0))),
                    ),
                )
                // A hard red band across the middle, drawn after the blur.
                .push(
                    Positioned::new().left(0.0).top(28.0).child(
                        Container::new()
                            .color(Color::rgb(255, 0, 0))
                            .child(SizedBox::from_size(Size::new(64.0, 8.0))),
                    ),
                ),
        )
        .into();

    let (pixels, report) = render(tree);
    assert_eq!(report.filtered_layers, 1);

    let (r, g, b, _) = at(&pixels, 32, 32);
    assert!(
        r > 200 && g < 60 && b < 60,
        "the band drawn after the filter must be on top and unblurred: ({r},{g},{b})"
    );
}

/// A filtered layer's own opacity is applied when the result is composited, so
/// a half-transparent blurred panel must let the page through.
#[test]
fn a_filtered_layers_group_opacity_is_honoured() {
    let tree: WidgetNode = Container::new()
        .color(Color::WHITE)
        .child(
            Opacity::new(0.5).child(
                Filtered::blur(2.0).child(
                    Container::new()
                        .color(Color::rgb(0, 0, 0))
                        .child(SizedBox::from_size(Size::new(64.0, 64.0))),
                ),
            ),
        )
        .into();

    let (pixels, _) = render(tree);
    let (r, _, _, _) = at(&pixels, 32, 32);
    assert!(
        (100..=160).contains(&r),
        "black at half opacity over white should be mid-grey, got {r}"
    );
}
