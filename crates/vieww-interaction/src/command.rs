//! A named-action registry: commands an application declares once, that a
//! menu item, a toolbar button and a keyboard [`Shortcut`] can all point at
//! by name instead of each holding its own copy of "what to do".
//!
//! # The problem this exists for
//!
//! Without it, "Save" is a closure wired into a menu item, a second closure
//! wired into a toolbar button, and a third comparison against Ctrl+S in a
//! key handler somewhere — three places that have to agree on whether saving
//! is currently possible, and three places that silently stop agreeing the
//! day one of them is updated and the other two are not. A [`CommandRegistry`]
//! makes "Save" one thing with one enabled flag and one action, and a menu
//! item, a toolbar button and the accelerator table below all just name it.

use std::fmt;

use vieww_foundation::KeyEvent;

use crate::shortcut::Shortcut;

/// A command's stable name.
///
/// A newtype around `&'static str` rather than a bare string: commands are
/// declared once at startup with string literals ("app.save",
/// "edit.find_next"), and a `CommandId` typo is a compile-time-visible
/// mismatch between two literals rather than a runtime "nothing happened"
/// from two `String`s that merely looked the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(pub &'static str);

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Why invoking a command did not run its action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// No command was ever registered under this id.
    Unknown(CommandId),
    /// The command exists but [`CommandRegistry::set_enabled`] has it off —
    /// "Save" while nothing is dirty, "Paste" while the clipboard is empty.
    Disabled(CommandId),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "no command registered as {id}"),
            Self::Disabled(id) => write!(f, "command {id} is disabled"),
        }
    }
}

impl std::error::Error for CommandError {}

struct Entry {
    id: CommandId,
    title: String,
    enabled: bool,
    shortcut: Option<Shortcut>,
    action: Box<dyn FnMut()>,
}

/// The set of named actions an application (or a plugin — see `vieww-plugin`)
/// has declared, with what each is called, whether it can currently run, and
/// what keyboard shortcut invokes it.
///
/// # Why a linear `Vec`, not a `HashMap<CommandId, Entry>`
///
/// An application registers tens of commands, not thousands, and every
/// lookup here (`find`, shortcut dispatch) already has to scan for the
/// *shortcut* match, which cannot be hashed on since a `KeyEvent` does not
/// carry a `CommandId` to hash toward. A second index for id lookups would
/// be a second data structure to keep in sync with the first for a workload
/// where the linear scan does not show up on a profile — the same call
/// `vieww-asset::ImageCache` makes about skipping eviction machinery a small
/// cache does not need.
#[derive(Default)]
pub struct CommandRegistry {
    commands: Vec<Entry>,
}

