//! Real backends for the desktop capabilities that live outside the window.
//!
//! `vieww_foundation::desktop` defines the seams — [`Tray`], [`GlobalHotkeys`]
//! and [`ContextMenus`](vieww_foundation::desktop::ContextMenus) — and ships
//! headless doubles for them. This module is the half that talks to an actual
//! desktop, and it exists only with the `desktop-services` feature on.
//!
//! # What is here, and what is not
//!
//! | capability | Windows | macOS | Linux |
//! |---|---|---|---|
//! | [`Tray`] | yes | yes | yes, over a GTK thread |
//! | [`GlobalHotkeys`] | yes | yes | X11 only |
//! | `ContextMenus` | **not registered** | **not registered** | **not registered** |
//!
//! **`ContextMenus` is deliberately absent rather than stubbed.** A native
//! context menu has to be parented to a real window — `TrackPopupMenu` takes an
//! `HWND`, `NSMenu::popUpContextMenu` takes an `NSView` — and the service is
//! built before any window exists, so wiring it means handing the runner's
//! window handle back to a service the application already holds. That is a
//! piece of plumbing rather than a guess, and it is not written yet. An
//! application asking for the service gets `None`, which the seam documents as
//! the honest answer for a capability a platform does not provide, and
//! `provide_headless` is still there for tests.
//!
//! On **Linux it will stay absent**, and that is a finding rather than a gap:
//! X11 and Wayland have no notion of a menu. What every "native" Linux menu
//! actually is, is a toolkit like GTK drawing one — so vieww drawing its own `Menu`
//! through the new `Overlay` is not a lesser substitute, it *is* what the
//! platform offers, and it comes with this framework's own accessibility tree
//! rather than a toolkit's.
//!
//! # The GTK thread
//!
//! `tray-icon` reaches the Linux status area through libappindicator, which
//! requires a GTK main loop to be running somewhere in the process. winit's loop
//! is not one and cannot be made into one, so the tray lives on a thread of its
//! own and is driven by a channel. That is what every Rust application with a
//! tray does today; it is a property of the Linux status-area protocol stack
//! rather than of this design.
//!
//! Menu clicks do **not** come back over that channel: `muda` publishes them on
//! a process-wide crossbeam channel that any thread may read, so
//! [`Tray::take_activated`] reads it directly and the polling contract in
//! `vieww_foundation::desktop` is unchanged.

use std::cell::RefCell;
use std::rc::Rc;

use vieww_foundation::desktop::{GlobalHotkeys, Hotkey, HotkeyId, MenuId, NativeMenuItem, Tray};
use vieww_foundation::{Color, IconData, LogicalKey, Modifiers, NamedKey, ServiceError, Services};

use crate::raster;

/// `muda`, reached **through `tray-icon`** rather than as a dependency of ours.
///
/// `TrayIcon::set_menu` takes a `Box<dyn tray_icon::menu::ContextMenu>`, and
/// `tray_icon::menu` is `pub use muda::*`. Depending on `muda` directly as well
/// would mean two version requirements that cargo is free to resolve to two
/// different crates — and the failure is not a version error, it is
/// `muda::Menu` not implementing `ContextMenu` because it is a *different*
/// `muda::Menu`.
///
/// Aliasing rather than rewriting every path: the code reads the same, and this
/// comment is where the reason lives.
use tray_icon::menu as muda;

/// How many pixels across a tray icon is rendered.
///
/// 32 rather than the 16 a tray usually shows, because every desktop in use
/// scales the status area on a HiDPI screen and all three take a larger source
/// happily. Rendering at 16 and letting the shell upscale is the one choice that
/// looks wrong everywhere.
const TRAY_ICON_PIXELS: u32 = 32;

