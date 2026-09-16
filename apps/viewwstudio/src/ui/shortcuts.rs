//! The layer that turns a keystroke into a `Command`.
//!
//! # Why this is a render object rather than a handler somewhere
//!
//! Keys in this framework go to whatever has focus and then **bubble outward**
//! to its ancestors, which is how a field can swallow ordinary typing while
//! Escape still reaches the dialog around it (`FocusManager::dispatch`). There
//! is no application-level key hook, and there should not be one: a shortcut
//! that fires while a text field is consuming the same key is the bug every
//! editor eventually has.
//!
//! So the studio's shortcuts live where the bubbling ends — one render object
//! wrapped around the whole shell. The editor sees `s` first and inserts it;
//! it sees `⌘S` and declines, because the shortcut modifier means the key is a
//! command rather than a character; the key then reaches here, matches
//! `Command::Save`, and is consumed.
//!
//! # It is focusable, and it has to be
//!
//! Bubbling starts at the focused object. With nothing focused — a studio that
//! has just opened and not been clicked — there is nothing to bubble *from*,
//! and no key would ever arrive. So this object accepts focus, sits outermost,
//! and [`crate::seed_focus`] points focus at it on the first frame. From then
//! on clicking the editor moves focus inwards, and this stays on the path back
//! out.
//!
//! # It changes nothing about layout
//!
//! One child, laid out against the same constraints, placed at the origin. A
//! layer that affected geometry would be a layer somebody had to think about
//! when reading the shell; this one is invisible in every sense but the one it
//! exists for.

use std::rc::Rc;

use vieww_foundation::{Constraints, KeyEvent, Offset, Size, TargetPlatform};
use vieww_render::{LayoutCtx, RenderObject};
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

use crate::state::Studio;

/// Wraps the shell and gives its keystrokes somewhere to go.
#[derive(Debug)]
pub struct Shortcuts {
    pub studio: Studio,
    pub child: WidgetNode,
}

impl Widget for Shortcuts {
    fn debug_name(&self) -> &'static str {
        "Shortcuts"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderSingleChild(&self.child)
    }
}

widget_node_from!(Shortcuts);

/// The render object behind [`Shortcuts`].
pub struct RenderShortcuts {
    /// Called with every key that reached this layer. Returns whether it was a
    /// command.
    ///
    /// A closure rather than a `Studio` field so that the object can be built
    /// in a test against a plain function, and so that this file does not have
    /// to name every signal the dispatcher touches.
    handle: Rc<dyn Fn(&KeyEvent) -> bool>,
}

impl std::fmt::Debug for RenderShortcuts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderShortcuts").finish_non_exhaustive()
    }
}

impl RenderShortcuts {
    /// The object the studio uses: every key matched against the command list.
    #[must_use]
    pub fn for_studio(studio: Studio) -> Self {
        let host = studio.host;
        Self::new(move |event| dispatch(&studio, event, host))
    }

    #[must_use]
    pub fn new(handle: impl Fn(&KeyEvent) -> bool + 'static) -> Self {
        Self {
            handle: Rc::new(handle),
        }
    }
}

