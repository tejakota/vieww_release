//! Desktop capabilities that live outside the window this framework draws.
//!
//! A tray icon, a global hotkey and a native context menu. Three unrelated
//! things with one property in common, and it is the property that decides the
//! whole shape of this module.
//!
//! # They are services, not widgets
//!
//! Everything else this framework draws is inside a surface it owns. These are
//! not:
//!
//! - a **tray icon** is drawn by the shell, in a bar this process cannot see and
//!   whose size, theme and behaviour belong to the desktop environment;
//! - a **global hotkey** fires while another application has focus, which is the
//!   entire reason for wanting one;
//! - a **native context menu** is a platform window with the platform's own
//!   look, keyboard handling, and — the part that actually matters —
//!   screen-reader support.
//!
//! Modelling any of them as a widget means inventing a coordinate space for
//! pixels we do not own, and the failure mode is a menu that looks *nearly*
//! native. That is worse than one that plainly is not: it invites the comparison
//! and then loses it, and it is the only one of the three that a user is
//! guaranteed to notice.
//!
//! So they arrive through [`Services`], like
//! [`DeepLinks`](crate::service::DeepLinks) and like a camera would — the same
//! seam a third party registers through, which is `docs/AIMS.md` §A's test
//! applied to vieww's own capabilities.
//!
//! # Four decisions worth keeping
//!
//! - **Events are polled.** A tray click arrives on the platform's thread at a
//!   moment unrelated to a frame, and a signal may only be written on the UI
//!   thread. So `take_*` rather than `on_*`, and the headless implementations
//!   are queues — which makes "the user clicked the tray icon" one line in a
//!   test.
//! - **A taken hotkey is [`ServiceError::Failed`], not `Unsupported`.** A global
//!   chord is machine-wide and first come first served, so losing one is
//!   *expected*: a screenshot tool has it, or a second copy of this application.
//!   An app that treats it as fatal will not start on a busy desktop.
//!   `Unsupported` means retrying with a different chord cannot help.
//! - **A separator is a property of the row above it, not a row.** As a variant
//!   it would put a hole in the indices the platform reports back, and the bug
//!   that follows is an application running the wrong command from its own tray
//!   menu.
//! - **The context menu answers later.** Windows' `TrackPopupMenu` does not
//!   return until the user has chosen, and macOS takes the run loop. Blocking a
//!   frame on either stops the application repainting mid-interaction, so the
//!   choice is polled like everything else here.

use std::cell::RefCell;
use std::rc::Rc;

use crate::keyboard::{LogicalKey, Modifiers};
use crate::service::{ServiceError, Services};
use crate::{IconData, Offset};

/// What an application calls one of its own menu rows.
///
/// A plain `u32` chosen by the caller rather than an index into the menu,
/// because a menu is rebuilt whenever its contents change and an index means
/// something different afterwards. The id is what the application actually knows
/// — "this is the Quit row" — and it survives the rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MenuId(pub u32);

/// One row of a native menu.
///
/// Deliberately not a widget: this describes a row to the *platform*, which then
/// draws it in its own style with its own keyboard handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeMenuItem {
    /// What comes back when this row is chosen.
    pub id: MenuId,
    pub label: String,
    /// A greyed-out row that still shows what is available.
    pub enabled: bool,
    /// A tick beside the label, for a row that toggles something.
    pub checked: bool,
    /// Draw a divider **after** this row.
    ///
    /// A property rather than a `Separator` variant, deliberately. A variant
    /// occupies a position in the list the platform reports choices back by, so
    /// every consumer has to remember to skip it when mapping a chosen index
    /// onto a command — and the one that forgets runs the row below the one the
    /// user picked. Made impossible instead of documented.
    pub separator_after: bool,
}

impl NativeMenuItem {
    /// An ordinary enabled row.
    #[must_use]
    pub fn new(id: u32, label: impl Into<String>) -> Self {
        Self {
            id: MenuId(id),
            label: label.into(),
            enabled: true,
            checked: false,
            separator_after: false,
        }
    }

    /// Greyed out, still visible.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Ticked.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Followed by a divider.
    #[must_use]
    pub fn then_separator(mut self) -> Self {
        self.separator_after = true;
        self
    }
}

/// A key combination the OS delivers even when this application is not focused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: Modifiers,
    pub key: LogicalKey,
}

impl Hotkey {
    #[must_use]
    pub const fn new(modifiers: Modifiers, key: LogicalKey) -> Self {
        Self { modifiers, key }
    }
}

/// A registration handle, so a hotkey can be released without naming it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyId(u32);

