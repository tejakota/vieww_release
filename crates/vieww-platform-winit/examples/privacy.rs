//! A masked subtree, and the lifecycle that masks it.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example privacy --release
//! ```
//!
//! # What this is for
//!
//! `docs/AIMS.md` §D's masking is closed in the framework and had **never been
//! looked at**. `crates/vieww/tests/sensitive_capture.rs` reads fills off the
//! `Scene` — deliberately, so it runs on a CI box with no GPU adapter — and
//! `the_mask_lands_exactly_where_the_secret_was` is a counted claim about
//! rectangles.
//!
//! Rectangles are not the risk. The risk is a cover that is one pixel short on
//! a fractional scale factor, a mask that reflows the page as it appears, or a
//! subtree that *looks* covered while the thing underneath still shows through
//! at the edges. All three are correct-by-the-numbers and wrong on a screen.
//!
//! # Things to check
//!
//! 1. **Press *Simulate capture*.** The card's secret is covered **and nothing
//!    else on the page moves**. That is the deliberate design: the child is
//!    still laid out while hidden, because a mask that collapsed would reflow —
//!    which leaks the shape of what was hidden and is visible for the frame it
//!    happens in.
//!
//! 2. **Look at the edges of the cover.** It must fully contain the text, on
//!    both a whole and a fractional scale factor. Drag the window between
//!    monitors of different densities if you have them; that is the case a
//!    test on a fixed scale cannot reach.
//!
//! 3. **Minimise the window, or cover it completely with another one.** The
//!    mask comes on by itself, with nothing pressed. That is
//!    `Lifecycle::capture` driving `FrameDriver::set_capture` — the wiring added
//!    2026-08-16 — and this is the only place it can be observed. Restore the
//!    window and the mask lifts.
//!
//! 4. **Click another application without covering this one.** The mask must
//!    **not** come on. Losing focus is deliberately not a capture: blanking a
//!    sensitive subtree every time somebody switches windows is how the feature
//!    gets turned off. See `lifecycle.rs`.
//!
//! 5. **The always-masked card never reveals**, whatever the capture state.
//!    `Sensitive::always` is a separate method from the automatic behaviour so
//!    that a call site says which of the two it is.
//!
//! # What this cannot show you
//!
//! **The Android recents thumbnail.** By the time winit reports `Suspended` the
//! surface is gone, so the masked frame has nowhere to go. Only `FLAG_SECURE`
//! beats that race, and it is a JNI call rather than a repaint — recorded in
//! `TRACKER.md` as the next piece of §D. What this example does cover is the
//! `Hidden` case, where the swapchain is still there to draw into.
//!
//! # Run it in release
//!
//! Debug builds of the layout pass are ten to thirty times slower.

use std::cell::Cell;
use std::rc::Rc;

use vieww_element::{Runtime, Signal};
use vieww_foundation::{Capture, Size};
use vieww_platform_winit::App;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

const SURFACE: Size = Size {
    width: 820.0,
    height: 620.0,
};

const BACKGROUND: Color = ThemeData::dark().colors.surface;

fn main() {
    // Shared with the button, and read once per presented frame.
    //
    // A plain `Cell` beside the signal rather than only the signal, because the
    // hook runs *outside* the tree: it has no `BuildContext`, so it cannot read
    // an inherited value or subscribe to anything. The signal drives the
    // rebuild, this drives the driver, and the button writes both.
    let simulated = Rc::new(Cell::new(false));

    let app = App::new()
        .title("vieww — privacy")
        .size(SURFACE)
        .background(BACKGROUND)
        .before_frame({
            let simulated = Rc::clone(&simulated);
            move |driver: &mut FrameDriver| {
                // `before_frame`, not `after_frame`: this has to *change* the
                // tree, and only this hook gets the driver mutably. It runs
                // just before the build, so the mask is in the frame about to
                // be drawn rather than one frame late — which for a privacy
                // feature is the difference between working and not.
                //
                // It only runs on a frame that happens at all, so a hidden
                // window leaves the `Recorded` that `Lifecycle::capture` set
                // standing rather than having it overwritten by a stale
                // `Screen` from here.
                driver.set_capture(if simulated.get() {
                    Capture::Recorded
                } else {
                    Capture::Screen
                });
            }
        });

    let result = app.run(move |driver: &mut FrameDriver| {
        let runtime = driver.elements().runtime().clone();
        driver.set_root(Shell {
            state: State::new(&runtime),
            simulated,
        });
    });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("privacy failed: {error}"),
    }
}

