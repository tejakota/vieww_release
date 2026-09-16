//! # Vieww web-backend certification
//!
//! One deterministic scene, two targets, one claim to hold: **the browser
//! canvas and the native rasteriser produce the same bytes.**
//!
//! ```console
//! cargo run --release -p test-web --example baseline -- /tmp/web-cert
//! apps/../examples/test-web/build-web.sh /tmp/web-cert/dist
//! ```
//!
//! # Why byte-for-byte and not "looks the same"
//!
//! `vieww-platform-web` presents `vieww-paint`'s CPU rasteriser — the *same*
//! code, compiled for wasm, drawing the *same* scene commands into the *same*
//! straight-alpha RGBA8 buffer, which `put_image_data` then hands to a 2D
//! canvas. There is no GPU, no driver, no compositor and no fontconfig
//! between the two: the fonts are the embedded set on both sides, the AA mode
//! is the default on both sides, and at `devicePixelRatio` 1 the scene is
//! presented unscaled ([`Scene::scaled`] short-circuits at 1.0). If the
//! pipeline is honest, the buffers are *equal* — and anything less than
//! equal is a real difference worth finding, not noise.
//!
//! # What the scene puts in the frame on purpose
//!
//! Every family of command the wasm path has to carry:
//!
//! * two gradient geometries (linear, radial) — the Phase-1 gradient work;
//! * a shadowed rounded card — the Phase-1 shadow path;
//! * a translucent, blurred, circle-clipped layer over a gradient, a ring,
//!   and a rotated group — layers, filters, shaped clips, transforms;
//! * Latin text at three sizes plus a bold weight, a monospace line, and a
//!   mixed Latin + CJK line — the Phase-2 embedded fallback chain, on wasm;
//! * a live counter — the tap target that proves pointer events reach the
//!   tree: `GestureDetector` has no hover-driven paint, so the frame after a
//!   tap differs from the frame before it *only* by the counter, and the
//!   "tapped" frame can be compared byte-for-byte against a native render of
//!   the tree at `taps = 1`.
//!
//! Nothing in the tree animates or reads a clock, which is what makes both
//! states stable enough to compare.
//!
//! # The two halves
//!
//! * `examples/baseline.rs` — the host half: renders `taps = 0` and `taps = 1`
//!   to PNG (for the eye) and raw RGBA (for the byte comparison).
//! * the `wasm32` module at the bottom of this file — the browser half:
//!   mounts the same tree on a `<canvas id="vieww-canvas">` through
//!   `vieww-platform-web`.
//!
//! [`Scene::scaled`]: vieww::paint::Scene::scaled

use vieww::prelude::*;
use vieww_foundation::{
    Color, FontFamily, FontWeight, Gradient, Offset, Path, Shadow, Size, Transform,
};

/// The canvas both halves render. Fixed, not `fill_window`, because a
/// certification scene that changed with the browser window could not be
/// compared against a native render of a fixed size.
pub const WIDTH: f32 = 960.0;
pub const HEIGHT: f32 = 600.0;

/// The ground both halves paint first — the `background` of the web mount and
/// the `base` of the native render, and it must be the same value in both or
/// the comparison measures the difference between two backgrounds.
pub const BG: Color = Color::rgb(0x14, 0x16, 0x1C);

// ─── palette ──────────────────────────────────────────────────────────────
//
// Fixed constants, not theme lookups: a certification scene that inherited a
// theme could drift when the theme's defaults change, and the drift would
// read as a backend difference.

const INK: Color = Color::rgb(0xF2, 0xF4, 0xF8);
const INK_2: Color = Color::rgb(0x9A, 0xA3, 0xB5);
const ACCENT: Color = Color::rgb(0x8E, 0xB8, 0xFF);
const CARD: Color = Color::rgb(0xF4, 0xF5, 0xF7);

/// The swatches are one `Flex::row` of four fixed squares, so the whole row
/// is `4 * SWATCH + 3 * GAP` wide: 4 * 186 + 3 * 20 = 804, at `LEFT` from the
/// left edge it ends 84 from the right — a composition that does not depend
/// on the window to lay out.
const SWATCH: f32 = 186.0;
const GAP: f32 = 20.0;
const LEFT: f32 = 48.0;