impl HotkeyId {
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// An id minted by a backend.
    ///
    /// Public because a real implementation lives in another crate and has to be
    /// able to hand back the id the platform already chose. Every desktop hotkey
    /// API derives an id from the chord itself — the same combination registered
    /// twice is the same number — so a backend that minted its own counter
    /// instead would have two names for one registration and lose track of which
    /// the OS will report.
    ///
    /// Not something an application should call: an id it invented names nothing,
    /// and [`GlobalHotkeys::unregister`] will refuse it.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// An icon in the system tray, notification area or menu bar.
pub trait Tray: 'static {
    /// Show an icon, replacing any current one.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where there is no tray, and
    /// [`ServiceError::Failed`] where there is one and it refused — most often a
    /// Linux session with no status-notifier host running.
    fn set_icon(&self, icon: &IconData) -> Result<(), ServiceError>;

    /// The text shown on hover.
    ///
    /// # Errors
    ///
    /// As [`set_icon`](Self::set_icon).
    fn set_tooltip(&self, text: &str) -> Result<(), ServiceError>;

    /// Replace the menu shown when the icon is clicked.
    ///
    /// # Errors
    ///
    /// As [`set_icon`](Self::set_icon).
    fn set_menu(&self, items: &[NativeMenuItem]) -> Result<(), ServiceError>;

    /// The menu row chosen since this was last called, if any.
    fn take_activated(&self) -> Option<MenuId>;

    /// Remove the icon.
    ///
    /// # Errors
    ///
    /// As [`set_icon`](Self::set_icon).
    fn remove(&self) -> Result<(), ServiceError>;
}

/// Key combinations delivered while another application has focus.
pub trait GlobalHotkeys: 'static {
    /// Claim a combination machine-wide.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] when something else already holds the chord —
    /// which is ordinary and recoverable by asking for a different one — and
    /// [`ServiceError::Unsupported`] only where no chord could ever work.
    fn register(&self, hotkey: &Hotkey) -> Result<HotkeyId, ServiceError>;

    /// Release a combination claimed earlier.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the id was never issued or already released.
    fn unregister(&self, id: HotkeyId) -> Result<(), ServiceError>;

    /// A combination pressed since this was last called, if any.
    fn take_pressed(&self) -> Option<HotkeyId>;
}

/// The platform's own right-click menu.
pub trait ContextMenus: 'static {
    /// Show a menu at `position`, in screen coordinates.
    ///
    /// Returns as soon as the menu is up. The choice arrives through
    /// [`take_chosen`](Self::take_chosen) — see the module docs for why this
    /// cannot block.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where the platform's menu cannot be
    /// attached to this framework's window.
    fn show(&self, items: &[NativeMenuItem], position: Offset) -> Result<(), ServiceError>;

    /// The row chosen since this was last called, if any.
    ///
    /// `None` also covers "dismissed without choosing", which is not an error
    /// and not distinguishable on every platform.
    fn take_chosen(&self) -> Option<MenuId>;
}

/// A tray that records what it was asked for and never draws anything.
///
/// A queue rather than a stub: [`push_activated`](Self::push_activated) is how a
/// test says "the user clicked Quit", which is the event an application's tray
/// handling exists to respond to and the one a stub cannot produce.
#[derive(Debug, Default)]
pub struct HeadlessTray {
    tooltip: RefCell<Option<String>>,
    menu: RefCell<Vec<NativeMenuItem>>,
    icon: RefCell<bool>,
    activated: RefCell<Vec<MenuId>>,
}

impl HeadlessTray {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend the user chose a row.
    pub fn push_activated(&self, id: MenuId) {
        self.activated.borrow_mut().push(id);
    }

    /// The menu currently set, for assertions.
    #[must_use]
    pub fn menu(&self) -> Vec<NativeMenuItem> {
        self.menu.borrow().clone()
    }

    /// The tooltip currently set, for assertions.
    #[must_use]
    pub fn tooltip(&self) -> Option<String> {
        self.tooltip.borrow().clone()
    }

    /// `true` while an icon is shown.
    #[must_use]
    pub fn has_icon(&self) -> bool {
        *self.icon.borrow()
    }
}

impl Tray for HeadlessTray {
    fn set_icon(&self, _icon: &IconData) -> Result<(), ServiceError> {
        *self.icon.borrow_mut() = true;
        Ok(())
    }

    fn set_tooltip(&self, text: &str) -> Result<(), ServiceError> {
        *self.tooltip.borrow_mut() = Some(text.to_owned());
        Ok(())
    }

