//! `winit` keyboard and input-method events as the framework's own.
//!
//! Pure, like [`PointerTranslator`](crate::PointerTranslator), and for the same
//! reason: the interesting part is the mapping, and a mapping can be tested
//! without a window.
//!
//! # Modifiers are tracked, not read off the event
//!
//! `winit` reports modifier *changes* through `ModifiersChanged` and does not
//! attach the current set to each key event. So the set is kept here and
//! attached on the way through — which also means a modifier released while the
//! window was unfocused cannot leave a phantom Control held down, because
//! `ModifiersChanged` fires on the way back in.

use std::time::Duration;

use vieww_foundation::{ImeEvent, KeyEvent, KeyState, LogicalKey, Modifiers, NamedKey};
use winit::event::{ElementState, Ime, KeyEvent as WinitKey};
use winit::keyboard::{Key as WinitLogical, ModifiersState, NamedKey as WinitNamed};

/// Holds what `winit` will not repeat, and translates the rest.
#[derive(Debug, Default)]
pub struct KeyTranslator {
    modifiers: Modifiers,
}

impl KeyTranslator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modifiers: Modifiers::NONE,
        }
    }

    /// The modifiers currently held.
    #[must_use]
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Follow a `ModifiersChanged`.
    pub fn set_modifiers(&mut self, state: ModifiersState) {
        let mut modifiers = Modifiers::NONE;
        if state.shift_key() {
            modifiers = modifiers.union(Modifiers::SHIFT);
        }
        if state.control_key() {
            modifiers = modifiers.union(Modifiers::CONTROL);
        }
        if state.alt_key() {
            modifiers = modifiers.union(Modifiers::ALT);
        }
        if state.super_key() {
            modifiers = modifiers.union(Modifiers::META);
        }
        self.modifiers = modifiers;
    }

    /// Every modifier released.
    ///
    /// What losing focus owes: the platform stops reporting changes while
    /// another window has the keyboard, so a Control held as the user
    /// alt-tabbed away would still be held when they came back, and the next
    /// letter they typed would be a shortcut.
    pub fn release_modifiers(&mut self) {
        self.modifiers = Modifiers::NONE;
    }

    /// One key event.
    ///
    /// `None` for a key with no meaning here — a media key, or a dead key
    /// mid-composition, which the input method will report as a preedit instead.
    pub fn key(&self, event: &WinitKey, now: Duration) -> Option<KeyEvent> {
        let key = logical_key(&event.logical_key)?;
        Some(KeyEvent {
            key,
            state: match event.state {
                ElementState::Pressed => KeyState::Down,
                ElementState::Released => KeyState::Up,
            },
            repeat: event.repeat,
            modifiers: self.modifiers,
            timestamp: now,
        })
    }
}

/// `winit`'s logical key as ours.
fn logical_key(key: &WinitLogical) -> Option<LogicalKey> {
    Some(match key {
        WinitLogical::Character(text) => LogicalKey::Character(text.as_str().to_owned()),
        WinitLogical::Named(named) => match named {
            WinitNamed::Enter => LogicalKey::Named(NamedKey::Enter),
            WinitNamed::Tab => LogicalKey::Named(NamedKey::Tab),
            WinitNamed::Space => LogicalKey::Named(NamedKey::Space),
            WinitNamed::Backspace => LogicalKey::Named(NamedKey::Backspace),
            WinitNamed::Delete => LogicalKey::Named(NamedKey::Delete),
            WinitNamed::Escape => LogicalKey::Named(NamedKey::Escape),
            WinitNamed::ArrowLeft => LogicalKey::Named(NamedKey::ArrowLeft),
            WinitNamed::ArrowRight => LogicalKey::Named(NamedKey::ArrowRight),
            WinitNamed::ArrowUp => LogicalKey::Named(NamedKey::ArrowUp),
            WinitNamed::ArrowDown => LogicalKey::Named(NamedKey::ArrowDown),
            WinitNamed::Home => LogicalKey::Named(NamedKey::Home),
            WinitNamed::End => LogicalKey::Named(NamedKey::End),
            WinitNamed::PageUp => LogicalKey::Named(NamedKey::PageUp),
            WinitNamed::PageDown => LogicalKey::Named(NamedKey::PageDown),
            WinitNamed::Insert => LogicalKey::Named(NamedKey::Insert),
            WinitNamed::Shift => LogicalKey::Named(NamedKey::Shift),
            WinitNamed::Control => LogicalKey::Named(NamedKey::Control),
            WinitNamed::Alt => LogicalKey::Named(NamedKey::Alt),
            WinitNamed::Super => LogicalKey::Named(NamedKey::Meta),
            WinitNamed::CapsLock => LogicalKey::Named(NamedKey::CapsLock),
            // A key this framework has no name for. Reported rather than
            // dropped, so an application can still see it, and ignored by
            // everything built in.
            _ => LogicalKey::Unidentified,
        },
        // A dead key on the way to a composed character. The input method
        // reports the result as a preedit; treating this as text would type the
        // accent on its own.
        WinitLogical::Dead(_) => return None,
        _ => LogicalKey::Unidentified,
    })
}

