//! Keys, modifiers, and what a keypress means to text.
//!
//! The counterpart to [`PointerEvent`](crate::PointerEvent): the shape a
//! platform bridge translates a keyboard *into*, defined here so that
//! `vieww-widget` and `vieww-render` can both name it without either depending
//! on the other, and so that recognising Ctrl+A costs no platform code.
//!
//! # Logical keys, not physical ones
//!
//! [`LogicalKey`] is what the key *produced* — after the layout, after the
//! modifiers, after the input method. On an AZERTY keyboard the key where a
//! QWERTY `Q` sits produces `"a"`, and a text field wants the `"a"`.
//!
//! Physical scan codes are deliberately absent. They are what a game needs
//! ("the key left of `S`, whatever it prints"), and a UI framework needs the
//! opposite. Adding them later is additive; guessing which one a caller wanted
//! is not.

use std::fmt;
use std::time::Duration;

use crate::TargetPlatform;

/// Which modifier keys were held.
///
/// A set rather than four booleans, because the interesting questions are about
/// the whole set: a shortcut is "control and nothing else", and testing that as
/// `ctrl && !alt && !meta` is how Ctrl+Alt+Left ends up also firing Ctrl+Left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    /// Command on Apple platforms, Windows/Super elsewhere.
    pub const META: Self = Self(1 << 3);

    /// The union of two sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Everything in `self` that is not in `other`.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// `true` if every modifier in `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `true` if nothing is held.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn shift(self) -> bool {
        self.contains(Self::SHIFT)
    }

    #[must_use]
    pub const fn control(self) -> bool {
        self.contains(Self::CONTROL)
    }

    #[must_use]
    pub const fn alt(self) -> bool {
        self.contains(Self::ALT)
    }

    #[must_use]
    pub const fn meta(self) -> bool {
        self.contains(Self::META)
    }

    /// The modifier that means "this is a command, not a character", on this
    /// platform.
    ///
    /// Command on Apple, Control everywhere else. This is a behavioural
    /// difference in the sense `docs/DESIGN.md` §8 means — both platforms can do
    /// either, they simply expect different ones — so it branches on
    /// [`TargetPlatform`] rather than living in a platform crate.
    #[must_use]
    pub const fn shortcut_for(platform: TargetPlatform) -> Self {
        if platform.is_apple() {
            Self::META
        } else {
            Self::CONTROL
        }
    }

    /// `true` when exactly the shortcut modifier is held, and nothing else
    /// except possibly shift.
    ///
    /// Shift is excluded from the "nothing else" because it is how a shortcut
    /// says *extend*: Ctrl+Shift+Left is a word selection, not a different
    /// command.
    #[must_use]
    pub const fn is_shortcut(self, platform: TargetPlatform) -> bool {
        let shortcut = Self::shortcut_for(platform);
        self.contains(shortcut) && self.without(shortcut).without(Self::SHIFT).is_empty()
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }
        let mut first = true;
        for (flag, name) in [
            (Self::CONTROL, "control"),
            (Self::ALT, "alt"),
            (Self::SHIFT, "shift"),
            (Self::META, "meta"),
        ] {
            if self.contains(flag) {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

/// A key that has a meaning rather than a character.
///
/// Deliberately not exhaustive of every key a keyboard has: this is the set that
/// something in the framework actually reacts to. A key with no variant here
/// arrives as [`LogicalKey::Unidentified`] and is ignored, which is the right
/// outcome for a media key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamedKey {
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    Escape,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    /// A modifier pressed on its own. Reported so that a UI can show a shortcut
    /// hint while it is held; never an edit.
    Shift,
    Control,
    Alt,
    Meta,
    CapsLock,
}

impl NamedKey {
    /// `true` for keys that are only ever a modifier.
    ///
    /// These arrive as ordinary key events and must not be treated as input —
    /// pressing shift is not typing.
    #[must_use]
    pub const fn is_modifier(self) -> bool {
        matches!(
            self,
            Self::Shift | Self::Control | Self::Alt | Self::Meta | Self::CapsLock
        )
    }
}

/// What a key produced.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogicalKey {
    /// Text the key produces, already through the layout and any dead-key
    /// composition. Usually one grapheme, but never assume one `char`.
    Character(String),
    /// A key with a meaning rather than a character.
    Named(NamedKey),
    /// A key this framework has no name for. Delivered rather than dropped, so
    /// an application can handle it, and ignored by everything built in.
    Unidentified,
}

