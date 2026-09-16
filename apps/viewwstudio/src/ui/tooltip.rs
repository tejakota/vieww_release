//! What the ten icons in the left column are for.
//!
//! # The finding
//!
//! The activity bar is a 48-point column of ten icons and no words. A magnifier
//! is a search anywhere, and a tuning fork is not obviously "Rust, Android and
//! Apple toolchains" to anyone who has not already found it — so the only way to
//! learn the bar was to press each button and watch what the sidebar became.
//! The icons carried a `Semantics::button(title)` for screen readers, which
//! means the *name* existed all along and sighted users were the ones who could
//! not read it.
//!
//! # The name, and nothing else
//!
//! This carried the name *and* `View::note` on one card, on the argument that a
//! tooltip reading "Snippets" over the snippets icon confirms a guess and
//! teaches nothing. Two lines is a paragraph that appears under a moving
//! pointer, and it made the card tall enough to cover the icon it points at —
//! which is half of the flicker described below. The name is what a hover is
//! asking for; the sidebar the click opens is where the explanation belongs.
//!
//! # Why the tooltip is drawn here and not in the bar
//!
//! A popup clipped by its parent is not a popup: the bar is 48 points wide and
//! a 220-point tooltip drawn inside it would be cut to a sliver. So it lives in
//! the shell's overlay stack, like the menus and the picker, and finds the
//! button it points at through [`Studio::view_anchors`] — which `Measured` fills
//! in during paint, because a widget cannot work out its own position.
//!
//! # It must not be able to take the pointer, and "no recogniser" was not enough
//!
//! This file used to say that `vieww_widget::Tooltip` has no barrier and no
//! recogniser anywhere in it, so the button underneath stayed pressable. That
//! was true and insufficient, and the reported defect was the proof: the card
//! flickered at frame rate under a pointer that was not moving.
//!
//! The card is placed above the icon and clamped down when it does not fit, so
//! over the top of the column it lands on the icon it explains. Its
//! `DecoratedBox` and `Text` are opaque to hit testing — not because anything
//! recognises a gesture, but because they *draw*, and a point inside something
//! drawn is a hit. So the hover moved from the icon to the tooltip, the icon
//! was told the pointer had left, `hovered_view` cleared, the card came down,
//! the hover returned to the icon, and around again once per frame.
//!
//! [`IgnorePointer`] is the fix, and it is the
//! whole subtree rather than the panel: an explanation of a control must not be
//! able to take input from the control under any arrangement of its insides.
//! `apps/viewwstudio/tests/hover.rs` holds the pointer still on every icon in
//! the bar and asserts the name does not change.

use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, SizedBox};

use crate::state::Studio;

/// How solid the tooltip's card is.
///
/// Enough that the words are the words and not a wash of the code behind them;
/// little enough that a card appearing under the pointer reads as an overlay
/// rather than as a hole punched in the window.
const TOOLTIP_OPACITY: f32 = 0.88;

#[derive(Debug)]
pub struct TooltipLayer {
    pub studio: Studio,
}

impl Widget for TooltipLayer {
    fn debug_name(&self) -> &'static str {
        "TooltipLayer"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let Some(view) = self.studio.hovered_view.get() else {
            return SizedBox::shrink().into();
        };

        // Where the button is, as of the last time it was painted. Zero means
        // it has not been laid out yet — the very first frame, or a bar that is
        // not on screen — and a tooltip pointing at the window's corner is
        // worse than none.
        let anchor = self.studio.view_anchors.borrow()[view.index()];
        if anchor.width() <= 0.0 || anchor.height() <= 0.0 {
            return SizedBox::shrink().into();
        }

        // **Not fully opaque.** `vieww_widget::Tooltip` draws on the theme's
        // inverse surface, which in a dark studio is a bright white card — and a
        // bright card that appears under the pointer every time it crosses the
        // activity bar is a flash, not an explanation. Letting a little of the
        // chrome through takes the edge off it while leaving the text at the
        // contrast the framework chose: `Opacity` applies per drawing command,
        // so the label fades with its own background rather than being
        // composited through it.
        // `IgnorePointer` outside the opacity, so nothing in the subtree —
        // panel, text, or anything added to it later — can be hit. See the
        // module docs: this is the flicker fix, not a precaution.
        vieww_widget::IgnorePointer::new()
            .child(
                Opacity::new(TOOLTIP_OPACITY)
                    .child(vieww_widget::Tooltip::new(view.title()).anchor(anchor)),
            )
            .into()
    }
}

widget_node_from!(TooltipLayer);
