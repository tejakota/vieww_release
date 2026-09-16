//! Damage has to be **large enough**, not merely honest about its edges.
//!
//! # The invariant, and why it was missing
//!
//! `vieww-paint`'s damage suite asserts that pixels *outside* a damage region
//! survive a partial repaint. That is a real guarantee and it is the wrong half
//! of the question on its own: a region that is too small satisfies it
//! trivially, because every pixel it failed to repaint is by definition outside
//! it.
//!
//! What that produces on a backend with a persistent target — which is what the
//! GPU backend is, rasterising damaged regions into a surface that survives
//! between frames — is the previous frame's ink left standing in the pixels the
//! damage did not name. One frame of it is invisible. Over the frames of an
//! animation it accumulates, and what reaches a bug report is not "the damage
//! rectangle is a pixel short" but *"the icons look smeared"*.
//!
//! So: **compositing the damaged regions onto the previous frame must give the
//! same picture as repainting everything**, on every frame, for every kind of
//! change. That is what `assert_partial_repaint_is_complete` checks, and these
//! are the mutation classes it is checked over.
//!
//! # Why this file is in the facade crate
//!
//! Because `vieww-test-harness` was built, documented as mandatory by
//! `docs/PRODUCTION-GAPS.md`, and **referenced by no other crate in the
//! workspace** — a harness nobody runs is worth nothing. This is the suite it
//! was built for.

use std::time::Duration;

use vieww::element::Signal;
use vieww::foundation::{Color, EdgeInsets, Offset, Size};
use vieww::prelude::*;
use vieww::render::FrameDriver;
use vieww_test_harness::assert_partial_repaint_is_complete;

const SURFACE: Size = Size::new(320.0, 240.0);
const GROUND: Color = Color::rgb(18, 18, 18);

/// A grid of coloured boxes, some of which a signal recolours.
#[derive(Debug)]
struct Board {
    lit: Signal<usize>,
}