impl LogicalKey {
    /// The text this key would insert, if it inserts anything.
    ///
    /// `None` for named keys, control characters, and anything with no textual
    /// meaning. [`NamedKey::Enter`] and [`NamedKey::Tab`] are deliberately
    /// excluded: whether they insert a newline or move focus is a decision for
    /// whatever has focus, not for the key.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Character(text) => {
                // A control character reaching a text field would insert an
                // unprintable byte. Some platforms report Ctrl+A as U+0001.
                let printable = !text.is_empty()
                    && !text.chars().any(|character| {
                        character.is_control() && character != '\n' && character != '\t'
                    });
                printable.then_some(text.as_str())
            }
            Self::Named(NamedKey::Space) => Some(" "),
            _ => None,
        }
    }
}

impl fmt::Display for LogicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Character(text) => write!(f, "{text:?}"),
            Self::Named(named) => write!(f, "{named:?}"),
            Self::Unidentified => f.write_str("unidentified"),
        }
    }
}

/// Whether a key went down or came up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyState {
    Down,
    Up,
}

/// One keyboard event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: LogicalKey,
    pub state: KeyState,
    /// `true` when the platform's auto-repeat produced this rather than a
    /// finger. A shortcut usually wants to ignore repeats; a caret moving left
    /// usually does not.
    pub repeat: bool,
    pub modifiers: Modifiers,
    /// When the platform observed it. Supplied rather than read from a clock
    /// here, for the same reason [`PointerEvent`](crate::PointerEvent)'s is.
    pub timestamp: Duration,
}

impl KeyEvent {
    /// A key going down with no modifiers held.
    #[must_use]
    pub const fn down(key: LogicalKey, timestamp: Duration) -> Self {
        Self {
            key,
            state: KeyState::Down,
            repeat: false,
            modifiers: Modifiers::NONE,
            timestamp,
        }
    }

    /// A key coming up.
    #[must_use]
    pub const fn up(key: LogicalKey, timestamp: Duration) -> Self {
        Self {
            key,
            state: KeyState::Up,
            repeat: false,
            modifiers: Modifiers::NONE,
            timestamp,
        }
    }

    /// A character key going down.
    #[must_use]
    pub fn character(text: impl Into<String>, timestamp: Duration) -> Self {
        Self::down(LogicalKey::Character(text.into()), timestamp)
    }

    /// A named key going down.
    #[must_use]
    pub const fn named(key: NamedKey, timestamp: Duration) -> Self {
        Self::down(LogicalKey::Named(key), timestamp)
    }

    /// The same event with modifiers held.
    #[must_use]
    pub const fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// The same event marked as auto-repeat.
    #[must_use]
    pub const fn repeated(mut self) -> Self {
        self.repeat = true;
        self
    }

    /// `true` if this is a press rather than a release.
    #[must_use]
    pub const fn is_down(&self) -> bool {
        matches!(self.state, KeyState::Down)
    }

