//! Keyboard shortcut / accelerator matching.
//!
//! # Why this does not live in `vieww-render` beside focus and key dispatch
//!
//! `vieww-render::focus` answers "which render object gets this key" by
//! bubbling it up from whatever has focus. A shortcut is a different
//! question — "does this key, with these modifiers, mean something the
//! *application* registered a name for" — and answering it needs no tree, no
//! frame and no window, the same argument `vieww-gestures`'s own module doc
//! makes for keeping gesture recognition pure logic over events. A
//! `Shortcut` is tested by handing it a [`KeyEvent`]; nothing here has ever
//! seen a `RenderId`.

use vieww_foundation::{KeyEvent, KeyState, LogicalKey, Modifiers, TargetPlatform};

/// One key combination: a [`LogicalKey`] plus the exact [`Modifiers`] that
/// must be held — no more, no less.
///
/// **Exact, not "at least".** [`Modifiers::is_shortcut`] answers "is the
/// platform's command modifier held, plus optionally shift" for the common
/// case of recognising *a* shortcut key on the fly; a registered accelerator
/// table needs the opposite guarantee — Ctrl+S must not also fire when the
/// user pressed Ctrl+Shift+S for a *different* command, so this compares the
/// whole set for equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Shortcut {
    key: LogicalKey,
    modifiers: Modifiers,
}

impl Shortcut {
    /// `key` with exactly `modifiers` held.
    #[must_use]
    pub fn new(key: LogicalKey, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// `key` with this platform's own "shortcut" modifier (⌘ on Apple, Ctrl
    /// elsewhere) and nothing else — the accelerator every "primary action"
    /// menu item binds. Built on [`Modifiers::shortcut_for`] so a shortcut
    /// table does not hand-roll the platform branch a second time.
    #[must_use]
    pub fn primary(key: LogicalKey, platform: TargetPlatform) -> Self {
        Self::new(key, Modifiers::shortcut_for(platform))
    }

    /// `key` with this platform's shortcut modifier plus Shift — the
    /// accelerator convention for "the same command, but more" (Redo beside
    /// Undo, Save As beside Save).
    #[must_use]
    pub fn primary_shift(key: LogicalKey, platform: TargetPlatform) -> Self {
        Self::new(
            key,
            Modifiers::shortcut_for(platform).union(Modifiers::SHIFT),
        )
    }

    /// `true` if `event` is this exact combination going down, and not an
    /// auto-repeat.
    ///
    /// Repeats are excluded by default because a shortcut is an action, not a
    /// value that free-runs while a key is held — the same distinction
    /// [`KeyEvent::repeat`]'s own doc draws for a caret ("a shortcut usually
    /// wants to ignore repeats; a caret moving left usually does not"). A
    /// command that *should* free-run under a held accelerator (zoom-in
    /// stepping while ⌘+ is held down, say) can still be built: match on
    /// [`Self::matches_allowing_repeat`] instead.
    #[must_use]
    pub fn matches(&self, event: &KeyEvent) -> bool {
        !event.repeat && self.matches_allowing_repeat(event)
    }

    /// [`Self::matches`], without excluding auto-repeat.
    #[must_use]
    pub fn matches_allowing_repeat(&self, event: &KeyEvent) -> bool {
        event.state == KeyState::Down && event.key == self.key && event.modifiers == self.modifiers
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use vieww_foundation::NamedKey;

    use super::*;

    fn at(key: LogicalKey, modifiers: Modifiers) -> KeyEvent {
        KeyEvent::down(key, Duration::ZERO).with_modifiers(modifiers)
    }

    #[test]
    fn an_exact_combination_matches_itself() {
        let save = Shortcut::new(LogicalKey::Character("s".into()), Modifiers::CONTROL);
        assert!(save.matches(&at(LogicalKey::Character("s".into()), Modifiers::CONTROL)));
    }

    /// The whole reason this is exact-equality and not `contains`: two
    /// different commands must not collide because one's binding is a subset
    /// of the other's modifiers.
    #[test]
    fn extra_modifiers_do_not_match() {
        let save = Shortcut::new(LogicalKey::Character("s".into()), Modifiers::CONTROL);
        let save_as = at(
            LogicalKey::Character("s".into()),
            Modifiers::CONTROL.union(Modifiers::SHIFT),
        );
        assert!(
            !save.matches(&save_as),
            "Ctrl+S must not also answer to Ctrl+Shift+S"
        );
    }

    #[test]
    fn missing_modifiers_do_not_match() {
        let save = Shortcut::new(LogicalKey::Character("s".into()), Modifiers::CONTROL);
        assert!(!save.matches(&at(LogicalKey::Character("s".into()), Modifiers::NONE)));
    }

    #[test]
    fn an_up_event_never_matches() {
        let save = Shortcut::new(LogicalKey::Character("s".into()), Modifiers::CONTROL);
        let up = KeyEvent::up(LogicalKey::Character("s".into()), Duration::ZERO)
            .with_modifiers(Modifiers::CONTROL);
        assert!(!save.matches(&up));
    }

    #[test]
    fn a_repeat_does_not_match_by_default_but_does_when_allowed() {
        let zoom_in = Shortcut::new(LogicalKey::Character("+".into()), Modifiers::CONTROL);
        let repeated = at(LogicalKey::Character("+".into()), Modifiers::CONTROL).repeated();
        assert!(!zoom_in.matches(&repeated));
        assert!(zoom_in.matches_allowing_repeat(&repeated));
    }

    #[test]
    fn primary_resolves_per_platform() {
        let save_mac = Shortcut::primary(LogicalKey::Character("s".into()), TargetPlatform::MacOS);
        assert!(save_mac.matches(&at(LogicalKey::Character("s".into()), Modifiers::META)));
        assert!(!save_mac.matches(&at(LogicalKey::Character("s".into()), Modifiers::CONTROL)));

        let save_win =
            Shortcut::primary(LogicalKey::Character("s".into()), TargetPlatform::Windows);
        assert!(save_win.matches(&at(LogicalKey::Character("s".into()), Modifiers::CONTROL)));
    }

    #[test]
    fn primary_shift_adds_shift_to_the_platform_modifier() {
        let redo =
            Shortcut::primary_shift(LogicalKey::Character("z".into()), TargetPlatform::Windows);
        assert!(redo.matches(&at(
            LogicalKey::Character("z".into()),
            Modifiers::CONTROL.union(Modifiers::SHIFT)
        )));
        assert!(!redo.matches(&at(LogicalKey::Character("z".into()), Modifiers::CONTROL)));
    }

    #[test]
    fn a_named_key_shortcut_matches_by_variant() {
        let close = Shortcut::new(LogicalKey::Named(NamedKey::Escape), Modifiers::NONE);
        assert!(close.matches(&at(LogicalKey::Named(NamedKey::Escape), Modifiers::NONE)));
        assert!(!close.matches(&at(LogicalKey::Named(NamedKey::Enter), Modifiers::NONE)));
    }
}