/// What colour to draw a tray icon in.
///
/// [`IconData`] carries a shape and no colour, so somebody has to choose, and
/// the right choice is per-platform convention rather than anything about the
/// icon:
///
/// - **macOS** wants a *template* image: black with an alpha channel, which the
///   system then recolours for the light or dark menu bar. Drawing white there
///   produces an invisible icon on a light bar.
/// - **Windows and Linux** get no such treatment and are drawn as given. White
///   is the safer default because the taskbar and the common Linux panels are
///   dark far more often than not.
///
/// This is the one place in the module where being wrong is merely ugly rather
/// than broken, and it is stated here so that an application that disagrees has
/// something to point at when asking for the setting.
const fn tray_foreground() -> Color {
    #[cfg(target_os = "macos")]
    {
        Color::BLACK
    }
    #[cfg(not(target_os = "macos"))]
    {
        Color::WHITE
    }
}

/// Build a `muda` menu from the framework's description of one.
///
/// The id round trip is the part worth reading. `MenuId` here is a `u32` the
/// application chose; `muda`'s is a `String`. Formatting one into the other and
/// parsing it back is not a hack — it is the only mapping that survives a menu
/// being rebuilt, which is exactly what `NativeMenuItem`'s own documentation
/// says the numeric id is for.
fn build_menu(items: &[NativeMenuItem]) -> Result<muda::Menu, ServiceError> {
    let menu = muda::Menu::new();
    for item in items {
        let id = muda::MenuId::new(item.id.0.to_string());
        // A checkable row and a plain row are different types in `muda`, and
        // which one to build is decided by whether the application ever ticks
        // it. `checked: false` still builds a check item, so a row that toggles
        // keeps its space in the gutter instead of moving when it is first
        // ticked.
        let entry: Box<dyn muda::IsMenuItem> = if item.checked {
            Box::new(muda::CheckMenuItem::with_id(
                id,
                &item.label,
                item.enabled,
                true,
                None,
            ))
        } else {
            Box::new(muda::MenuItem::with_id(id, &item.label, item.enabled, None))
        };
        menu.append(entry.as_ref())
            .map_err(|error| ServiceError::failed(format!("menu row {:?}: {error}", item.id)))?;

        // **After the row, never as a row of its own.** A separator that
        // occupied a position would put a hole in the indices the platform
        // reports back, and the bug that follows is an application running the
        // wrong command from its own tray menu. `NativeMenuItem` makes it a
        // property for exactly this reason.
        if item.separator_after {
            menu.append(&muda::PredefinedMenuItem::separator())
                .map_err(|error| ServiceError::failed(format!("separator: {error}")))?;
        }
    }
    Ok(menu)
}

/// Drain whatever `muda` has published since the last look.
///
/// `try_recv`, so a frame that asks and finds nothing costs one atomic load.
fn take_menu_event() -> Option<MenuId> {
    muda::MenuEvent::receiver()
        .try_recv()
        .ok()
        .and_then(|event| event.id.0.parse::<u32>().ok())
        .map(MenuId)
}

/// A tray icon, as the platform actually draws one.
pub struct PlatformTray {
    inner: Rc<TrayInner>,
}

impl std::fmt::Debug for PlatformTray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformTray").finish_non_exhaustive()
    }
}

#[cfg(not(target_os = "linux"))]
struct TrayInner {
    /// `None` until something is shown. Held rather than rebuilt because
    /// dropping a `TrayIcon` removes it from the status area.
    icon: RefCell<Option<tray_icon::TrayIcon>>,
}

#[cfg(target_os = "linux")]
struct TrayInner {
    /// Commands for the GTK thread. See the module docs.
    commands: std::sync::mpsc::Sender<TrayCommand>,
}

/// What the Linux GTK thread can be asked to do.
#[cfg(target_os = "linux")]
enum TrayCommand {
    Icon(Vec<u8>, u32),
    Tooltip(String),
    Menu(Vec<NativeMenuItem>),
    Remove,
}