    fn set_menu(&self, items: &[NativeMenuItem]) -> Result<(), ServiceError> {
        *self.menu.borrow_mut() = items.to_vec();
        Ok(())
    }

    fn take_activated(&self) -> Option<MenuId> {
        let mut queue = self.activated.borrow_mut();
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }

    fn remove(&self) -> Result<(), ServiceError> {
        *self.icon.borrow_mut() = false;
        self.menu.borrow_mut().clear();
        Ok(())
    }
}

/// Hotkeys that are claimed in a `Vec` rather than from the window server.
///
/// Models the one behaviour that actually catches applications out: a chord
/// already held is **refused**. Registering the same combination twice against
/// this fake fails exactly as it does against a desktop where a screenshot tool
/// got there first.
#[derive(Debug, Default)]
pub struct HeadlessGlobalHotkeys {
    held: RefCell<Vec<(HotkeyId, Hotkey)>>,
    next: RefCell<u32>,
    pressed: RefCell<Vec<HotkeyId>>,
}

impl HeadlessGlobalHotkeys {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend the user pressed a registered chord.
    pub fn push_pressed(&self, id: HotkeyId) {
        self.pressed.borrow_mut().push(id);
    }

    /// How many chords are currently claimed.
    #[must_use]
    pub fn held(&self) -> usize {
        self.held.borrow().len()
    }
}

impl GlobalHotkeys for HeadlessGlobalHotkeys {
    fn register(&self, hotkey: &Hotkey) -> Result<HotkeyId, ServiceError> {
        let mut held = self.held.borrow_mut();
        if held.iter().any(|(_, held)| held == hotkey) {
            return Err(ServiceError::failed(
                "that combination is already claimed on this machine",
            ));
        }
        let mut next = self.next.borrow_mut();
        let id = HotkeyId(*next);
        *next += 1;
        held.push((id, hotkey.clone()));
        Ok(id)
    }

    fn unregister(&self, id: HotkeyId) -> Result<(), ServiceError> {
        let mut held = self.held.borrow_mut();
        let before = held.len();
        held.retain(|(held, _)| *held != id);
        if held.len() == before {
            return Err(ServiceError::failed("that hotkey is not registered"));
        }
        Ok(())
    }

    fn take_pressed(&self) -> Option<HotkeyId> {
        let mut queue = self.pressed.borrow_mut();
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }
}

/// A context menu that is never shown and can still be chosen from.
#[derive(Debug, Default)]
pub struct HeadlessContextMenus {
    shown: RefCell<Option<(Vec<NativeMenuItem>, Offset)>>,
    chosen: RefCell<Vec<MenuId>>,
}

impl HeadlessContextMenus {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend the user picked a row from the menu currently up.
    pub fn push_chosen(&self, id: MenuId) {
        self.chosen.borrow_mut().push(id);
    }

    /// The menu currently up, and where it was put.
    #[must_use]
    pub fn shown(&self) -> Option<(Vec<NativeMenuItem>, Offset)> {
        self.shown.borrow().clone()
    }
}

impl ContextMenus for HeadlessContextMenus {
    fn show(&self, items: &[NativeMenuItem], position: Offset) -> Result<(), ServiceError> {
        *self.shown.borrow_mut() = Some((items.to_vec(), position));
        Ok(())
    }

    fn take_chosen(&self) -> Option<MenuId> {
        let mut queue = self.chosen.borrow_mut();
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }
}