impl fmt::Debug for CommandRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandRegistry")
            .field(
                "commands",
                &self.commands.iter().map(|e| e.id).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl CommandRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Register a command. Replaces any existing registration under the same
    /// `id` — re-registering is how a plugin unload-then-reload or a hot
    /// path that re-declares its commands each startup stays idempotent
    /// rather than accumulating duplicates that would each independently
    /// answer a shortcut lookup.
    pub fn register(
        &mut self,
        id: CommandId,
        title: impl Into<String>,
        shortcut: Option<Shortcut>,
        action: impl FnMut() + 'static,
    ) {
        let entry = Entry {
            id,
            title: title.into(),
            enabled: true,
            shortcut,
            action: Box::new(action),
        };
        match self.commands.iter().position(|e| e.id == id) {
            Some(index) => self.commands[index] = entry,
            None => self.commands.push(entry),
        }
    }

    fn find_mut(&mut self, id: CommandId) -> Option<&mut Entry> {
        self.commands.iter_mut().find(|e| e.id == id)
    }

    fn find(&self, id: CommandId) -> Option<&Entry> {
        self.commands.iter().find(|e| e.id == id)
    }

    /// `true` if a command is registered under `id`.
    #[must_use]
    pub fn contains(&self, id: CommandId) -> bool {
        self.find(id).is_some()
    }

    /// What a command is currently called, if it exists.
    #[must_use]
    pub fn title(&self, id: CommandId) -> Option<&str> {
        self.find(id).map(|e| e.title.as_str())
    }

    /// `true` if a command exists and is enabled. `false`, not an error, for
    /// an unregistered id — a menu binding a not-yet-registered command (a
    /// plugin that has not loaded yet) reads as "grey it out", which is the
    /// same treatment a disabled one gets.
    #[must_use]
    pub fn is_enabled(&self, id: CommandId) -> bool {
        self.find(id).is_some_and(|e| e.enabled)
    }

    /// Enable or disable a command. Disabling one also stops its shortcut
    /// (if any) from firing — see [`Self::handle_key`].
    pub fn set_enabled(&mut self, id: CommandId, enabled: bool) {
        if let Some(entry) = self.find_mut(id) {
            entry.enabled = enabled;
        }
    }

    /// Run a command's action directly — what a menu item or a toolbar
    /// button calls on click.
    ///
    /// # Errors
    /// [`CommandError::Unknown`] if nothing is registered under `id`,
    /// [`CommandError::Disabled`] if it is registered but turned off.
    pub fn invoke(&mut self, id: CommandId) -> Result<(), CommandError> {
        let entry = self.find_mut(id).ok_or(CommandError::Unknown(id))?;
        if !entry.enabled {
            return Err(CommandError::Disabled(id));
        }
        (entry.action)();
        Ok(())
    }

    /// Find and run whichever enabled command's shortcut matches `event`.
    ///
    /// A disabled command's shortcut is skipped rather than reported as
    /// [`CommandError::Disabled`] — a key handler asking "did something just
    /// happen" wants a bool, and a disabled shortcut that silently does
    /// nothing is the correct behaviour for it (the visible menu item is
    /// already greyed out; the accelerator agreeing is the whole point of
    /// routing both through the same registry). Returns the id that fired,
    /// so a caller that wants to log or animate the match still can.
    pub fn handle_key(&mut self, event: &KeyEvent) -> Option<CommandId> {
        let id = self.commands.iter().find_map(|entry| {
            let shortcut = entry.shortcut.as_ref()?;
            (entry.enabled && shortcut.matches(event)).then_some(entry.id)
        })?;
        // `invoke` cannot fail here: `id` was just found enabled, and nothing
        // between that lookup and this call can disable it (single-threaded,
        // reentrant registration would replace-not-remove). Still routed
        // through the same one call site as every other invocation rather
        // than duplicating `(entry.action)()`, so there is exactly one place
        // a command ever actually runs.
        self.invoke(id).ok();
        Some(id)
    }

    /// Every registered command's id, title and shortcut, in registration
    /// order — what a menu builds itself from.
    pub fn iter(&self) -> impl Iterator<Item = (CommandId, &str, Option<&Shortcut>)> {
        self.commands
            .iter()
            .map(|e| (e.id, e.title.as_str(), e.shortcut.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use vieww_foundation::{KeyEvent, LogicalKey, Modifiers};

    use super::*;

    const SAVE: CommandId = CommandId("app.save");
    const FIND: CommandId = CommandId("app.find");

    fn counter() -> (Rc<RefCell<u32>>, impl FnMut()) {
        let count = Rc::new(RefCell::new(0));
        let action = {
            let count = Rc::clone(&count);
            move || *count.borrow_mut() += 1
        };
        (count, action)
    }

    fn ctrl_s() -> KeyEvent {
        KeyEvent::down(LogicalKey::Character("s".into()), Duration::ZERO)
            .with_modifiers(Modifiers::CONTROL)
    }

    #[test]
    fn invoking_a_registered_command_runs_its_action_exactly_once() {
        let (count, action) = counter();
        let mut registry = CommandRegistry::new();
        registry.register(SAVE, "Save", None, action);

        registry.invoke(SAVE).expect("registered and enabled");
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn invoking_an_unknown_command_is_an_error_not_a_silent_no_op() {
        let mut registry = CommandRegistry::new();
        assert_eq!(registry.invoke(SAVE), Err(CommandError::Unknown(SAVE)));
    }

    #[test]
    fn a_disabled_command_refuses_to_run() {
        let (count, action) = counter();
        let mut registry = CommandRegistry::new();
        registry.register(SAVE, "Save", None, action);
        registry.set_enabled(SAVE, false);

        assert_eq!(registry.invoke(SAVE), Err(CommandError::Disabled(SAVE)));
        assert_eq!(*count.borrow(), 0, "the action must not have run");
    }

    #[test]
    fn re_registering_the_same_id_replaces_rather_than_duplicates() {
        let (count_a, action_a) = counter();
        let (count_b, action_b) = counter();
        let mut registry = CommandRegistry::new();
        registry.register(SAVE, "Save", None, action_a);
        registry.register(SAVE, "Save As", None, action_b);

        registry.invoke(SAVE).unwrap();
        assert_eq!(
            *count_a.borrow(),
            0,
            "the first action must be gone, not also fired"
        );
        assert_eq!(*count_b.borrow(), 1);
        assert_eq!(registry.title(SAVE), Some("Save As"));
        assert_eq!(registry.iter().count(), 1, "one entry, not two");
    }

    #[test]
    fn a_matching_shortcut_invokes_its_command() {
        let (count, action) = counter();
        let mut registry = CommandRegistry::new();
        registry.register(
            SAVE,
            "Save",
            Some(Shortcut::new(
                LogicalKey::Character("s".into()),
                Modifiers::CONTROL,
            )),
            action,
        );

        let fired = registry.handle_key(&ctrl_s());
        assert_eq!(fired, Some(SAVE));
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn a_disabled_commands_shortcut_does_not_fire() {
        let (count, action) = counter();
        let mut registry = CommandRegistry::new();
        registry.register(
            SAVE,
            "Save",
            Some(Shortcut::new(
                LogicalKey::Character("s".into()),
                Modifiers::CONTROL,
            )),
            action,
        );
        registry.set_enabled(SAVE, false);

        assert_eq!(registry.handle_key(&ctrl_s()), None);
        assert_eq!(*count.borrow(), 0);
    }

    #[test]
    fn an_unmatched_key_fires_nothing() {
        let (count, action) = counter();
        let mut registry = CommandRegistry::new();
        registry.register(
            SAVE,
            "Save",
            Some(Shortcut::new(
                LogicalKey::Character("s".into()),
                Modifiers::CONTROL,
            )),
            action,
        );

        let unrelated = KeyEvent::down(LogicalKey::Character("x".into()), Duration::ZERO);
        assert_eq!(registry.handle_key(&unrelated), None);
        assert_eq!(*count.borrow(), 0);
    }

    #[test]
    fn two_commands_with_different_shortcuts_do_not_cross_fire() {
        let (save_count, save_action) = counter();
        let (find_count, find_action) = counter();
        let mut registry = CommandRegistry::new();
        registry.register(
            SAVE,
            "Save",
            Some(Shortcut::new(
                LogicalKey::Character("s".into()),
                Modifiers::CONTROL,
            )),
            save_action,
        );
        registry.register(
            FIND,
            "Find",
            Some(Shortcut::new(
                LogicalKey::Character("f".into()),
                Modifiers::CONTROL,
            )),
            find_action,
        );

        registry.handle_key(&ctrl_s());
        assert_eq!(*save_count.borrow(), 1);
        assert_eq!(*find_count.borrow(), 0);
    }

    #[test]
    fn an_unregistered_command_reads_as_disabled_rather_than_erroring_on_query() {
        let registry = CommandRegistry::new();
        assert!(!registry.is_enabled(SAVE));
        assert!(!registry.contains(SAVE));
        assert_eq!(registry.title(SAVE), None);
    }
}