impl PlatformTray {
    /// Claim a place in the status area, lazily.
    ///
    /// Nothing appears until [`set_icon`](Tray::set_icon) is called, which is
    /// what the trait promises and also what keeps an application that merely
    /// *registered* the service from putting an empty icon on somebody's panel.
    #[must_use]
    pub fn new() -> Self {
        #[cfg(target_os = "linux")]
        {
            let (commands, orders) = std::sync::mpsc::channel();
            // A thread rather than the winit loop, because libappindicator needs
            // a GTK main loop and winit's is not one. It outlives the service on
            // purpose: the icon has to stay up for as long as the process does,
            // and a thread that exited would take it down.
            std::thread::spawn(move || gtk_tray_thread(&orders));
            Self {
                inner: Rc::new(TrayInner { commands }),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self {
                inner: Rc::new(TrayInner {
                    icon: RefCell::new(None),
                }),
            }
        }
    }
}

impl Default for PlatformTray {
    fn default() -> Self {
        Self::new()
    }
}

/// Own the tray on a GTK main loop, and take orders over `orders`.
#[cfg(target_os = "linux")]
fn gtk_tray_thread(orders: &std::sync::mpsc::Receiver<TrayCommand>) {
    if gtk::init().is_err() {
        // No GTK, no status area. Logged once rather than retried: a session
        // without it will not grow one, and an application that cannot show a
        // tray icon should still run.
        log::warn!(
            "vieww: GTK would not start, so there is no tray on this session. \
             The rest of the application is unaffected."
        );
        return;
    }

    let mut icon: Option<tray_icon::TrayIcon> = None;
    loop {
        // Drain everything waiting, then let GTK breathe. Polling rather than
        // blocking on the channel because the GTK loop has to keep running for
        // the icon to respond at all.
        while let Ok(command) = orders.try_recv() {
            apply(&mut icon, command);
        }
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

#[cfg(target_os = "linux")]
fn apply(icon: &mut Option<tray_icon::TrayIcon>, command: TrayCommand) {
    match command {
        TrayCommand::Icon(rgba, size) => {
            let Ok(image) = tray_icon::Icon::from_rgba(rgba, size, size) else {
                log::error!("vieww: the tray refused the rasterised icon");
                return;
            };
            match icon {
                Some(tray) => {
                    let _ = tray.set_icon(Some(image));
                }
                None => match tray_icon::TrayIconBuilder::new().with_icon(image).build() {
                    Ok(tray) => *icon = Some(tray),
                    Err(error) => log::error!("vieww: no tray icon: {error}"),
                },
            }
        }
        TrayCommand::Tooltip(text) => {
            if let Some(tray) = icon {
                let _ = tray.set_tooltip(Some(text));
            }
        }
        TrayCommand::Menu(items) => {
            if let (Some(tray), Ok(menu)) = (icon.as_ref(), build_menu(&items)) {
                tray.set_menu(Some(Box::new(menu)));
            }
        }
        TrayCommand::Remove => *icon = None,
    }
}

impl Tray for PlatformTray {
    fn set_icon(&self, icon: &IconData) -> Result<(), ServiceError> {
        let rgba = raster::rasterise(icon, TRAY_ICON_PIXELS, tray_foreground());

        #[cfg(target_os = "linux")]
        {
            self.inner
                .commands
                .send(TrayCommand::Icon(rgba, TRAY_ICON_PIXELS))
                .map_err(|_| ServiceError::failed("the tray thread is gone"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let image = tray_icon::Icon::from_rgba(rgba, TRAY_ICON_PIXELS, TRAY_ICON_PIXELS)
                .map_err(|error| ServiceError::failed(format!("tray icon: {error}")))?;
            let mut held = self.inner.icon.borrow_mut();
            match held.as_ref() {
                Some(tray) => tray
                    .set_icon(Some(image))
                    .map_err(|error| ServiceError::failed(format!("tray icon: {error}"))),
                None => {
                    let builder = tray_icon::TrayIconBuilder::new().with_icon(image);
                    // macOS recolours a template image for the menu bar it is
                    // actually on, which is the only way one icon can be right
                    // on both a light and a dark bar.
                    //
                    // A shadowing `let` under the `cfg` rather than a `mut` and
                    // an assignment: `mut` would be unused on Windows, and
                    // `-D warnings` does not forgive that.
                    #[cfg(target_os = "macos")]
                    let builder = builder.with_icon_as_template(true);
                    let tray = builder
                        .build()
                        .map_err(|error| ServiceError::failed(format!("tray icon: {error}")))?;
                    *held = Some(tray);
                    Ok(())
                }
            }
        }
    }

    fn set_tooltip(&self, text: &str) -> Result<(), ServiceError> {
        #[cfg(target_os = "linux")]
        {
            self.inner
                .commands
                .send(TrayCommand::Tooltip(text.to_owned()))
                .map_err(|_| ServiceError::failed("the tray thread is gone"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let held = self.inner.icon.borrow();
            let Some(tray) = held.as_ref() else {
                return Err(ServiceError::failed(
                    "there is no tray icon yet — call set_icon first",
                ));
            };
            tray.set_tooltip(Some(text))
                .map_err(|error| ServiceError::failed(format!("tray tooltip: {error}")))
        }
    }

    fn set_menu(&self, items: &[NativeMenuItem]) -> Result<(), ServiceError> {
        #[cfg(target_os = "linux")]
        {
            self.inner
                .commands
                .send(TrayCommand::Menu(items.to_vec()))
                .map_err(|_| ServiceError::failed("the tray thread is gone"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let menu = build_menu(items)?;
            let held = self.inner.icon.borrow();
            let Some(tray) = held.as_ref() else {
                return Err(ServiceError::failed(
                    "there is no tray icon yet — call set_icon first",
                ));
            };
            tray.set_menu(Some(Box::new(menu)));
            Ok(())
        }
    }

    fn take_activated(&self) -> Option<MenuId> {
        // Read here on whichever thread is asking, rather than routed back from
        // the GTK thread on Linux: `muda` publishes to a process-wide channel.
        take_menu_event()
    }

    fn remove(&self) -> Result<(), ServiceError> {
        #[cfg(target_os = "linux")]
        {
            self.inner
                .commands
                .send(TrayCommand::Remove)
                .map_err(|_| ServiceError::failed("the tray thread is gone"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Dropping it is how a `TrayIcon` leaves the status area.
            *self.inner.icon.borrow_mut() = None;
            Ok(())
        }
    }
}

/// Machine-wide key combinations.
pub struct PlatformGlobalHotkeys {
    manager: global_hotkey::GlobalHotKeyManager,
    /// What each issued id was registered as, so `unregister` can name it again
    /// — the underlying API takes the chord back, not a handle.
    held: RefCell<Vec<(HotkeyId, global_hotkey::hotkey::HotKey)>>,
}

impl std::fmt::Debug for PlatformGlobalHotkeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformGlobalHotkeys")
            .field("held", &self.held.borrow().len())
            .finish_non_exhaustive()
    }
}

impl PlatformGlobalHotkeys {
    /// # Errors
    ///
    /// [`ServiceError::Failed`] where no hotkey manager can be created at all —
    /// a headless session, or Wayland, where there is deliberately no way for
    /// one client to claim a chord from another.
    ///
    /// `Failed` rather than `Unsupported` even for Wayland, because the caller
    /// this reaches is [`provide`], which responds by not registering the
    /// service at all. An application therefore sees the capability as absent,
    /// which is the distinction it actually acts on — and reporting
    /// `Unsupported` here would encode a claim about the *platform* from what is
    /// really a fact about this session.
    pub fn new() -> Result<Self, ServiceError> {
        let manager = global_hotkey::GlobalHotKeyManager::new().map_err(|error| {
            ServiceError::failed(format!("no global hotkey manager on this session: {error}"))
        })?;
        Ok(Self {
            manager,
            held: RefCell::new(Vec::new()),
        })
    }
}

/// Translate a chord into the physical key code the OS registers against.
///
/// # Why a *physical* code from a *logical* key
///
/// Because that is the mismatch the platform forces. vieww's [`LogicalKey`] is
/// what the key produced after the layout — which is the right thing for typing
/// — and every global-hotkey API on every OS registers a scancode. There is no
/// lossless mapping, so this one takes the pragmatic route used by every
/// application that offers configurable shortcuts: the ASCII letters and digits
/// map to their US-layout positions, and named keys map exactly.
///
/// The consequence is worth stating rather than hiding: on an AZERTY keyboard,
/// asking for `Ctrl+A` claims the key in the QWERTY `A` position. Every
/// screenshot tool on Linux behaves the same way, and the alternative — refusing
/// letters entirely — is worse.
fn code_for(key: &LogicalKey) -> Option<global_hotkey::hotkey::Code> {
    use global_hotkey::hotkey::Code;

    match key {
        LogicalKey::Character(text) => {
            let mut chars = text.chars();
            let (Some(first), None) = (chars.next(), chars.next()) else {
                // A grapheme that is not one `char` is not a key anybody can
                // register — an emoji, or a composed sequence.
                return None;
            };
            Some(match first.to_ascii_lowercase() {
                'a' => Code::KeyA,
                'b' => Code::KeyB,
                'c' => Code::KeyC,
                'd' => Code::KeyD,
                'e' => Code::KeyE,
                'f' => Code::KeyF,
                'g' => Code::KeyG,
                'h' => Code::KeyH,
                'i' => Code::KeyI,
                'j' => Code::KeyJ,
                'k' => Code::KeyK,
                'l' => Code::KeyL,
                'm' => Code::KeyM,
                'n' => Code::KeyN,
                'o' => Code::KeyO,
                'p' => Code::KeyP,
                'q' => Code::KeyQ,
                'r' => Code::KeyR,
                's' => Code::KeyS,
                't' => Code::KeyT,
                'u' => Code::KeyU,
                'v' => Code::KeyV,
                'w' => Code::KeyW,
                'x' => Code::KeyX,
                'y' => Code::KeyY,
                'z' => Code::KeyZ,
                '0' => Code::Digit0,
                '1' => Code::Digit1,
                '2' => Code::Digit2,
                '3' => Code::Digit3,
                '4' => Code::Digit4,
                '5' => Code::Digit5,
                '6' => Code::Digit6,
                '7' => Code::Digit7,
                '8' => Code::Digit8,
                '9' => Code::Digit9,
                _ => return None,
            })
        }
        LogicalKey::Named(named) => Some(match named {
            NamedKey::Enter => Code::Enter,
            NamedKey::Tab => Code::Tab,
            NamedKey::Space => Code::Space,
            NamedKey::Backspace => Code::Backspace,
            NamedKey::Delete => Code::Delete,
            NamedKey::Escape => Code::Escape,
            NamedKey::ArrowLeft => Code::ArrowLeft,
            NamedKey::ArrowRight => Code::ArrowRight,
            NamedKey::ArrowUp => Code::ArrowUp,
            NamedKey::ArrowDown => Code::ArrowDown,
            NamedKey::Home => Code::Home,
            NamedKey::End => Code::End,
            NamedKey::PageUp => Code::PageUp,
            NamedKey::PageDown => Code::PageDown,
            NamedKey::Insert => Code::Insert,
            // A modifier on its own is not a chord. Refused rather than
            // approximated: registering "Shift" machine-wide would take the
            // key away from every other application on the desktop.
            NamedKey::Shift
            | NamedKey::Control
            | NamedKey::Alt
            | NamedKey::Meta
            | NamedKey::CapsLock => return None,
            // `NamedKey` is `#[non_exhaustive]`, so this arm is required even
            // though every variant above is named. It is **not** the same
            // statement as the modifiers arm: that one says "this key must
            // never be claimed machine-wide", and this one says "this key was
            // added to the framework after this mapping was written".
            //
            // Refusing is the right answer to both, and it is worth knowing
            // that the compiler cannot tell you when a new variant lands here.
            // Anything added to `NamedKey` that has a physical key position —
            // an F-key, a media key — needs a line above, and nothing will
            // point at this function to say so.
            _ => return None,
        }),
        LogicalKey::Unidentified => None,
    }
}

fn modifiers_for(modifiers: Modifiers) -> global_hotkey::hotkey::Modifiers {
    use global_hotkey::hotkey::Modifiers as Native;

    let mut native = Native::empty();
    if modifiers.contains(Modifiers::SHIFT) {
        native |= Native::SHIFT;
    }
    if modifiers.contains(Modifiers::CONTROL) {
        native |= Native::CONTROL;
    }
    if modifiers.contains(Modifiers::ALT) {
        native |= Native::ALT;
    }
    if modifiers.contains(Modifiers::META) {
        native |= Native::META;
    }
    native
}

impl GlobalHotkeys for PlatformGlobalHotkeys {
    fn register(&self, hotkey: &Hotkey) -> Result<HotkeyId, ServiceError> {
        let Some(code) = code_for(&hotkey.key) else {
            return Err(ServiceError::failed(
                "that key cannot be claimed machine-wide — a modifier on its \
                 own, or a character with no key position",
            ));
        };
        let native =
            global_hotkey::hotkey::HotKey::new(Some(modifiers_for(hotkey.modifiers)), code);

        // **`Failed`, not `Unsupported`.** Losing a chord to a screenshot tool
        // or to a second copy of this application is ordinary, and an app that
        // treated it as fatal would not start on a busy desktop. The seam's own
        // documentation makes this the load-bearing distinction.
        self.manager.register(native).map_err(|error| {
            ServiceError::failed(format!("that combination is already claimed: {error}"))
        })?;

        let id = HotkeyId::from_raw(native.id());
        self.held.borrow_mut().push((id, native));
        Ok(id)
    }

    fn unregister(&self, id: HotkeyId) -> Result<(), ServiceError> {
        let mut held = self.held.borrow_mut();
        let Some(index) = held.iter().position(|(held, _)| *held == id) else {
            return Err(ServiceError::failed("that hotkey is not registered"));
        };
        let (_, native) = held.remove(index);
        self.manager
            .unregister(native)
            .map_err(|error| ServiceError::failed(format!("releasing the chord failed: {error}")))
    }

    fn take_pressed(&self) -> Option<HotkeyId> {
        // Presses arrive on the platform's own thread and are queued; this is
        // the poll the seam is built around. `Pressed` only — a chord reports
        // both edges and firing an application's action twice per press is the
        // bug this filter exists to prevent.
        loop {
            let event = global_hotkey::GlobalHotKeyEvent::receiver()
                .try_recv()
                .ok()?;
            if event.state == global_hotkey::HotKeyState::Pressed {
                return Some(HotkeyId::from_raw(event.id));
            }
        }
    }
}

/// Register the real desktop services this platform can provide.
///
/// Called by [`services::platform`](crate::services::platform) when the
/// `desktop-services` feature is on. A capability that cannot be created is
/// **left unregistered** rather than registered as something that always fails:
/// an application checks once with `Services::get` and hides the affordance,
/// which is the whole point of the seam returning `Option`.
pub fn provide(services: &mut Services) {
    services.provide::<dyn Tray>(Rc::new(PlatformTray::new()));

    match PlatformGlobalHotkeys::new() {
        Ok(hotkeys) => services.provide::<dyn GlobalHotkeys>(Rc::new(hotkeys)),
        // Wayland is the common case and is not a defect: there is deliberately
        // no way for one client to claim a chord from another, which is a
        // security property rather than a missing feature.
        Err(error) => log::info!("vieww: no global hotkeys on this session ({error})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_maps_to_its_key_position() {
        use global_hotkey::hotkey::Code;
        assert_eq!(
            code_for(&LogicalKey::Character("k".into())),
            Some(Code::KeyK)
        );
        assert_eq!(
            code_for(&LogicalKey::Character("K".into())),
            Some(Code::KeyK),
            "case is a layout artefact, not a different key"
        );
    }

    #[test]
    fn no_two_named_keys_map_to_the_same_code() {
        // The realistic bug in a fifteen-arm table is a copy-paste — `ArrowLeft`
        // mapping to `Code::ArrowRight` — and it is invisible by inspection
        // because both sides read correctly. Nothing else here would catch it:
        // the chord registers, the OS accepts it, and the application's shortcut
        // fires on the wrong key.
        let named = [
            NamedKey::Enter,
            NamedKey::Tab,
            NamedKey::Space,
            NamedKey::Backspace,
            NamedKey::Delete,
            NamedKey::Escape,
            NamedKey::ArrowLeft,
            NamedKey::ArrowRight,
            NamedKey::ArrowUp,
            NamedKey::ArrowDown,
            NamedKey::Home,
            NamedKey::End,
            NamedKey::PageUp,
            NamedKey::PageDown,
            NamedKey::Insert,
        ];

        let mut seen = Vec::new();
        for key in named {
            let code = code_for(&LogicalKey::Named(key))
                .unwrap_or_else(|| panic!("{key:?} has no key position, but it is not a modifier"));
            assert!(
                !seen.contains(&code),
                "{key:?} maps to {code:?}, which another key already claimed"
            );
            seen.push(code);
        }
    }

    #[test]
    fn a_modifier_alone_is_refused() {
        // Registering this would take Shift away from every application on the
        // desktop, which is not something an accidental call should be able to
        // do.
        assert_eq!(code_for(&LogicalKey::Named(NamedKey::Shift)), None);
    }

    #[test]
    fn a_multi_char_grapheme_is_refused_rather_than_truncated() {
        // Truncating would claim the chord for whatever the first `char`
        // happened to be, which is a shortcut the application never asked for.
        assert_eq!(code_for(&LogicalKey::Character("é\u{0301}".into())), None);
        assert_eq!(code_for(&LogicalKey::Character("🙂".into())), None);
    }

    #[test]
    fn every_modifier_survives_translation() {
        use global_hotkey::hotkey::Modifiers as Native;
        let all = Modifiers::SHIFT
            .union(Modifiers::CONTROL)
            .union(Modifiers::ALT)
            .union(Modifiers::META);
        let native = modifiers_for(all);
        assert!(native.contains(Native::SHIFT));
        assert!(native.contains(Native::CONTROL));
        assert!(native.contains(Native::ALT));
        assert!(native.contains(Native::META));
    }

    /// **Not on Linux**, and the reason is the same one the GTK thread exists
    /// for: `muda`'s GTK backend builds real widgets, so constructing a `Menu`
    /// without an initialised GTK aborts the test process rather than failing
    /// the test. Everything this asserts is platform-independent mapping logic;
    /// the Windows and macOS runs cover it, and the Linux GTK thread builds its
    /// menus through the same `build_menu`.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_separator_does_not_become_a_row() {
        // The id mapping's load-bearing property, checked against the real menu
        // builder rather than against the description of one: `muda` is what
        // reports a choice back, and a separator occupying a position is how an
        // application ends up running the row below the one that was clicked.
        let items = [
            NativeMenuItem::new(1, "Open").then_separator(),
            NativeMenuItem::new(2, "Quit"),
        ];
        let menu = build_menu(&items).expect("two rows and a divider");
        assert_eq!(
            menu.items().len(),
            3,
            "two commands and one separator, and the separator carries no id"
        );
    }
}