#[derive(Debug, Clone)]
struct State {
    /// What the *button* asks for, which is not the whole story.
    ///
    /// The real capture state is published above the tree by the frame driver
    /// and reaches `Sensitive` without passing through here — that is the whole
    /// design, and it is why minimising the window works with no application
    /// code involved. This signal exists only so the button has something to
    /// toggle and the readout has something to name.
    simulated: Signal<bool>,
}

impl State {
    fn new(runtime: &Runtime) -> Self {
        Self {
            simulated: runtime.signal(false),
        }
    }
}

#[derive(Debug)]
struct Shell {
    state: State,
    simulated: Rc<Cell<bool>>,
}

impl Widget for Shell {
    fn debug_name(&self) -> &'static str {
        "Shell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Theme::new(ThemeData::dark())
            .child(Body {
                state: self.state.clone(),
                simulated: Rc::clone(&self.simulated),
            })
            .into()
    }
}

widget_node_from!(Shell);

#[derive(Debug)]
struct Body {
    state: State,
    simulated: Rc<Cell<bool>>,
}

impl Widget for Body {
    fn debug_name(&self) -> &'static str {
        "Body"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        // Read from the tree, not from the button: this is the published state,
        // so it is already `Recorded` when the window is hidden.
        let capture = *ctx.inherit_or(Capture::default());
        let simulated = self.state.simulated.get();
        let toggle = self.state.simulated.clone();
        let flag = Rc::clone(&self.simulated);

        SafeArea::new()
            .child(
                Padding::new(EdgeInsets::all(28.0)).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(20.0)
                        .children(children![
                            Text::new("Sensitive content").style(theme.text.headline),
                            Text::new(format!(
                                "published capture state: {capture} — minimise this window \
                                 and watch it change with nothing pressed"
                            ))
                            .style(theme.text.label)
                            .color(theme.colors.on_surface_variant),
                            Flex::row()
                                .main_axis_size(MainAxisSize::Min)
                                .spacing(24.0)
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .children(children![
                                    card(
                                        "Automatic",
                                        "4242 4242 4242 4242",
                                        Sensitive::new().label("Card number hidden"),
                                        &theme
                                    ),
                                    card(
                                        "Always",
                                        "seed: correct horse battery staple",
                                        Sensitive::new()
                                            .always(true)
                                            .label("Recovery phrase hidden"),
                                        &theme
                                    ),
                                ]),
                            Button::new(if simulated {
                                "Stop simulating"
                            } else {
                                "Simulate capture"
                            })
                            .on_pressed(move || {
                                // Both halves: the flag the frame hook reads,
                                // and the signal that marks this element so the
                                // readout above rebuilds.
                                flag.set(!simulated);
                                toggle.set(!simulated);
                            }),
                            // A semantics demonstration rather than a visual one,
                            // and it belongs here because it is the same idea: a
                            // subtree that is present and deliberately not
                            // reachable. `BlockSemantics` is what every modal
                            // barrier uses; `ExcludeSemantics` is what a covered
                            // route uses. Neither changes a pixel, which is
                            // exactly why they need saying out loud somewhere.
                            Text::new("Semantics").style(theme.text.title),
                            Flex::row()
                                .main_axis_size(MainAxisSize::Min)
                                .spacing(24.0)
                                .children(children![
                                    ExcludeSemantics::new(true).child(note(
                                        "Excluded — invisible to a screen reader",
                                        &theme
                                    )),
                                    BlockSemantics::new(true).child(note(
                                        "Blocking — hides everything under it",
                                        &theme
                                    )),
                                ]),
                        ]),
                ),
            )
            .into()
    }
}

widget_node_from!(Body);

/// One labelled card with a secret in it.
///
/// The secret is a plain `Text` — nothing about it knows it is sensitive. That
/// is the design being shown: the declaration is one wrapper, and the pixels,
/// the semantics tree and the hit testing change together because
/// `skips_children` already meant all three.
fn card(title: &str, secret: &str, sensitive: Sensitive, theme: &ThemeData) -> WidgetNode {
    DecoratedBox::outlined(theme.colors.outline, 1.0, theme.metrics.corner)
        .child(
            Padding::new(EdgeInsets::all(16.0)).child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .main_axis_size(MainAxisSize::Min)
                    .spacing(8.0)
                    .children(children![
                        Text::new(title)
                            .style(theme.text.label)
                            .color(theme.colors.on_surface_variant),
                        sensitive.child(Text::new(secret).style(theme.text.body)),
                    ]),
            ),
        )
        .into()
}

fn note(text: &str, theme: &ThemeData) -> WidgetNode {
    DecoratedBox::rounded(theme.colors.surface_variant, theme.metrics.corner)
        .child(
            Padding::new(EdgeInsets::symmetric(10.0, 14.0))
                .child(Text::new(text).style(theme.text.label)),
        )
        .into()
}