/// The tree. `taps` is the signal the `GestureDetector` writes and the
/// counter reads — the one piece of state, so the two certified states are
/// `taps = 0` and `taps = 1` and nothing else can differ between them.
pub fn screen(taps: Signal<i32>) -> WidgetNode {
    Screen { taps }.into()
}

/// The whole scene is one tap target, deliberately: the certification clicks
/// the *canvas*, not a control, and a `GestureDetector` (unlike a `Button`)
/// paints nothing for hover or press — so wherever the pointer ends up, the
/// pixels are the tree at `taps`, and the comparison stays exact.
#[derive(Debug)]
struct Screen {
    taps: Signal<i32>,
}

#[widget]
impl Screen {
    fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
        let taps = self.taps.get();
        let bump = self.taps.clone();
        GestureDetector::new()
            .on_tap(move |_tap| bump.update(|n| *n += 1))
            .child(
                Stack::new()
                    .push(Positioned::fill().child(background()))
                    .push(Positioned::new().left(LEFT).top(40.0).child(header()))
                    .push(Positioned::new().left(LEFT).top(238.0).child(swatch_row()))
                    .push(Positioned::new().left(LEFT).bottom(40.0).child(footer()))
                    .push(Positioned::new().right(LEFT).top(44.0).child(counter(taps))),
            )
    }
}

/// The ground: a radial wash, so the frame carries a gradient before any
/// swatch is reached — and so a backend that dropped the background fill
/// shows a flat dark page instead of this ramp.
fn background() -> WidgetNode {
    Container::new()
        .gradient(Gradient::radial(Offset::new(0.5, 0.18), 1.1).with_stops(&[
            (0.0, Color::rgb(0x1E, 0x22, 0x30)),
            (0.6, BG),
            (1.0, Color::rgb(0x0D, 0x0E, 0x13)),
        ]))
        .child(SizedBox::from_size(Size::new(WIDTH, HEIGHT)))
        .into()
}

/// The headline block: Latin at two sizes, a bold weight, and the mixed
/// Latin + CJK line — every character of the CJK is one the embedded subset
/// already draws in `test-text-fidelity`'s certified fallback screen.
fn header() -> WidgetNode {
    Flex::column()
        .children(children![
            Text::new("vieww on the web")
                .size(30.0)
                .weight(FontWeight::Bold)
                .color(INK),
            SizedBox::height(10.0),
            Text::new("one deterministic scene — the browser canvas and the native render must agree byte for byte")
                .size(14.0)
                .color(INK_2),
            SizedBox::height(16.0),
            Text::new("同一行 Latin 与 CJK 混排，两种字面，一种度量。")
                .size(17.0)
                .color(INK),
            SizedBox::height(6.0),
            Text::new("the embedded fallback chain, shaped on wasm")
                .size(12.0)
                .color(INK_2),
        ])
        .into()
}

