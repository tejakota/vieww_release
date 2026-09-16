//! Assertions that only a desktop can answer, run inside the demo app.
//!
//! The sibling of `device_tests.rs`, and deliberately not a copy of it. That
//! file asks what must be true on *any* device — a frame arrived, the metrics
//! reached the tree, the budget was met — and every one of those questions is
//! just as good on a workstation, which is why `examples/desktop.rs` installs
//! **both** and this file adds only what a phone cannot be asked:
//!
//! | question | why it is not in `device_tests.rs` |
//! |---|---|
//! | does a second window open, and does a dialog close with its parent | `PlatformWindows` answers `Unsupported` on Android and iOS by design — an application does not own its windows there |
//! | is the shortcut modifier the one this OS expects | Command on macOS, Control on Linux and Windows. A phone has no modifier key |
//! | did the platform give us the *conventional* data directory | `data_dir` has a different documented answer per desktop, and a silent fall back to the temporary directory that loses settings |
//! | does the pasteboard round-trip | mobile pasteboards need an activity or a `UIPasteboard`; the desktop one needs a live X11/Wayland/Win32 connection that only exists while a window does |
//! | what locale did the system report | the same call on a phone answers from the device settings, and is already exercised there |
//!
//! # Why it reports through `DeviceSuite`
//!
//! [`DeviceSuite::check`](super::device_tests::DeviceSuite::check) and
//! `observe` already exist for exactly this — "record a check from outside the
//! tree" — and they were written with a note saying the iOS entry point would
//! one day have platform questions of its own to ask. This is the desktop
//! answering first. Sharing them means **one** report, on **one** sentinel
//! stream, with one `PASS`/`FAIL` verdict at the end of it, so
//! `ci/certify/desktop-suite.sh` greps the same thing `ci/mobile/device-suite.sh` does and
//! neither script has to learn a second format.
//!
//! # What is here and what the script asserts
//!
//! The same split as the device suite, for the same reason. In here: what must
//! hold on **any** desktop, so a failure is a bug. A machine with one monitor is
//! a real machine and a machine at 1x is a real machine, so neither "there are
//! two displays" nor "the density is not 1" is asserted here — those are facts
//! about the hardware in front of the operator, and `ci/certify/desktop-suite.sh`
//! asserts them when the operator says they are true.

use std::cell::RefCell;
use std::rc::Rc;

use vieww_foundation::{Clipboard, Modifiers, Size, Storage, TargetPlatform};
use vieww_platform_winit::clipboard::PlatformClipboard;
use vieww_platform_winit::{data_dir, locale, FileStorage, PlatformWindows};
use vieww_render::{FrameDriver, WindowSpec, Windows, WindowsExt};
use vieww_widget::prelude::*;

use crate::device_tests::DeviceSuite;

/// A key no window was ever minted under.
///
/// `PlatformWindows` counts up from 1 for every window it opens, so a number
/// this far out cannot collide with a live one however long the app runs — and
/// the check it is for wants a key that is *syntactically* fine and refers to
/// nothing, which is precisely the mistake an application makes when it keeps a
/// key past the window's life.
const NO_SUCH_WINDOW: u64 = 0xDEAD_BEEF;

/// The desktop half of the suite.
///
/// Holds the [`DeviceSuite`] it reports through rather than a `Results` of its
/// own — see the module docs.
#[derive(Debug, Clone)]
pub(crate) struct DesktopSuite {
    device: DeviceSuite,
    /// The window this suite opened, once it has one. Shared because the hook
    /// that opens it and the hook that closes it are separate closures on
    /// separate frames.
    opened: Rc<RefCell<Option<vieww_render::WindowKey>>>,
}

impl DesktopSuite {
    pub(crate) fn new(device: &DeviceSuite) -> Self {
        Self {
            device: device.clone(),
            opened: Rc::new(RefCell::new(None)),
        }
    }

    /// Everything answerable before a window exists.
    ///
    /// Called from `main` rather than from a hook, because none of it needs a
    /// frame and a check that runs 600 times to produce one line is a check that
    /// makes the report slower to read for no gain.
    pub(crate) fn environment(&self) {
        self.platform();
        self.shortcut_modifier();
        self.storage();
        self.locale();
    }