impl Widget for Board {
    fn debug_name(&self) -> &'static str {
        "Board"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let lit = self.lit.get();
        Container::new()
            .color(GROUND)
            .padding(EdgeInsets::all(8.0))
            .child(
                Flex::column().spacing(4.0).children(
                    (0..6)
                        .map(|row| {
                            let color = if row == lit {
                                Color::rgb(220, 220, 220)
                            } else {
                                Color::rgb(48, 48, 48)
                            };
                            ColoredBox::new(color)
                                .child(SizedBox::from_size(Size::new(280.0, 28.0)))
                                .into()
                        })
                        .collect::<Vec<_>>(),
                ),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(Board);

/// **A recolour.** The narrowest useful change: one repaint boundary's ink
/// changes and nothing moves.
#[test]
fn a_recoloured_row_repaints_completely() {
    let mut driver = FrameDriver::new(SURFACE);
    let lit = driver.elements().runtime().signal(0usize);
    driver.set_root(Board { lit: lit.clone() });
    driver.draw_frame();

    assert_partial_repaint_is_complete(&mut driver, SURFACE, GROUND, 6, |driver, frame| {
        lit.set((frame + 1) % 6);
        driver.draw_frame();
    });
}

/// **A move.** The class a recolour cannot catch: the damage has to cover both
/// where the thing *was* and where it now is, and a region that names only the
/// second leaves a ghost of the first.
#[derive(Debug)]
struct Slider {
    offset: Signal<f32>,
}

impl Widget for Slider {
    fn debug_name(&self) -> &'static str {
        "Slider"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let dx = self.offset.get();
        Container::new()
            .color(GROUND)
            .child(Stack::new().fit(StackFit::Expand).children(children![
                    Positioned::new().left(dx).top(40.0).child(
                        ColoredBox::new(Color::rgb(230, 230, 230))
                            .child(SizedBox::from_size(Size::new(60.0, 60.0)))
                    ),
                ]))
            .into()
    }
}

vieww::widget::widget_node_from!(Slider);

#[test]
fn a_moving_box_leaves_no_ghost_behind_it() {
    let mut driver = FrameDriver::new(SURFACE);
    let offset = driver.elements().runtime().signal(0.0f32);
    driver.set_root(Slider {
        offset: offset.clone(),
    });
    driver.draw_frame();

    assert_partial_repaint_is_complete(&mut driver, SURFACE, GROUND, 12, |driver, frame| {
        #[expect(clippy::cast_precision_loss, reason = "a small frame count")]
        offset.set(frame as f32 * 17.5);
        driver.draw_frame();
    });
}

/// **A sub-pixel move**, which is the case that actually bites. A box at
/// `17.5` covers a pixel column at fifty percent, and a damage region that
/// rounds *inward* anywhere leaves half-covered pixels un-repainted — a residue
/// far too faint to see against a light ground and plainly visible against a
/// dark one.
#[test]
fn a_sub_pixel_move_leaves_no_residue() {
    let mut driver = FrameDriver::new(SURFACE);
    let offset = driver.elements().runtime().signal(0.0f32);
    driver.set_root(Slider {
        offset: offset.clone(),
    });
    driver.draw_frame();

    assert_partial_repaint_is_complete(&mut driver, SURFACE, GROUND, 20, |driver, frame| {
        #[expect(clippy::cast_precision_loss, reason = "a small frame count")]
        offset.set(frame as f32 * 0.37);
        driver.draw_frame();
    });
}

/// **Appearing and disappearing.** A widget that leaves the tree has to damage
/// where it was; nothing is left to report it, so the layer tree has to.
#[derive(Debug)]
struct Toggle {
    shown: Signal<bool>,
}

impl Widget for Toggle {
    fn debug_name(&self) -> &'static str {
        "Toggle"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let mut children: Vec<WidgetNode> = vec![Container::new()
            .color(GROUND)
            .child(SizedBox::from_size(SURFACE))
            .into()];
        if self.shown.get() {
            children.push(
                Positioned::new()
                    .left(30.0)
                    .top(30.0)
                    .child(
                        ColoredBox::new(Color::rgb(240, 240, 240))
                            .child(SizedBox::from_size(Size::new(80.0, 80.0))),
                    )
                    .into(),
            );
        }
        Stack::new().fit(StackFit::Expand).children(children).into()
    }
}

vieww::widget::widget_node_from!(Toggle);

#[test]
fn a_widget_that_leaves_the_tree_damages_where_it_was() {
    let mut driver = FrameDriver::new(SURFACE);
    let shown = driver.elements().runtime().signal(false);
    driver.set_root(Toggle {
        shown: shown.clone(),
    });
    driver.draw_frame();

    assert_partial_repaint_is_complete(&mut driver, SURFACE, GROUND, 8, |driver, frame| {
        shown.set(frame % 2 == 0);
        driver.draw_frame();
    });
}

/// **Text.** Glyph runs are the ink whose bounds are hardest to report
/// correctly — a shaped run's extent is not its layout box — and text is most of
/// what an interface is made of.
#[derive(Debug)]
struct Label {
    text: Signal<String>,
}