/// `winit`'s input method event as ours.
#[must_use]
pub fn ime(event: &Ime) -> ImeEvent {
    match event {
        Ime::Enabled => ImeEvent::Enabled,
        Ime::Preedit(text, cursor) => ImeEvent::Preedit {
            text: text.clone(),
            cursor: *cursor,
        },
        Ime::Commit(text) => ImeEvent::Commit(text.clone()),
        Ime::Disabled => ImeEvent::Disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translator_with(state: ModifiersState) -> KeyTranslator {
        let mut translator = KeyTranslator::new();
        translator.set_modifiers(state);
        translator
    }

    #[test]
    fn every_modifier_crosses_over() {
        let translator = translator_with(
            ModifiersState::SHIFT
                | ModifiersState::CONTROL
                | ModifiersState::ALT
                | ModifiersState::SUPER,
        );
        let held = translator.modifiers();
        assert!(held.shift() && held.control() && held.alt() && held.meta());
    }

    #[test]
    fn losing_focus_releases_everything() {
        let mut translator = translator_with(ModifiersState::CONTROL);
        assert!(translator.modifiers().control());

        translator.release_modifiers();
        assert!(
            translator.modifiers().is_empty(),
            "a control held across an alt-tab turns the next letter typed into \
             a shortcut"
        );
    }

    #[test]
    fn a_dead_key_is_not_text() {
        assert_eq!(
            logical_key(&WinitLogical::Dead(Some('\u{301}'))),
            None,
            "typing the accent on its own is what this prevents"
        );
    }

    #[test]
    fn a_named_key_this_framework_knows_crosses_over() {
        assert_eq!(
            logical_key(&WinitLogical::Named(WinitNamed::Backspace)),
            Some(LogicalKey::Named(NamedKey::Backspace))
        );
        assert_eq!(
            logical_key(&WinitLogical::Named(WinitNamed::Super)),
            Some(LogicalKey::Named(NamedKey::Meta)),
            "winit calls it Super and the framework calls it Meta"
        );
    }

    #[test]
    fn a_named_key_it_does_not_know_is_delivered_as_unidentified() {
        assert_eq!(
            logical_key(&WinitLogical::Named(WinitNamed::F13)),
            Some(LogicalKey::Unidentified),
            "dropped would mean an application could never handle it"
        );
    }

    #[test]
    fn the_ime_events_cross_over_including_the_cursor() {
        assert_eq!(ime(&Ime::Enabled), ImeEvent::Enabled);
        assert_eq!(
            ime(&Ime::Preedit("ka".to_owned(), Some((0, 2)))),
            ImeEvent::Preedit {
                text: "ka".to_owned(),
                cursor: Some((0, 2)),
            }
        );
        assert_eq!(
            ime(&Ime::Commit("か".to_owned())),
            ImeEvent::Commit("か".to_owned())
        );
        assert_eq!(ime(&Ime::Disabled), ImeEvent::Disabled);
    }
}
