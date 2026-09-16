//! The half of the application that gets rebuilt while it runs.
//!
//! **Edit anything in this file and the running window follows** — see
//! `examples/reload-host` for the loop that makes that happen, and
//! `docs/HOT-RELOAD.md` for why it works.
//!
//! The two things to try, in this order, because they demonstrate opposite
//! halves of the design:
//!
//! 1. **Change `BAND` to another colour** and rebuild. The band changes and
//!    the tap count below it **does not reset** — the element kept its state
//!    across a new library being loaded, which is the whole thesis.
//! 2. **Add a field to [`Taps`]** and rebuild. The count *does* reset, on
//!    purpose: the state changed shape, the host notices through the
//!    fingerprint, and it rebuilds the tree instead of reading an old
//!    allocation with a new layout. That is the difference between a reload and
//!    undefined behaviour.

use std::any::Any;

use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, ElementState};

/// Edit me. The most obvious thing on the screen.
const BAND: Color = Color::rgb(186, 217, 212);

const INK: Color = Color::rgb(226, 232, 240);

/// State the reload has to preserve.
///
/// Declared to `guest!` below. Adding a field here changes its size, which the
/// host sees as an incompatible build.
#[derive(Debug, Default)]
pub struct Taps {
    count: u32,
    /// Set by the tap handler, taken by the element tree.
    ///
    /// **Writing state is not enough to redraw.** A handler runs during input
    /// dispatch, long after the build that created it returned, and nothing
    /// walks the tree looking for states that changed — so a count that goes up
    /// with no flag raised is a count nobody rebuilds and nobody sees. This is
    /// the whole of the "why is the number not moving" bug.
    pending: bool,
}

impl ElementState for Taps {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    /// Take the flag rather than read it: a state that answers `true` for ever
    /// rebuilds its element for ever, which looks exactly like a runaway
    /// animation.
    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

/// A button that counts, so there is something to lose.
#[derive(Debug)]
struct Counter;

impl Widget for Counter {
    fn debug_name(&self) -> &'static str {
        "Counter"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(Taps::default()))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let count = ctx.state::<Taps, _>(|taps| taps.count).unwrap_or(0);
        let handle = ctx.state_handle();

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(12.0)
            .children(children![
                Text::new(format!("tapped {count} times"))
                    .color(INK)
                    .size(20.0),
                Button::new("Tap me").on_pressed(move || {
                    // Written from a handler rather than from the build, which
                    // is the only place `ElementState` may be written.
                    if let Some(state) = &handle {
                        if let Some(taps) = state.borrow_mut().as_any_mut().downcast_mut::<Taps>() {
                            taps.count += 1;
                            // Without this the number never moves on screen.
                            taps.pending = true;
                        }
                    }
                }),
            ])
            .into()
    }
}

widget_node_from!(Counter);

/// The application's screen.
fn screen() -> impl Into<WidgetNode> {
    Container::new().padding(EdgeInsets::all(32.0)).child(
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(24.0)
            .children(children![
                // Edit this colour and watch it change without the count
                // below resetting.
                ColoredBox::new(BAND).child(SizedBox::from_size(Size::new(360.0, 64.0))),
                Text::new("edit examples/reload-guest/src/lib.rs")
                    .color(INK)
                    .size(14.0),
                Counter,
            ]),
    )
}

vieww_reload::guest! {
    root: screen,
    state: [Taps],
}