    /// The build's idea of which platform it is, against the compiler's.
    ///
    /// # Why this is worth a line
    ///
    /// `TargetPlatform::current` is `const` and resolved from `cfg`, and every
    /// behavioural branch in the framework — the shortcut modifier, the slider
    /// thumb, the dialog button order, the scroll physics — hangs off it. If it
    /// is ever wrong for a target, nothing fails: the application simply behaves
    /// like a different operating system, correctly and consistently, and a
    /// person has to notice that the buttons are the wrong way round.
    ///
    /// `std::env::consts::OS` is the same `cfg` read through a different door,
    /// so this is cheap. It is also the check that fails loudest the day someone
    /// adds a platform to the enum and forgets an arm.
    fn platform(&self) {
        let current = TargetPlatform::current();
        self.device.check(
            "the_target_platform_matches_the_operating_system",
            current.name() == std::env::consts::OS,
            format!(
                "TargetPlatform::current() is {} and target_os is {}",
                current.name(),
                std::env::consts::OS
            ),
        );
    }

    /// Command on Apple, Control everywhere else.
    ///
    /// The single most visible cross-platform convention there is, and the one a
    /// Linux-only test suite can never catch getting it wrong: on the machine it
    /// was written on, `shortcut_for` returning `CONTROL` is right for the wrong
    /// reason if the branch is inverted, because `is_apple` is false there
    /// either way.
    fn shortcut_modifier(&self) {
        let platform = TargetPlatform::current();
        let modifier = Modifiers::shortcut_for(platform);
        let expected = if platform.is_apple() {
            Modifiers::META
        } else {
            Modifiers::CONTROL
        };
        // Named rather than `{modifier:?}`, which prints the bitflags' raw value
        // — `Modifiers(2)` in a report is a number the reader has to go and look
        // up, on the one line where the whole point is which key it is.
        let name = |modifiers: Modifiers| {
            if modifiers == Modifiers::META {
                "Command/Super"
            } else if modifiers == Modifiers::CONTROL {
                "Control"
            } else {
                "something else"
            }
        };
        self.device.check(
            "the_shortcut_modifier_matches_the_platform",
            modifier == expected,
            format!(
                "{} on {}, expected {}",
                name(modifier),
                platform.name(),
                name(expected)
            ),
        );
    }

    /// A directory this application may write to, in the place this platform
    /// keeps such things.
    ///
    /// Three separate questions, because they fail separately and a single
    /// boolean would hide which:
    ///
    /// 1. **Is it writable.** A round-trip through `FileStorage`, not a
    ///    permission bit — the bit is advisory on Windows and says nothing about
    ///    a directory that does not exist yet.
    /// 2. **Is it the conventional place.** `data_dir`'s own doc comment names
    ///    one path per platform; this asserts the path it returned is under
    ///    that. A settings file in the wrong directory works perfectly and is
    ///    invisible until a user asks why their settings did not migrate.
    /// 3. **Is it the fallback.** `data_dir` falls back to the temporary
    ///    directory when it cannot find a home, which keeps an application
    ///    running at the documented cost that settings may not survive a reboot.
    ///    That is the right behaviour and the wrong thing to ship without
    ///    knowing, so it is a check rather than a silence.
    fn storage(&self) {
        let dir = data_dir();
        self.device.observe("data_dir", dir.display().to_string());

        let storage = FileStorage::at(&dir);
        let value = format!("desktop-suite-{}", std::process::id());
        let wrote = storage.set("__vieww_desktop_suite", &value);
        let read = storage.get("__vieww_desktop_suite");
        // Left tidy: a suite that grows the user's real settings file every run
        // is a suite that eventually gets blamed for something.
        let _ = storage.remove("__vieww_desktop_suite");

        self.device.check(
            "the_platform_gave_us_a_writable_data_directory",
            wrote.is_ok() && matches!(&read, Ok(Some(back)) if back == &value),
            match (&wrote, &read) {
                (Err(error), _) => format!("writing to {}: {error}", dir.display()),
                (_, Err(error)) => format!("reading back from {}: {error}", dir.display()),
                (Ok(()), Ok(back)) => format!("{} round-tripped {back:?}", dir.display()),
            },
        );

        let path = dir.to_string_lossy().replace('\\', "/");
        let convention = if cfg!(target_os = "macos") {
            "Library/Application Support"
        } else if cfg!(target_os = "windows") {
            "AppData"
        } else {
            ".local/share"
        };
        // XDG_DATA_HOME is a legitimate override that can point anywhere, so on
        // Linux the convention is only asserted when nothing overrode it.
        let overridden = cfg!(target_os = "linux") && std::env::var_os("XDG_DATA_HOME").is_some();
        self.device.check(
            "the_data_directory_follows_this_platform_s_convention",
            overridden || path.contains(convention),
            if overridden {
                format!("XDG_DATA_HOME is set, so {path} is whatever it says")
            } else {
                format!("expected {convention} in {path}")
            },
        );

        let temporary = std::env::temp_dir();
        self.device.check(
            "the_data_directory_is_not_the_no_home_fallback",
            !dir.starts_with(&temporary),
            format!(
                "{} is {}under the temporary directory {}",
                dir.display(),
                if dir.starts_with(&temporary) {
                    ""
                } else {
                    "not "
                },
                temporary.display()
            ),
        );
    }