    /// What this keypress asks a text field to do, if anything.
    ///
    /// Returns `None` for releases, for modifiers pressed alone, and for
    /// anything with no editing meaning — so a field can call this and act on
    /// what comes back rather than matching on keys itself.
    ///
    /// Some intents ([`TextIntent::MoveLineStart`] and friends, and vertical
    /// movement) cannot be carried out by [`TextEditingValue`](crate::TextEditingValue)
    /// alone: where a line begins is a fact about the *laid out* text, which
    /// this crate cannot see. They are still produced here, because deciding
    /// what Home means is a keyboard question; carrying it out belongs to
    /// whatever holds the geometry.
    #[must_use]
    pub fn text_intent(&self, platform: TargetPlatform) -> Option<TextIntent> {
        if !self.is_down() {
            return None;
        }
        if let LogicalKey::Named(named) = &self.key {
            if named.is_modifier() {
                return None;
            }
        }

        let extend = self.modifiers.shift();
        let shortcut = self.modifiers.is_shortcut(platform);
        // On Apple, word-wise movement is Alt+Arrow; elsewhere it is
        // Ctrl+Arrow, which is the same key as the shortcut modifier. Asking
        // "is this word-wise" separately keeps both true at once from meaning
        // two different things.
        let word = if platform.is_apple() {
            self.modifiers.alt()
        } else {
            self.modifiers.control()
        };

        if shortcut {
            if let LogicalKey::Character(text) = &self.key {
                return match text.to_ascii_lowercase().as_str() {
                    "a" => Some(TextIntent::SelectAll),
                    // Lowercased above, so shift+cmd+C arrives here as "c" too.
                    // That is deliberate: the shifted forms are platform
                    // shortcuts for *other* things (copy-with-formatting, paste
                    // and match style), and vieww carries plain text only — so
                    // treating them as the plain ones is closer to right than
                    // dropping the key.
                    "c" => Some(TextIntent::Copy),
                    "x" => Some(TextIntent::Cut),
                    "v" => Some(TextIntent::Paste),
                    _ => None,
                };
            }
        }

        match &self.key {
            LogicalKey::Named(NamedKey::Backspace) if word => Some(TextIntent::DeleteWordBackward),
            LogicalKey::Named(NamedKey::Backspace) => Some(TextIntent::DeleteBackward),
            LogicalKey::Named(NamedKey::Delete) if word => Some(TextIntent::DeleteWordForward),
            LogicalKey::Named(NamedKey::Delete) => Some(TextIntent::DeleteForward),

            LogicalKey::Named(NamedKey::ArrowLeft) if word => {
                Some(TextIntent::MoveWordPrevious { extend })
            }
            LogicalKey::Named(NamedKey::ArrowRight) if word => {
                Some(TextIntent::MoveWordNext { extend })
            }
            LogicalKey::Named(NamedKey::ArrowLeft) => Some(TextIntent::MovePrevious { extend }),
            LogicalKey::Named(NamedKey::ArrowRight) => Some(TextIntent::MoveNext { extend }),
            LogicalKey::Named(NamedKey::ArrowUp) => Some(TextIntent::MoveUp { extend }),
            LogicalKey::Named(NamedKey::ArrowDown) => Some(TextIntent::MoveDown { extend }),

            LogicalKey::Named(NamedKey::Home) => Some(TextIntent::MoveLineStart { extend }),
            LogicalKey::Named(NamedKey::End) => Some(TextIntent::MoveLineEnd { extend }),
            LogicalKey::Named(NamedKey::PageUp) => Some(TextIntent::MovePageUp { extend }),
            LogicalKey::Named(NamedKey::PageDown) => Some(TextIntent::MovePageDown { extend }),

            LogicalKey::Named(NamedKey::Enter) => Some(TextIntent::Newline),

            // Everything else that produces text. Checked last, so a named key
            // with a textual fallback cannot pre-empt its own meaning.
            //
            // Control and Meta both mean "this is a command", on every platform
            // and not only the one whose shortcut modifier they are. An
            // unrecognised chord must therefore do *nothing* rather than fall
            // through to its letter — Ctrl+A on a Mac is not select-all, but it
            // is certainly not typing an "a" either.
            //
            // Alt is deliberately not in that list: AltGr is how a great many
            // layouts produce ordinary characters, and on Apple Option does the
            // same. Excluding it would make `@` untypeable on a German
            // keyboard.
            key if !self.modifiers.control() && !self.modifiers.meta() => {
                key.text().map(|text| TextIntent::Insert(text.to_owned()))
            }
            _ => None,
        }
    }
}

