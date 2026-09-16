//! Translucent colours, all the way to pixels.
//!
//! # Why this test exists
//!
//! The studio draws a one-pixel highlight along the top edge of every raised
//! surface at *twenty of 255* — barely there, which is the point. It arrived on
//! screen as a pure white line. On an LCD a full-strength white hairline against
//! near-black picks up subpixel colour fringing, so the bug was reported as
//! *"yellow horizontal lines between the panes"*, which is a sentence with no
//! obvious connection to the alpha channel.
//!
//! Nothing above the renderer could have caught it. The widget holds the right
//! colour, layout puts it in the right place, and the scene carries the right
//! four bytes — the test that would have failed is one that looks at a pixel,
//! and every pixel test in this workspace happened to sample an opaque one.
//!
//! So this samples translucent ones, at every step where an alpha can be
//! dropped: a bare fill, a fill inside a clip, a fill inside a layer, and the
//! same colour reached through a gradient stop.
//!
//! ```console
//! cargo test -p vieww --features native --test translucent_to_pixels
//! ```

#![cfg(feature = "native")]

use vieww::foundation::{Color, Size};
use vieww::prelude::*;

const SIDE: u32 = 40;

/// What a colour at `alpha` over `under` should composite to, per channel.
fn expected(over: u8, under: u8, alpha: u8) -> u8 {
    let a = f32::from(alpha) / 255.0;
    let value = f32::from(over).mul_add(a, f32::from(under) * (1.0 - a));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a channel, 0..=255 by construction"
    )]
    let rounded = (value + 0.5) as u8;
    rounded
}

/// The colour at the centre of a `SIDE`-square window holding `root`.
fn centre(root: impl Into<WidgetNode>) -> (u8, u8, u8) {
    let mut driver = FrameDriver::new(Size::square(SIDE as f32));
    driver.elements().set_root(root);
    driver.draw_frame();

    let mut renderer = vieww::paint::native::NativeRenderer::new();
    let (pixels, _) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::BLACK)
        .expect("rasterising");
    // `Pixels::pixel` reads straight, not premultiplied, alpha. Everything here
    // sits on an opaque ground so the composite is opaque and the two agree —
    // but the check is written out rather than assumed, because reading
    // premultiplied bytes as straight ones is exactly the sort of thing that
    // would report this bug fixed while it was still there.
    let pixel = pixels.pixel(SIDE / 2, SIDE / 2);
    assert_eq!(pixel.a, 255, "the sampled pixel is not opaque");
    (pixel.r, pixel.g, pixel.b)
}

/// A dark ground with `child` over it, the way a panel sits on the window.
///
/// Both are given an explicit size. A `Stack`'s non-positioned children are
/// laid out *loose*, so a `Container` with a colour and no size collapses to
/// nothing there — which is a real property of the layout model and, on the
/// first draft of this file, four tests that failed for the wrong reason.
fn over_ground(child: impl Into<WidgetNode>) -> WidgetNode {
    let side = SIDE as f32;
    Container::new()
        .color(Color::hex(0x18_1818))
        .size(side, side)
        .child(SizedBox::square(side).child(child.into()))
        .into()
}

/// Assert a grey, allowing the one step a rounding difference can cost.
///
/// Not exact: `Color::over` rounds half up in `f32` and the rasteriser has its
/// own path to eight bits. What this is checking is the difference between 42
/// and 255, and a test that also insisted on 42-not-43 would be a test that
/// went red the next time either rounded differently.
#[track_caller]
fn assert_grey(found: (u8, u8, u8), want: u8, what: &str) {
    let off = |value: u8| (i16::from(value) - i16::from(want)).abs() > 1;
    assert!(
        !(off(found.0) || off(found.1) || off(found.2)),
        "{what}: got {found:?}, wanted about ({want}, {want}, {want})"
    );
}

#[test]
fn a_translucent_fill_composites_rather_than_covering() {
    let found = centre(over_ground(
        Container::new().color(Color::rgba(255, 255, 255, 20)),
    ));
    assert_grey(found, expected(255, 0x18, 20), "a white veil at 20/255");
}

#[test]
fn a_translucent_fill_inside_a_clip_keeps_its_alpha() {
    // The studio's rim is inside `Clip::rounded`, which is a layer.
    let found = centre(over_ground(
        Clip::rounded(4.0).child(Container::new().color(Color::rgba(255, 255, 255, 20))),
    ));
    assert_grey(found, expected(255, 0x18, 20), "a veil inside a clip");
}

#[test]
fn a_translucent_fill_inside_an_opacity_layer_keeps_its_alpha() {
    let found = centre(over_ground(
        Opacity::new(0.5).child(Container::new().color(Color::rgba(255, 255, 255, 40))),
    ));
    // Two alphas multiply: 40/255 at half strength is 20/255.
    assert_grey(found, expected(255, 0x18, 20), "a veil inside an opacity");
}

#[test]
fn a_translucent_gradient_stop_keeps_its_alpha() {
    // The sheen is a gradient whose top stop is a veil over the base. If a
    // gradient forced its stops opaque the sheen would be a white band.
    let veil = Color::rgba(255, 255, 255, 20);
    let found = centre(over_ground(
        Container::new().gradient(vieww::foundation::Gradient::vertical().between(veil, veil)),
    ));
    assert_grey(found, expected(255, 0x18, 20), "a translucent gradient");
}