/// Turn one key into at most one command.
///
/// Split out from the render object so the whole binding table can be tested
/// against a `Studio` with no tree, no window and no focus — which is the only
/// way to check thirty shortcuts without thirty end-to-end runs.
///
/// Returns whether the key was consumed. A key that matched nothing is **not**
/// consumed, so Tab traversal and anything else the framework does with an
/// unhandled key still works.
#[must_use]
pub fn dispatch(studio: &Studio, event: &KeyEvent, host: TargetPlatform) -> bool {
    // The palette owns the arrow keys and Enter while it is open. Checked
    // before the command table so that Enter in the palette runs the highlighted
    // command rather than doing whatever Enter would otherwise mean.
    if studio.palette_open.peek() && event.is_down() {
        use vieww_foundation::{LogicalKey, NamedKey};
        match &event.key {
            LogicalKey::Named(NamedKey::ArrowDown) => {
                studio.palette_step(1);
                return true;
            }
            LogicalKey::Named(NamedKey::ArrowUp) => {
                studio.palette_step(-1);
                return true;
            }
            LogicalKey::Named(NamedKey::Enter) if event.modifiers.is_empty() => {
                studio.palette_accept();
                return true;
            }
            _ => {}
        }
    }

    // The completion list owns the same four keys while it is open, and for
    // the same reason the palette does — and it is checked *after* the palette
    // because a palette opened over a completion list is the more recent
    // decision. Enter accepts, Tab accepts, the arrows move, and everything
    // else falls through to the editor so that typing keeps narrowing the
    // list rather than being swallowed by it.
    if studio.completion.peek().is_some() && event.is_down() {
        use vieww_foundation::{LogicalKey, NamedKey};
        match &event.key {
            LogicalKey::Named(NamedKey::ArrowDown) => {
                studio.move_completion(1);
                return true;
            }
            LogicalKey::Named(NamedKey::ArrowUp) => {
                studio.move_completion(-1);
                return true;
            }
            LogicalKey::Named(NamedKey::Enter | NamedKey::Tab) if event.modifiers.is_empty() => {
                studio.accept_completion();
                return true;
            }
            _ => {}
        }
    }

    // Through the studio rather than `Command::for_key`, so a chord the user
    // rebound in their keymap file is the chord that fires. `for_key` remains
    // the answer for anything with no studio to ask — the palette's own tests.
    let _ = host;
    let Some(command) = studio.command_for_key(event) else {
        return false;
    };

    // A chord that maps to a command which would do nothing is still consumed.
    // Letting ⌘Z fall through to the editor when there is nothing to undo would
    // insert a control character into the buffer, which is a far worse outcome
    // than a keypress that did not visibly do anything.
    if studio.can_run(command) {
        studio.run(command);
    }
    true
}

impl RenderObject for RenderShortcuts {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_key(&self, event: &KeyEvent) -> bool {
        (self.handle)(event)
    }

    /// Never provokes a relayout.
    ///
    /// The object is rebuilt every frame — it closes over a `Studio` that the
    /// shell hands down — and without this, every frame would report its layout
    /// as different from the last and relayout the entire window. It has no
    /// geometry of its own to differ by.
    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        false
    }

    fn debug_name(&self) -> &'static str {
        "Shortcuts"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;
    use vieww_foundation::{LogicalKey, Modifiers, NamedKey};

    #[test]
    fn a_key_that_matches_nothing_is_not_consumed() {
        let seen = Rc::new(Cell::new(0));
        let counter = Rc::clone(&seen);
        let object = RenderShortcuts::new(move |_| {
            counter.set(counter.get() + 1);
            false
        });

        let event = KeyEvent::character("a", Duration::ZERO);
        assert!(!object.handle_key(&event));
        assert_eq!(seen.get(), 1, "it was still offered the key");
    }

    #[test]
    fn the_layer_takes_focus_so_that_keys_have_somewhere_to_bubble_from() {
        let object = RenderShortcuts::new(|_| false);
        assert!(
            object.is_focusable(),
            "with nothing focused there is nothing to bubble from, and no \
             shortcut would ever arrive"
        );
    }

    #[test]
    fn escape_reaches_the_layer_as_a_command() {
        let event = KeyEvent::named(NamedKey::Escape, Duration::ZERO);
        assert_eq!(
            crate::command::Command::for_key(&event, TargetPlatform::Linux),
            Some(crate::command::Command::CloseFind)
        );
    }

    #[test]
    fn a_plain_letter_is_left_for_the_editor() {
        let event = KeyEvent::character("s", Duration::ZERO);
        assert_eq!(
            crate::command::Command::for_key(&event, TargetPlatform::Linux),
            None,
            "typing `s` must reach the buffer, not save the file"
        );
    }

    #[test]
    fn the_modifier_is_what_makes_a_key_a_command() {
        let event = KeyEvent::character("s", Duration::ZERO).with_modifiers(Modifiers::CONTROL);
        assert!(matches!(
            crate::command::Command::for_key(&event, TargetPlatform::Linux),
            Some(crate::command::Command::Save)
        ));
        assert!(matches!(&event.key, LogicalKey::Character(_)));
    }
}