/// What a keypress asks a text field to do.
///
/// The seam between "which key was that" and "what changes". A platform decides
/// the first, a field carries out the second, and nothing has to know both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextIntent {
    /// Type this text, replacing any selection.
    Insert(String),
    /// Type a line break. Separate from `Insert("\n")` because a single-line
    /// field submits instead.
    Newline,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    MovePrevious {
        extend: bool,
    },
    MoveNext {
        extend: bool,
    },
    MoveWordPrevious {
        extend: bool,
    },
    MoveWordNext {
        extend: bool,
    },
    /// Needs the laid-out text; see [`KeyEvent::text_intent`].
    MoveUp {
        extend: bool,
    },
    MoveDown {
        extend: bool,
    },
    MoveLineStart {
        extend: bool,
    },
    MoveLineEnd {
        extend: bool,
    },
    /// A screenful up. Needs the laid-out text *and* a page size — see
    /// [`TextIntent::needs_layout`].
    MovePageUp {
        extend: bool,
    },
    MovePageDown {
        extend: bool,
    },
    SelectAll,
    /// Put the selected text on the pasteboard, leaving it in place.
    ///
    /// A no-op with nothing selected rather than copying the whole field: a
    /// caret is not a selection, and silently copying everything is the kind of
    /// helpfulness that overwrites what the user actually had on their
    /// pasteboard.
    Copy,
    /// Put the selected text on the pasteboard and delete it.
    Cut,
    /// Replace the selection — or insert at the caret — with the pasteboard's
    /// text.
    Paste,
}

impl TextIntent {
    /// `true` for intents that need the text's *geometry* to carry out, which
    /// this crate does not have.
    #[must_use]
    pub const fn needs_layout(&self) -> bool {
        matches!(
            self,
            Self::MoveUp { .. }
                | Self::MoveDown { .. }
                | Self::MovePageUp { .. }
                | Self::MovePageDown { .. }
                | Self::MoveLineStart { .. }
                | Self::MoveLineEnd { .. }
        )
    }

    /// Whether carrying this out needs the system pasteboard.
    ///
    /// The sibling of [`needs_layout`](Self::needs_layout), and separate from it
    /// because the two are refused by
    /// [`TextEditingValue::apply`](crate::TextEditingValue::apply) for different
    /// reasons: one wants geometry, this wants a platform service. A field with
    /// no `Clipboard` provided drops these rather than pretending.
    #[must_use]
    pub const fn needs_clipboard(&self) -> bool {
        matches!(self, Self::Copy | Self::Cut | Self::Paste)
    }
}

/// What a platform input method is doing to the text.
///
/// The shape a bridge translates the OS's input method into, and the reason
/// [`TextEditingValue::composing`](crate::TextEditingValue::composing) exists.
///
/// # Why this is not just "insert text"
///
/// An input method is not a keyboard with extra steps. Typing `ka` towards `か`
/// puts provisional text in the document so it can be *seen* and corrected,
/// then replaces it — possibly several times, as candidates are cycled — before
/// anything is committed. A bridge that only inserted would leave `kka` behind
/// on the second keystroke, and there is no way to recover from that after the
/// fact.
///
/// Autocomplete and spellcheck replacement on mobile are the same mechanism,
/// which is why they come out working once composition does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// A session opened. The field should expect preedits.
    Enabled,
    /// Provisional text, replacing whatever was being composed before it.
    ///
    /// `cursor` is where the input method wants the caret or selection *within*
    /// this text, in bytes from its start. `None` means the end.
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// The composition is finished and this text is real.
    Commit(String),
    /// The session closed. Any composition in progress is abandoned.
    Disabled,
}