    /// What the system said about language and region.
    ///
    /// An observation and not a check: every value is legitimate, including
    /// `None` on a machine with nothing set. It is here because a text run that
    /// differs between two machines is answered by these two lines more often
    /// than by anything else in the report — locale drives digit shaping, date
    /// and number formatting, and which face a fallback chain reaches for CJK.
    fn locale(&self) {
        self.device.observe(
            "system_locale",
            locale::system().map_or_else(|| String::from("none"), |locale| format!("{locale:?}")),
        );
        self.device
            .observe("preferred_locales", format!("{:?}", locale::preferred()));
    }

    /// The pasteboard, round-tripped through the real platform connection.
    ///
    /// # Why this is a hook and not part of [`environment`](Self::environment)
    ///
    /// On X11 the clipboard is owned by a live client with a connection to the
    /// display server. Before `App::run` there is no window and, on a bare
    /// session, no connection to make — so the same code that passes from a hook
    /// fails from `main` for a reason that has nothing to do with the framework.
    ///
    /// # Why it puts back what it found
    ///
    /// Because a person is sitting at this machine. A suite that silently
    /// replaces whatever they had copied with a test nonce is a suite they stop
    /// running, and the restore is four lines.
    ///
    /// Reported once, on the first frame that reaches it.
    pub(crate) fn clipboard(&self) -> impl FnMut(&mut FrameDriver) + 'static {
        let device = self.device.clone();
        let mut done = false;
        move |_driver| {
            if done {
                return;
            }
            done = true;

            let clipboard = PlatformClipboard::new();
            let existing = clipboard.read_text().ok().flatten();
            let nonce = format!("vieww-desktop-suite-{}", std::process::id());

            let result = clipboard
                .write_text(&nonce)
                .and_then(|()| clipboard.read_text());

            match result {
                Ok(back) => device.check(
                    "the_pasteboard_round_trips",
                    back.as_deref() == Some(nonce.as_str()),
                    format!("wrote {nonce:?} and read back {back:?}"),
                ),
                Err(error) => device.check(
                    "the_pasteboard_round_trips",
                    false,
                    format!("the platform pasteboard refused: {error}"),
                ),
            }

            if let Some(existing) = existing {
                let _ = clipboard.write_text(&existing);
            }
        }
    }

    /// Open a second window and a dialog on it, then close the parent and check
    /// that the dialog went with it.
    ///
    /// # What this closes
    ///
    /// `WindowSet` — which window belongs to what, what closes with what, which
    /// window is inert — is thoroughly unit-tested as bookkeeping over keys, and
    /// that is the right place for the policy to live. What no test on that side
    /// can say is whether the **event loop applies the answers**: whether a key
    /// the policy declared doomed corresponds to a surface that actually left
    /// the screen. A dialog that outlives its parent is an orphan nothing can
    /// close, and it is invisible to every test that does not open a real
    /// window.
    ///
    /// It is also the one check here that a phone must *refuse*. Running the
    /// same entry point on Android would get `Unsupported` from `open_boxed`,
    /// which is the documented answer and not a failure — so this hook is
    /// installed only by `examples/desktop.rs`.
    ///
    /// # Why it is a staged machine
    ///
    /// A window is requested on one frame and exists on a later one — that is
    /// what `open_boxed`'s doc means by "the key is minted now and the window
    /// appears later". Asking `is_open` on the frame that asked for it would
    /// read the request rather than the window. So: open, let a frame pass,
    /// check, close, let a frame pass, check.
    ///
    /// The counter is frames rather than time because this is driven from
    /// `before_frame` and a frame is the only clock it has.
    pub(crate) fn windows(
        &self,
        windows: Rc<PlatformWindows>,
    ) -> impl FnMut(&mut FrameDriver) + 'static {
        let device = self.device.clone();
        let opened = Rc::clone(&self.opened);
        let mut frame = 0_u32;

        move |_driver| {
            frame += 1;
            match frame {
                // Not frame one. The primary window's own surface is still being
                // created there, and a second request on top of it measures the
                // startup path rather than the thing being asked about.
                30 => {
                    let refused = windows.open_boxed(
                        WindowSpec::new("vieww — orphan dialog")
                            .as_dialog(vieww_render::WindowKey::new(NO_SUCH_WINDOW)),
                        Box::new(|_key, _driver| {}),
                    );
                    device.check(
                        "a_dialog_whose_parent_is_not_open_is_refused",
                        refused.is_err(),
                        match refused {
                            Err(error) => format!("{error}"),
                            Ok(key) => format!("it opened as {key:?}, which nothing could close"),
                        },
                    );

                    match windows.open(
                        WindowSpec::new("vieww — second window").with_size(Size::new(360.0, 240.0)),
                        |_key, driver| {
                            let root: WidgetNode = Container::new()
                                .color(vieww_foundation::Color::rgb(58, 122, 246))
                                .into();
                            driver.set_root(root);
                        },
                    ) {
                        Ok(key) => *opened.borrow_mut() = Some(key),
                        Err(error) => device.check(
                            "a_second_window_opens",
                            false,
                            format!("the platform refused: {error}"),
                        ),
                    }
                }
                40 => {
                    let Some(key) = *opened.borrow() else { return };
                    device.check(
                        "a_second_window_opens",
                        windows.is_open(key),
                        format!("{key:?} is {}", state(windows.is_open(key))),
                    );
                    // On the second window rather than the primary one, so the
                    // close below tests the parent/child rule rather than
                    // quitting the application.
                    match windows.open(
                        WindowSpec::new("vieww — dialog")
                            .with_size(Size::new(280.0, 160.0))
                            .as_dialog(key),
                        |_key, driver| {
                            let root: WidgetNode = Container::new()
                                .color(vieww_foundation::Color::rgb(240, 240, 240))
                                .into();
                            driver.set_root(root);
                        },
                    ) {
                        Ok(dialog) => {
                            device.check(
                                "a_dialog_opens_on_a_live_parent",
                                true,
                                format!("{dialog:?} belongs to {key:?}"),
                            );
                            *opened.borrow_mut() = Some(key);
                            device.observe("dialog_key", format!("{dialog:?}"));
                        }
                        Err(error) => device.check(
                            "a_dialog_opens_on_a_live_parent",
                            false,
                            format!("the platform refused: {error}"),
                        ),
                    }
                }
                50 => {
                    let Some(key) = *opened.borrow() else { return };
                    // Closing the parent is what the rule is about: the dialog
                    // is never closed by name anywhere in this hook.
                    let _ = windows.close(key);
                }
                60 => {
                    let Some(key) = opened.borrow_mut().take() else {
                        return;
                    };
                    let open = windows.open_windows().get();
                    device.check(
                        "closing_a_window_closes_its_dialog",
                        !open.contains(&key) && open.len() <= 1,
                        format!(
                            "{key:?} is {} and {} window(s) remain: {open:?}",
                            state(open.contains(&key)),
                            open.len()
                        ),
                    );
                }
                _ => {}
            }
        }
    }

    /// Report what the window actually became, against what was asked for.
    ///
    /// An observation, and `examples/ios.rs` explains at length why it can never
    /// be a check: a size handed to `App::size` is a request, and a window
    /// manager with a panel and a title bar will hand back something shorter. It
    /// is in the report because that number is the first thing to look at when a
    /// screenshot from one machine is laid out differently from another's, and
    /// the second thing to look at is the density beside it.
    pub(crate) fn surface(&self, requested: Size) -> impl FnMut(&FrameDriver) + 'static {
        let device = self.device.clone();
        let mut done = false;
        move |driver| {
            if done {
                return;
            }
            let metrics = driver.view_metrics();
            let surface = metrics.size;
            if surface.width <= 0.0 || surface.height <= 0.0 {
                return;
            }
            done = true;
            device.observe(
                "surface",
                format!(
                    "asked for {}x{}, got {}x{} logical at {}x",
                    requested.width,
                    requested.height,
                    surface.width,
                    surface.height,
                    metrics.device_pixel_ratio,
                ),
            );
        }
    }
}

/// "open"/"closed", so the detail lines read as sentences rather than as
/// `true`/`false` a reader has to map back onto the check's name.
const fn state(open: bool) -> &'static str {
    if open {
        "open"
    } else {
        "closed"
    }
}