/// Register all three headless implementations.
///
/// For tests and for a headless build. An application asking for a tray gets one
/// that accepts everything and shows nothing, which is the behaviour that lets a
/// test drive tray handling without a desktop — and is *not* what a real backend
/// should do, since a real one has to report the session's actual answer.
pub fn provide_headless(services: &mut Services) {
    services.provide::<dyn Tray>(Rc::new(HeadlessTray::new()));
    services.provide::<dyn GlobalHotkeys>(Rc::new(HeadlessGlobalHotkeys::new()));
    services.provide::<dyn ContextMenus>(Rc::new(HeadlessContextMenus::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::NamedKey;

    fn chord() -> Hotkey {
        Hotkey::new(
            Modifiers::CONTROL.union(Modifiers::SHIFT),
            LogicalKey::Character("k".into()),
        )
    }

    #[test]
    fn a_separator_does_not_take_a_row_of_its_own() {
        // The decision, as a test. If a separator ever becomes a variant, the
        // ids stop lining up with the rows and this is what notices.
        let items = [
            NativeMenuItem::new(1, "Open").then_separator(),
            NativeMenuItem::new(2, "Quit"),
        ];
        assert_eq!(items.len(), 2, "two commands are two rows, divider or not");
        assert_eq!(items[0].id, MenuId(1));
        assert_eq!(items[1].id, MenuId(2));
        assert!(items[0].separator_after);
    }

    #[test]
    fn a_tray_click_is_a_line_in_a_test() {
        let tray = HeadlessTray::new();
        tray.set_menu(&[NativeMenuItem::new(7, "Quit")])
            .expect("headless accepts a menu");
        assert!(tray.take_activated().is_none(), "nothing clicked yet");

        tray.push_activated(MenuId(7));
        assert_eq!(tray.take_activated(), Some(MenuId(7)));
        assert!(
            tray.take_activated().is_none(),
            "an activation is delivered once — a take, not a read"
        );
    }

    #[test]
    fn a_taken_chord_is_refused_rather_than_silently_shared() {
        // The case that decides whether an application starts on a busy desktop.
        let hotkeys = HeadlessGlobalHotkeys::new();
        let first = hotkeys.register(&chord()).expect("nobody holds it yet");

        let Err(error) = hotkeys.register(&chord()) else {
            panic!("two owners of one machine-wide chord is not a thing");
        };
        assert!(
            !error.is_permanent(),
            "losing a chord is recoverable by asking for a different one, so it \
             must not read as Unsupported"
        );

        hotkeys.unregister(first).expect("registered a moment ago");
        assert_eq!(hotkeys.held(), 0);
        hotkeys
            .register(&chord())
            .expect("released, so claimable again");
    }

    #[test]
    fn releasing_an_unheld_hotkey_says_so() {
        let hotkeys = HeadlessGlobalHotkeys::new();
        let id = hotkeys.register(&chord()).expect("free");
        hotkeys.unregister(id).expect("held");
        assert!(
            hotkeys.unregister(id).is_err(),
            "releasing twice must not quietly succeed — the second call would \
             otherwise look like it freed a chord somebody else now holds"
        );
    }

    #[test]
    fn a_hotkey_press_arrives_by_polling() {
        let hotkeys = HeadlessGlobalHotkeys::new();
        let id = hotkeys.register(&chord()).expect("free");
        assert!(hotkeys.take_pressed().is_none());
        hotkeys.push_pressed(id);
        assert_eq!(hotkeys.take_pressed(), Some(id));
        assert!(hotkeys.take_pressed().is_none());
    }

    #[test]
    fn a_context_menu_answers_later_rather_than_blocking() {
        let menus = HeadlessContextMenus::new();
        let items = [
            NativeMenuItem::new(1, "Copy"),
            NativeMenuItem::new(2, "Cut"),
        ];

        menus
            .show(&items, Offset::new(120.0, 40.0))
            .expect("headless shows anything");
        let (shown, at) = menus.shown().expect("a menu is up");
        assert_eq!(shown.len(), 2);
        assert_eq!(at, Offset::new(120.0, 40.0));

        // `show` returned without a choice — that is the property. The choice
        // comes on a later poll.
        assert!(menus.take_chosen().is_none());
        menus.push_chosen(MenuId(2));
        assert_eq!(menus.take_chosen(), Some(MenuId(2)));
    }

    #[test]
    fn a_dismissed_menu_is_not_an_error() {
        let menus = HeadlessContextMenus::new();
        menus
            .show(&[NativeMenuItem::new(1, "Copy")], Offset::ZERO)
            .expect("shown");
        assert!(
            menus.take_chosen().is_none(),
            "walking away from a menu is ordinary, and must not read as failure"
        );
    }

    #[test]
    fn all_three_arrive_through_the_ordinary_seam() {
        // The §A claim, checked on these three: nothing here needs a field on
        // `Services` or a line in that file.
        let mut services = Services::new();
        provide_headless(&mut services);

        assert!(services.get::<dyn Tray>().is_some());
        assert!(services.get::<dyn GlobalHotkeys>().is_some());
        assert!(services.get::<dyn ContextMenus>().is_some());
    }

    #[test]
    fn a_disabled_row_is_still_a_row() {
        let item = NativeMenuItem::new(3, "Update").disabled();
        assert!(!item.enabled);
        assert_eq!(
            item.label, "Update",
            "greyed out still says what is unavailable, which is the point of \
             showing it at all"
        );
    }

    #[test]
    fn a_named_key_chord_is_expressible() {
        // Not every global hotkey is a letter — F-keys and media keys are most
        // of the real ones.
        let hotkeys = HeadlessGlobalHotkeys::new();
        let chord = Hotkey::new(Modifiers::META, LogicalKey::Named(NamedKey::Escape));
        assert!(hotkeys.register(&chord).is_ok());
    }
}