impl ImeEvent {
    /// A preedit with the caret at its end.
    #[must_use]
    pub fn preedit(text: impl Into<String>) -> Self {
        Self::Preedit {
            text: text.into(),
            cursor: None,
        }
    }

    /// A commit.
    #[must_use]
    pub fn commit(text: impl Into<String>) -> Self {
        Self::Commit(text.into())
    }

    /// `true` for the events that change the document.
    #[must_use]
    pub const fn is_edit(&self) -> bool {
        matches!(self, Self::Preedit { .. } | Self::Commit(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(named: NamedKey) -> KeyEvent {
        KeyEvent::named(named, Duration::ZERO)
    }

    const LINUX: TargetPlatform = TargetPlatform::Linux;
    const IOS: TargetPlatform = TargetPlatform::IOS;

    #[test]
    fn a_modifier_set_answers_about_the_whole_set_not_one_flag() {
        let held = Modifiers::CONTROL.union(Modifiers::ALT);
        assert!(held.control());
        assert!(held.alt());
        assert!(!held.shift());
        assert!(
            !held.is_shortcut(LINUX),
            "control+alt is not a control shortcut, and treating it as one \
             makes every AltGr character fire one"
        );
    }

    #[test]
    fn the_shortcut_modifier_follows_the_platforms_convention() {
        assert_eq!(Modifiers::shortcut_for(LINUX), Modifiers::CONTROL);
        assert_eq!(Modifiers::shortcut_for(IOS), Modifiers::META);
        assert_eq!(
            Modifiers::shortcut_for(TargetPlatform::MacOS),
            Modifiers::META,
            "the mac harness must exercise the iOS path"
        );
    }

    #[test]
    fn shift_does_not_stop_something_being_a_shortcut() {
        let held = Modifiers::CONTROL.union(Modifiers::SHIFT);
        assert!(
            held.is_shortcut(LINUX),
            "ctrl+shift+left is a word selection, not an unrecognised chord"
        );
    }

    #[test]
    fn a_control_character_is_not_text_to_insert() {
        // Some platforms report Ctrl+A as U+0001 rather than as "a".
        let control = LogicalKey::Character("\u{1}".to_owned());
        assert_eq!(
            control.text(),
            None,
            "inserting this would put an unprintable byte in the document"
        );
        assert_eq!(LogicalKey::Character("a".to_owned()).text(), Some("a"));
        assert_eq!(LogicalKey::Named(NamedKey::Space).text(), Some(" "));
    }

    #[test]
    fn enter_and_tab_are_not_text_because_only_their_target_knows_what_they_mean() {
        assert_eq!(LogicalKey::Named(NamedKey::Enter).text(), None);
        assert_eq!(LogicalKey::Named(NamedKey::Tab).text(), None);
    }

    #[test]
    fn a_release_asks_a_field_for_nothing() {
        let up = KeyEvent::up(LogicalKey::Character("a".to_owned()), Duration::ZERO);
        assert_eq!(up.text_intent(LINUX), None);
    }

    #[test]
    fn pressing_shift_is_not_typing() {
        assert_eq!(key(NamedKey::Shift).text_intent(LINUX), None);
        assert_eq!(key(NamedKey::Control).text_intent(LINUX), None);
    }

    #[test]
    fn a_letter_is_an_insert() {
        let event = KeyEvent::character("q", Duration::ZERO);
        assert_eq!(
            event.text_intent(LINUX),
            Some(TextIntent::Insert("q".to_owned()))
        );
    }

    #[test]
    fn the_arrows_move_and_shift_extends() {
        assert_eq!(
            key(NamedKey::ArrowLeft).text_intent(LINUX),
            Some(TextIntent::MovePrevious { extend: false })
        );
        assert_eq!(
            key(NamedKey::ArrowRight)
                .with_modifiers(Modifiers::SHIFT)
                .text_intent(LINUX),
            Some(TextIntent::MoveNext { extend: true })
        );
    }

    #[test]
    fn word_movement_is_control_on_linux_and_alt_on_apple() {
        assert_eq!(
            key(NamedKey::ArrowLeft)
                .with_modifiers(Modifiers::CONTROL)
                .text_intent(LINUX),
            Some(TextIntent::MoveWordPrevious { extend: false })
        );
        assert_eq!(
            key(NamedKey::ArrowLeft)
                .with_modifiers(Modifiers::ALT)
                .text_intent(IOS),
            Some(TextIntent::MoveWordPrevious { extend: false })
        );
        assert_eq!(
            key(NamedKey::ArrowLeft)
                .with_modifiers(Modifiers::META)
                .text_intent(IOS),
            Some(TextIntent::MovePrevious { extend: false }),
            "command+left is line-wise on Apple, not word-wise; it must not be \
             mistaken for the word shortcut merely because a modifier was held"
        );
    }

    #[test]
    fn select_all_is_the_platforms_shortcut_and_not_a_bare_letter() {
        assert_eq!(
            KeyEvent::character("a", Duration::ZERO)
                .with_modifiers(Modifiers::CONTROL)
                .text_intent(LINUX),
            Some(TextIntent::SelectAll)
        );
        assert_eq!(
            KeyEvent::character("a", Duration::ZERO)
                .with_modifiers(Modifiers::CONTROL)
                .text_intent(IOS),
            None,
            "control+a is not select-all on Apple, and firing it there would \
             fight the platform's own emacs binding"
        );
        assert_eq!(
            KeyEvent::character("A", Duration::ZERO)
                .with_modifiers(Modifiers::META)
                .text_intent(IOS),
            Some(TextIntent::SelectAll),
            "caps lock must not break a shortcut"
        );
    }

    #[test]
    fn an_unhandled_shortcut_inserts_nothing() {
        assert_eq!(
            KeyEvent::character("k", Duration::ZERO)
                .with_modifiers(Modifiers::CONTROL)
                .text_intent(LINUX),
            None,
            "ctrl+k typing a 'k' is the classic version of this bug"
        );
        assert_eq!(
            KeyEvent::character("k", Duration::ZERO)
                .with_modifiers(Modifiers::META)
                .text_intent(LINUX),
            None,
            "and it must hold for the modifier that is *not* this platform's \
             shortcut, which is where it actually slipped through"
        );
    }

    #[test]
    fn altgr_still_types_a_character() {
        // On a German layout AltGr+Q is `@`. A blanket "a modifier means a
        // command" rule makes that untypeable, which is why Alt is excluded.
        assert_eq!(
            KeyEvent::character("@", Duration::ZERO)
                .with_modifiers(Modifiers::ALT)
                .text_intent(LINUX),
            Some(TextIntent::Insert("@".to_owned()))
        );
    }

    #[test]
    fn backspace_deletes_and_the_word_modifier_deletes_more() {
        assert_eq!(
            key(NamedKey::Backspace).text_intent(LINUX),
            Some(TextIntent::DeleteBackward)
        );
        assert_eq!(
            key(NamedKey::Backspace)
                .with_modifiers(Modifiers::CONTROL)
                .text_intent(LINUX),
            Some(TextIntent::DeleteWordBackward)
        );
    }

    #[test]
    fn the_intents_that_need_geometry_say_so() {
        for intent in [
            TextIntent::MoveUp { extend: false },
            TextIntent::MoveLineStart { extend: false },
            TextIntent::MoveLineEnd { extend: true },
        ] {
            assert!(intent.needs_layout(), "{intent:?}");
        }
        assert!(!TextIntent::DeleteBackward.needs_layout());
        assert!(!TextIntent::Insert("a".to_owned()).needs_layout());
    }
}