/// Four fixed squares, each holding one family of scene command.
fn swatch_row() -> WidgetNode {
    Flex::row()
        .spacing(GAP)
        .children(children![
            // 1. A three-stop linear ramp, rounded.
            Container::new()
                .gradient(Gradient::vertical().with_stops(&[
                    (0.0, Color::rgb(0x66, 0x7E, 0xEA)),
                    (0.55, Color::rgb(0x76, 0x4B, 0xA2)),
                    (1.0, Color::rgb(0x2B, 0x1A, 0x4A)),
                ]),)
                .radius(14.0)
                .child(SizedBox::from_size(Size::square(SWATCH))),
            // 2. A radial ramp, rounded the same way.
            Container::new()
                .gradient(Gradient::radial_fill().with_stops(&[
                    (0.0, Color::rgb(0xF6, 0xD3, 0x65)),
                    (1.0, Color::rgb(0xFD, 0xA0, 0x85)),
                ]),)
                .radius(14.0)
                .child(SizedBox::from_size(Size::square(SWATCH))),
            // 3. The shadow card: a light rounded box over the dark ground,
            //    lifted by a soft shadow — an offscreen blur per frame.
            Container::new()
                .color(CARD)
                .radius(14.0)
                .shadow(Shadow::new(
                    Color::rgba(0, 0, 0, 90),
                    Offset::new(0.0, 10.0),
                    24.0,
                ))
                .alignment(Alignment::CENTER)
                .child(
                    Text::new("shadow")
                        .size(13.0)
                        .color(Color::rgb(0x3A, 0x40, 0x50))
                )
                .size(SWATCH, SWATCH),
            // 4. The painter's square: a blurred, circle-clipped translucent
            //    layer over a ramp; a ring; a rotated group. Layers, filters,
            //    shaped clips and transforms in one 186x186 box.
            Painting::sized(
                Size::square(SWATCH),
                PaintWith::new(|book: &mut vieww_foundation::Sketchbook, size: Size| {
                    let w = size.width;
                    let h = size.height;
                    let centre = Offset::new(w * 0.5, h * 0.5);
                    // The clipped, blurred, translucent layer: two coloured
                    // discs behind a circle clip, at 0.8 alpha with a 4px
                    // blur — three offscreen paths at once.
                    let clip = Path::arc(centre, w * 0.46, 0.0, std::f32::consts::TAU);
                    book.layer(0.8, 4.0, Some(clip), |inner| {
                        inner.circle(
                            Offset::new(w * 0.36, h * 0.40),
                            w * 0.28,
                            Color::rgb(0x7E, 0x5C, 0xE8),
                        );
                        inner.circle(
                            Offset::new(w * 0.64, h * 0.58),
                            w * 0.28,
                            Color::rgb(0x38, 0xB8, 0x9E),
                        );
                    });
                    // A ring on top, crisp.
                    book.ring(centre, w * 0.40, 2.5, Color::rgba(255, 255, 255, 210));
                    // And a rotated group: a square outline standing on its
                    // corner, proving a transform survives the wasm path.
                    book.transformed(Transform::rotate(std::f32::consts::FRAC_PI_4), |inner| {
                        let half = w * 0.30;
                        let side = half * 2.0;
                        inner.stroke_rrect(
                            Rect::new(-half, -half, -half + side, -half + side),
                            3.0,
                            Color::rgb(0xF6, 0xD3, 0x65),
                            1.5,
                        );
                    });
                }),
            ),
        ])
        .into()
}

/// The caption under the swatches: the monospace embedded face, which is a
/// different physical font from the sans one above it.
fn footer() -> WidgetNode {
    Text::new(
        "embedded DejaVu sans + mono + CJK · gradients · shadow · layer + blur + clip · transform",
    )
    .style(
        TextStyle::new(13.0)
            .family(FontFamily::Monospace)
            .color(INK_2),
    )
    .into()
}

/// The live counter, top right: the one thing in the frame that changes, and
/// the reason the scene carries a pointer pipeline at all.
fn counter(taps: i32) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::End)
        .children(children![
            Text::new(format!("taps: {taps}"))
                .size(26.0)
                .weight(FontWeight::Bold)
                .color(ACCENT),
            SizedBox::height(4.0),
            Text::new("tap anywhere").size(12.0).color(INK_2),
        ])
        .into()
}

// ─── the browser half ─────────────────────────────────────────────────────

/// The wasm entry. `#[wasm_bindgen(start)]` runs at instantiation, so the
/// page's `init()` promise resolving is also "the tree is mounted and the
/// first frame is in flight" — which is what the certification page waits on
/// before it starts polling the canvas.
///
/// Top-level, like `viewwsite`'s entry and for the same reason: this *is*
/// the crate's public wasm API, and a `pub fn` in a private module is
/// `unreachable_pub` to everything but the attribute that reads it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    #[cfg(target_arch = "wasm32")]
    use vieww_platform_web::{WebApp, WebSurface};

    // `by_id` starts from the canvas's attribute size (960x600 in the page,
    // matching `WIDTH`/`HEIGHT`); `resize` then applies the device-pixel
    // ratio to the backing buffer. At the certification's `devicePixelRatio`
    // 1 the two agree, and the comparison is exact.
    let mut surface = WebSurface::by_id("vieww-canvas")
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    let _ = surface.resize(Size::new(WIDTH, HEIGHT));
    WebApp::new(surface)
        .background(BG)
        .mount_with(|driver| {
            // No `set_fonts`: the embedded set is the deterministic choice,
            // and it is what the native baseline uses too.
            let runtime = driver.elements().runtime().clone();
            let taps = runtime.signal(0_i32);
            driver.set_root(screen(taps));
        })
        // The handle is forgotten rather than kept: the page owns the
        // application for as long as the tab lives, the same decision
        // `apps/viewwsite`'s demo island makes.
        .map(std::mem::forget)
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    Ok(())
}