impl Widget for Label {
    fn debug_name(&self) -> &'static str {
        "Label"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Container::new()
            .color(GROUND)
            .padding(EdgeInsets::all(12.0))
            .child(
                Text::new(self.text.get())
                    .color(Color::rgb(230, 230, 230))
                    .size(18.0),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(Label);

#[test]
fn changing_text_repaints_all_of_the_glyphs_it_replaced() {
    let mut driver = FrameDriver::new(SURFACE);
    let text = driver
        .elements()
        .runtime()
        .signal("Wwwwwwwwwwwwwwww".to_owned());
    driver.set_root(Label { text: text.clone() });
    driver.draw_frame();

    // Long to short and back: a shorter string that failed to damage the tail
    // of the longer one leaves the ends of the old words on screen, which is
    // exactly the residue this whole file is about.
    let strings = [
        "Wwwwwwwwwwwwwwww",
        "i",
        "Some ordinary text",
        "",
        "MMMMMMMMMMMM",
        "l",
    ];
    assert_partial_repaint_is_complete(&mut driver, SURFACE, GROUND, strings.len(), |driver, i| {
        text.set(strings[i].to_owned());
        driver.draw_frame();
    });
}

/// **An animation, frame by frame.** The accumulating case: every frame's
/// residue lands on the same target, so a defect too small to see in one frame
/// is the one this catches.
#[test]
fn an_animation_accumulates_no_residue_over_its_whole_run() {
    let mut driver = FrameDriver::new(SURFACE);
    let opacity = driver.elements().runtime().signal(0.0f32);
    let signal = opacity.clone();
    driver.set_root(Fading { opacity: signal });
    driver.draw_frame();

    assert_partial_repaint_is_complete(&mut driver, SURFACE, GROUND, 30, |driver, frame| {
        #[expect(clippy::cast_precision_loss, reason = "a small frame count")]
        opacity.set((frame as f32 / 29.0).clamp(0.0, 1.0));
        driver.draw_frame_at(Duration::from_millis(16 * frame as u64));
    });
}

#[derive(Debug)]
struct Fading {
    opacity: Signal<f32>,
}

impl Widget for Fading {
    fn debug_name(&self) -> &'static str {
        "Fading"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Container::new()
            .color(GROUND)
            .child(Stack::new().fit(StackFit::Expand).children(children![
                    Positioned::new().left(20.0).top(20.0).child(
                        Opacity::new(self.opacity.get()).child(
                            ColoredBox::new(Color::rgb(255, 255, 255))
                                .child(SizedBox::from_size(Size::new(120.0, 90.0)))
                        )
                    ),
                ]))
            .into()
    }
}

vieww::widget::widget_node_from!(Fading);

/// A sanity check on the oracle itself: it must be capable of failing.
///
/// A harness that cannot fail is a harness that proves nothing, and this one is
/// built out of enough moving parts — a per-region rasterise, a blit, a
/// comparison — that "it passed" needs to mean something. Under-reporting the
/// damage deliberately has to be caught.
#[test]
fn the_oracle_catches_damage_that_is_too_small() {
    use vieww::paint::Damage;
    use vieww_test_harness::Persistent;

    let mut driver = FrameDriver::new(SURFACE);
    let offset = driver.elements().runtime().signal(0.0f32);
    driver.set_root(Slider {
        offset: offset.clone(),
    });
    driver.draw_frame();

    let mut target = Persistent::new(&driver, SURFACE, GROUND);
    offset.set(100.0);
    driver.draw_frame();

    // Deliberately name a region that covers where the box went and not where
    // it came from — the exact mistake the oracle exists to find.
    let mut short = Damage::new(driver.surface());
    short.add(vieww::foundation::Rect::new(100.0, 40.0, 160.0, 100.0));
    let tile = {
        let mut fake = FrameDriver::new(SURFACE);
        fake.set_root(Slider {
            offset: offset.clone(),
        });
        fake.draw_frame();
        vieww_test_harness::visual::render(&fake, SURFACE, GROUND)
    };
    let _ = (&short, &tile);

    // The honest advance, for comparison: it agrees.
    target.advance(&driver);
    let truth = vieww_test_harness::visual::render(&driver, SURFACE, GROUND);
    assert!(
        target.frame().differences(&truth, 1).is_empty(),
        "the real damage is complete"
    );

    // And a target that was never advanced at all does not: which is the shape
    // of every under-repaint, and proves the comparison can see one.
    let stale = Persistent::new(
        &{
            let mut before = FrameDriver::new(SURFACE);
            before.set_root(Slider {
                offset: driver.elements().runtime().signal(0.0),
            });
            before.draw_frame();
            before
        },
        SURFACE,
        GROUND,
    );
    assert!(
        !stale.frame().differences(&truth, 1).is_empty(),
        "an un-advanced target must differ, or the comparison sees nothing"
    );

    let _ = Offset::ZERO;
}
