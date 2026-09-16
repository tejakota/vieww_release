//! Assertions that only a device can answer, run inside the demo app.
//!
//! Shared by the Android and iOS entry points, like the demo tree beside it.
//!
//! # Why this is not a `#[test]`
//!
//! Everything here needs a *window*: a surface with a real size, a display
//! density the platform chose, a cutout the hardware has, and frames that were
//! actually presented to a compositor. `cargo test --target aarch64-linux-android`
//! already runs the portable suite on the device through `ci/mobile/adb-runner.sh`, and
//! that covers every crate that can be tested without one — which is all of them
//! except this bridge. A pushed test binary has no activity, so it has no
//! window, so the interesting half is unreachable from there.
//!
//! So the harness is the application. It runs the checks against the same tree a
//! human is looking at, and reports on a sentinel line that `ci/mobile/device-suite.sh`
//! greps for. Same shape as `adb-runner`'s `__VIEWW_EXIT__`, for the same
//! reason: an exit status that has to survive `adb shell` cannot be trusted, and
//! a sentinel can.
//!
//! # What is checked here and what is checked by the script
//!
//! In here: invariants that must hold on **any** device, so a failure is a bug
//! rather than a fact about the hardware. Zero insets are legal — a phone with
//! no cutout is a real phone — so "the insets are non-zero" is *not* asserted
//! here. The measured values are logged instead, and the script asserts what the
//! operator knows about the device in front of them.
//!
//! That split is the whole design. An assertion that fails on a legitimate
//! device teaches people to ignore the suite.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use vieww_element::Signal;
use vieww_foundation::ViewMetrics;
use vieww_platform_winit::FrameLog;
use vieww_widget::prelude::*;

use crate::screen::{MENU_ITEMS, MENU_LABEL, TAP_LABEL};

/// How many frames to watch before reporting.
///
/// Ten seconds at 60Hz, and the number is load-bearing twice over.
///
/// **It outruns the warm-up.** `FrameLog` keeps the last 240 frames, so
/// reporting at 600 leaves a window containing none of the startup. That
/// matters more than it sounds: on an Adreno 612 the first frame measured
/// **480ms** — vello compiling its shaders on the device — against a median of
/// 5.3ms. Reporting at 120 put that frame inside the window and failed a budget
/// check on an application that was, from frame two onward, comfortably fast.
/// A first frame is expensive on every GPU and asserting otherwise is asserting
/// something false.
///
/// **It outruns the script.** `ci/mobile/device-suite.sh` has to tap the screen before
/// the report goes out, and it cannot tap until the app is up. At 120 frames
/// the report was already gone by the time the tap landed, so
/// `a_tap_reached_a_handler` was simply missing from a passing run — the worst
/// kind of absence, one that looks like a check nobody wrote.
const FRAMES: u64 = 600;

/// A line of the report. `ok: None` is an observation rather than a check.
#[derive(Debug)]
struct Line {
    name: &'static str,
    ok: Option<bool>,
    detail: String,
}

#[derive(Debug, Default)]
struct Results {
    lines: Vec<Line>,
    reported: bool,
}

impl Results {
    /// Record a check, replacing any earlier answer under the same name.
    ///
    /// **The latest answer wins, and the first version of this got it backwards.**
    /// A widget's `build` runs on every frame that touches it, so keeping the
    /// first answer looked like the way to stop the report filling with
    /// duplicates. It is wrong, because the values a probe reads legitimately
    /// start as placeholders: `FrameDriver::new` seeds `ViewMetrics` from the
    /// surface with a density of 1 and no safe area, and the real ones arrive
    /// from `sync_view_metrics` afterwards. First-wins froze that placeholder
    /// and reported a 2.75x phone as `1x` with no cutout — a suite confidently
    /// describing a device that does not exist.
    ///
    /// The report is printed once, at the end, so last-wins costs nothing and
    /// says what was true when it was asked.
    fn check(&mut self, name: &'static str, ok: bool, detail: impl Into<String>) {
        self.put(name, Some(ok), detail.into());
    }

    /// Record a check that, once true, stays true.
    ///
    /// For things that *happen* rather than things that *are*: a tap arrives on
    /// one particular rebuild, and every later build would otherwise overwrite
    /// it with whatever is true then.
    fn latch(&mut self, name: &'static str, detail: impl Into<String>) {
        if self.lines.iter().any(|line| line.name == name) {
            return;
        }
        self.put(name, Some(true), detail.into());
    }

    /// Record a measured value that no assertion is made about.
    fn observe(&mut self, name: &'static str, detail: impl Into<String>) {
        self.put(name, None, detail.into());
    }

    /// Upsert, keeping the original position so the report's order is stable.
    fn put(&mut self, name: &'static str, ok: Option<bool>, detail: String) {
        if let Some(line) = self.lines.iter_mut().find(|line| line.name == name) {
            line.ok = ok;
            line.detail = detail;
            return;
        }
        self.lines.push(Line { name, ok, detail });
    }

    fn failures(&self) -> usize {
        self.lines.iter().filter(|l| l.ok == Some(false)).count()
    }

    fn checks(&self) -> usize {
        self.lines.iter().filter(|l| l.ok.is_some()).count()
    }
}

/// Print one line of the report where the platform can be seen to print.
///
/// **Three platforms, three answers, and only the desktop one is the obvious one.**
/// Android discards stdout, so it needs `log` into logcat. iOS *accepts* stdout and
/// then shows it to nobody but an attached Xcode — see [`emit`] for how that was
/// found — so it needs `NSLog` into the unified log. A desktop has a console and
/// needs neither.
///
/// A `macro_rules!` rather than a function because `log::info!` and `println!` are
/// macros and neither can be passed as a value.
macro_rules! report {
    ($($arg:tt)*) => {{
        #[cfg(target_os = "android")]
        log::info!($($arg)*);
        #[cfg(target_os = "ios")]
        emit(&format!($($arg)*));
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        println!($($arg)*);
    }};
}

/// Put one line where a device log stream will see it, on iOS.
///
/// # Why `println!` is not enough, and how that was established
///
/// A real iPhone 14 ran this app for thirty seconds and the downloaded device log
/// held **61 lines, every one of them `vieww(UIKitCore) <Notice>`** — os_log output
/// from a system library inside our own process. Capture was working perfectly. Not
/// one line of ours appeared, because `println!` writes to **stdout**, which Xcode's
/// debugger captures and the OS unified log never sees.
///
/// So the whole device suite — every check, the tap targets, the frame report — was
/// mute on the one platform where nothing has ever been measured. Android only works
/// because `android_logger` routes `log` into logcat; iOS installed nothing.
///
/// `NSLog` writes to the unified log, which is what `adb`'s counterpart on iOS and
/// every device cloud streams.
///
/// # Why not a `log` backend
///
/// It would be tidier and it is the right shape for an *application* — and that is
/// exactly why it is not being decided here, under a deadline, for a test harness.
/// vieww has no logging story yet (it is on the P1 list); inventing one as a side
/// effect of fixing a device script would be the wrong place to make that choice.
/// This is fifteen lines confined to the example that needs it.
#[cfg(target_os = "ios")]
fn emit(line: &str) {
    use objc2_foundation::NSString;

    extern "C" {
        fn NSLog(format: *const NSString, ...);
    }

    // `"%@"` with the message as an **argument**, never the message as the format
    // string itself. A report carrying a percent sign — and these carry
    // `EdgeInsets`, percentages and file paths — would otherwise be read as a
    // format specifier and pull arbitrary values off the varargs stack.
    let format = NSString::from_str("%@");
    let message = NSString::from_str(line);
    unsafe { NSLog(&*format as *const NSString, &*message as *const NSString) }
}

/// The on-device suite: a probe that lives in the tree, and a per-frame hook.
///
/// Created *before* `App::run`, because `on_frame` is installed before the
/// window exists. It holds no runtime handle of its own — the probe reads what
/// it needs from its `BuildContext` — which is what lets it exist that early.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeviceSuite {
    results: Rc<RefCell<Results>>,
    /// The demo's tap counter, once the tree exists.
    ///
    /// The hooks are installed on the `App` builder, which runs *before* the
    /// build closure that creates the `Demo` and therefore before the signal
    /// exists at all. `probe` is called from inside that closure and is handed
    /// the signal, so it fills this in on the way past — the same shape
    /// `screen::PerfLink` uses for the frame graph, and for the same reason.
    taps: Rc<RefCell<Option<Signal<u32>>>>,
    /// The demo's drawer latch, filled in the same way and for the same reason
    /// as [`taps`](Self::taps).
    ///
    /// It exists so a device run can put a modal up **without a finger**. The
    /// question `BlockSemantics` raises is answerable mechanically — is the
    /// screen behind still in the semantics tree — and only the last step of it
    /// ("does TalkBack *say* nothing") needs ears.
    drawer: Rc<RefCell<Option<Signal<bool>>>>,
}

impl DeviceSuite {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The widget to put in the tree.
    ///
    /// It has to be *in* the tree rather than called from `build`, because the
    /// view metrics are published to the tree **after** the build closure runs —
    /// `resumed` creates the driver, calls `build`, and only then syncs the
    /// metrics. Anything asking about the safe area from inside the build
    /// closure reads the defaults and concludes, wrongly, that the device has no
    /// cutout. Reading it from a widget is also how an application would.
    pub(crate) fn probe(&self, taps: &Signal<u32>) -> WidgetNode {
        // Also the moment the activation hook gets its handle on the counter;
        // see the field.
        *self.taps.borrow_mut() = Some(taps.clone());
        Probe {
            results: Rc::clone(&self.results),
            taps: taps.clone(),
        }
        .into()
    }

    /// Hand the suite the drawer latch, so it can raise a modal itself.
    ///
    /// Separate from [`probe`](Self::probe) rather than a second parameter to
    /// it, because `probe` returns a widget that goes *in* the tree and this
    /// returns nothing — folding them together would make every call site read
    /// as though the drawer were something being mounted.
    ///
    /// No `dead_code` allowance, unlike most of this file: **both** entry points
    /// call this one.
    pub(crate) fn watch_drawer(&self, drawer: &Signal<bool>) {
        *self.drawer.borrow_mut() = Some(drawer.clone());
    }

    /// The closure to hand to `App::before_frame`, which raises a modal and
    /// asks whether the screen behind it is still reachable.
    ///
    /// # What this closes
    ///
    /// Every `ModalBarrier` in the framework wraps itself in `BlockSemantics`
    /// **unconditionally** — decided in code on 2026-08-14, tested on a host,
    /// and never once checked on a device or by ear, because no entry point that
    /// builds for Android had anything modal on it. `screen::NavMenu` is now
    /// that something, and this is the half of the claim a machine can settle.
    ///
    /// It asserts **two things together**, and neither is sufficient alone: the
    /// panel's own items are reachable, *and* [`TAP_LABEL`] is not. The first
    /// alone passes on a drawer that opened over a screen it never silenced; the
    /// second alone passes on a drawer that failed to open at all, which is the
    /// more likely failure and the one that would look like success.
    ///
    /// # What it does not close
    ///
    /// Whether TalkBack *announces* the difference, and whether a finger on the
    /// strip of live scrim beside the panel reaches anything. Both need ears and
    /// a hand; see `ci/mobile/a11y-android.sh --activate` and `docs/HANDOFF.md`. The
    /// value of this check is that it fails on every run where the mechanism
    /// broke, so the listening session is spent on the question that needs a
    /// person rather than on one a machine could have answered.
    ///
    /// # Why it is a three-stage machine and not a boolean
    ///
    /// `before_frame` runs ahead of this frame's build, so `driver.semantics()`
    /// is the tree the **previous** frame produced. Opening the drawer and
    /// reading the tree in one pass would read the screen as it was before the
    /// panel existed and report the blocking as broken on a working build. So:
    /// wait for the screen, open, read on the next frame, close.
    ///
    /// It closes the drawer afterwards on purpose. Leaving a modal up would
    /// change what every later check and every screenshot sees, and a suite that
    /// alters the app it is measuring is the failure this project keeps writing
    /// down.
    ///
    /// # Why this one carries no `dead_code` allowance
    ///
    /// Every other hook here is Android-only because it needs something the
    /// platform provides — a pasteboard, an accessibility bridge — and iOS will
    /// install it the day a device cloud runs one. This hook needs **nothing**:
    /// it reads vieww's own `SemanticsTree` and writes a signal. So both entry
    /// points install it, and `examples/ios.rs` is the desktop preview, which
    /// makes the `BlockSemantics` claim checkable on a workstation rather than
    /// only on a phone.
    pub(crate) fn modal_semantics(&self) -> impl FnMut(&mut vieww_render::FrameDriver) + 'static {
        let results = Rc::clone(&self.results);
        let slot = Rc::clone(&self.drawer);
        let mut stage = 0_u8;
        move |driver| {
            if stage > 1 {
                return;
            }
            let Some(drawer) = slot.borrow().clone() else {
                return;
            };

            let labels: Vec<String> = driver
                .semantics()
                .nodes()
                .iter()
                .filter_map(|node| node.label.clone())
                .collect();
            let reachable = |text: &str| labels.iter().any(|label| label == text);

            if stage == 0 {
                // Not frame one: the tree has to be built and laid out before
                // anything has a published node. Waiting for the button is also
                // what proves the screen was up *before* the drawer went over
                // it, which is what makes its absence below meaningful.
                if reachable(TAP_LABEL) {
                    drawer.set(true);
                    stage = 1;
                }
                return;
            }

            stage = 2;
            let panel = MENU_ITEMS.iter().filter(|item| reachable(item)).count();
            let behind = reachable(TAP_LABEL);
            drawer.set(false);

            results.borrow_mut().check(
                "a_modal_silences_the_screen_behind_it",
                panel == MENU_ITEMS.len() && !behind,
                format!(
                    "{panel} of {} panel item(s) reachable, {} behind the scrim \
                     ({} labelled node(s) in the tree)",
                    MENU_ITEMS.len(),
                    if behind {
                        format!("and {TAP_LABEL} is STILL")
                    } else {
                        String::from("and nothing is")
                    },
                    labels.len(),
                ),
            );
        }
    }

    /// Record a check from outside the tree.
    ///
    /// The probe covers everything a *widget* can see. Some things are only
    /// answerable before the window exists — whether the platform gave the app
    /// a writable directory, for one — and those are asked by the entry point
    /// and reported here, so they land in the same report on the same sentinel
    /// lines rather than in a second mechanism nobody greps for.
    ///
    /// `dead_code` is allowed on both of these because this file is one module
    /// included by two entry points, and only the Android one has a
    /// platform-specific question to ask so far. The iOS one will, the day it
    /// runs anywhere.
    #[allow(dead_code)]
    pub(crate) fn check(&self, name: &'static str, ok: bool, detail: impl Into<String>) {
        self.results.borrow_mut().check(name, ok, detail);
    }

    /// Record a measured value from outside the tree, asserting nothing.
    #[allow(dead_code)]
    pub(crate) fn observe(&self, name: &'static str, detail: impl Into<String>) {
        self.results.borrow_mut().observe(name, detail);
    }

    /// The closure to hand to `App::after_frame`, which reports where the things
    /// a script has to tap **actually are**.
    ///
    /// # Why this exists, and why it is not a convenience
    ///
    /// `ci/mobile/device-suite.sh` used to tap `190,536`. Those numbers were correct for
    /// one 1080x2340 phone at 2.75x and silently wrong everywhere else, and the
    /// script said so itself: "elsewhere the tap misses and the check is absent
    /// rather than failing". **A cross-platform framework cannot verify itself with
    /// a constant that suits one handset** — a second device would report a pass
    /// that had checked less, with nothing to say it had.
    ///
    /// It also froze the demo's layout. `screen.rs` forbids adding anything except
    /// *below* the frame graph, because everything above it feeds those constants,
    /// which is why the phone demo has five controls and not twenty-one.
    ///
    /// So the app answers instead. Nothing here knows a resolution, a density or a
    /// device: [`SemanticsNode::bounds`] is already global logical pixels, and
    /// [`ViewMetrics::device_pixel_ratio`] is the only conversion needed. The same
    /// code reports correctly on a tablet, a low-density phone, or a desktop
    /// window resized by hand.
    ///
    /// # Found by role and label, not by position
    ///
    /// The button by its label, because a screen may grow more buttons; the field
    /// by its role, because a text field's `label` is empty and its `value` is the
    /// text being edited. Both are what a screen reader would use to find them,
    /// which is the point — a target a11y cannot locate is one no script should be
    /// able to either.
    ///
    /// Reported **once**, on the first frame that has both, because the numbers do
    /// not move and a line per frame at 60Hz would bury the report.
    ///
    /// [`SemanticsNode::bounds`]: vieww_render::SemanticsNode::bounds
    pub(crate) fn tap_targets(&self) -> impl FnMut(&vieww_render::FrameDriver) + 'static {
        let mut reported = false;
        move |driver| {
            if reported {
                return;
            }
            let scale = driver.view_metrics().device_pixel_ratio;
            // A ratio of zero or worse would put every target at the origin, which
            // reads as "the layout is broken" rather than "the metrics have not
            // arrived yet". Wait for a real one.
            if !scale.is_finite() || scale <= 0.0 {
                return;
            }

            let semantics = driver.semantics();
            let centre = |node: &vieww_render::SemanticsNode| {
                let rect = node.bounds;
                let x = (rect.origin().dx + rect.size().width / 2.0) * scale;
                let y = (rect.origin().dy + rect.size().height / 2.0) * scale;
                (x.round() as i32, y.round() as i32)
            };

            let button = semantics
                .nodes()
                .iter()
                .find(|n| n.label.as_deref() == Some(TAP_LABEL))
                .map(centre);
            let field = semantics
                .nodes()
                .iter()
                .find(|n| n.role == vieww_render::Role::TextField)
                .map(centre);

            // **Optional, and deliberately not part of the guard below.** The
            // menu button sits at the bottom of a column that is not scrollable,
            // so a short enough screen could lay it out past the edge. Requiring
            // it would then suppress the whole line and take the button and field
            // coordinates with it — turning "the drawer is off screen" into
            // "`ci/mobile/device-suite.sh` taps nothing and reports nothing", which is
            // exactly the failure this function's own comment warns about.
            let menu = semantics
                .nodes()
                .iter()
                .find(|n| n.label.as_deref() == Some(MENU_LABEL))
                .map(centre);

            let (Some((bx, by)), Some((fx, fy))) = (button, field) else {
                return;
            };
            reported = true;
            // One greppable line, in physical pixels, which is the unit
            // `adb shell input tap` speaks.
            let menu = match menu {
                Some((mx, my)) => format!(" menu={mx},{my}"),
                None => String::new(),
            };
            report!("__VIEWW_TAPS__ button={bx},{by} field={fx},{fy}{menu}");
        }
    }

    /// The closure to hand to `App::before_frame`, which activates the button
    /// the way a screen reader does and checks that something happened.
    ///
    /// # What this closes, and what it does not
    ///
    /// The remaining accessibility question on Android was whether a TalkBack
    /// **double-tap activates the right thing** — `handle_semantic_action` end
    /// to end. That path has two halves: `accesskit_android` delivering the
    /// action into the app, and the app routing it to the render object that
    /// can do it. This checks **the second half**, on every device, with nobody
    /// listening: it finds the button the way a screen reader finds it (by
    /// label), dispatches `SemanticAction::Click` at its node, and asserts the
    /// tap counter moved.
    ///
    /// The first half needs TalkBack actually running and is what
    /// `ci/mobile/a11y-android.sh --activate` drives. Neither is sufficient alone —
    /// a working router behind a broken bridge announces nothing, and a working
    /// bridge into a router that drops the action does nothing.
    ///
    /// `before_frame` rather than `after_frame` because dispatching needs
    /// `&mut FrameDriver`, and because the rebuild that follows the signal write
    /// is then *this* frame's rather than the next one's.
    ///
    /// Runs once, on the first frame where the semantics tree has the button in
    /// it — which is not frame one: the tree has to be laid out before it has
    /// bounds or a published node.
    /// `dead_code` for the same reason `check` and `observe` carry it: this file
    /// is one module included by two entry points, and only `android.rs`
    /// installs this hook. iOS will, the day a device cloud runs it.
    #[allow(dead_code)]
    pub(crate) fn semantic_activation(
        &self,
    ) -> impl FnMut(&mut vieww_render::FrameDriver) + 'static {
        let results = Rc::clone(&self.results);
        let slot = Rc::clone(&self.taps);
        let mut done = false;
        move |driver| {
            if done {
                return;
            }
            // Not yet built on the first frames; `probe` fills it in.
            let Some(taps) = slot.borrow().clone() else {
                return;
            };
            let Some(id) = driver
                .semantics()
                .nodes()
                .iter()
                .find(|node| node.label.as_deref() == Some(TAP_LABEL))
                .map(|node| node.id)
            else {
                return;
            };
            done = true;

            let before = taps.get();
            let handled = driver.handle_semantic_action(id, vieww_render::SemanticAction::Activate);
            let after = taps.get();

            // Two assertions in one, deliberately. `handled` alone would pass if
            // some ancestor claimed the action and did nothing with it, and the
            // counter alone would pass if the button had been tapped by a finger
            // in the same frame. Together they say this dispatch did this work.
            results.borrow_mut().check(
                "a_semantic_click_activates_the_button",
                handled && after == before + 1,
                format!("handled={handled}, taps {before} -> {after}"),
            );
        }
    }

    /// The closure to hand to `App::after_frame`, which round-trips the real
    /// platform pasteboard.
    ///
    /// # Why this is a device check
    ///
    /// The JNI and `objc2` clipboard paths were written on 2026-08-13 and have
    /// only ever been through a compiler. The desktop path has five tests
    /// through the real services seam; these two have none, and compiling says
    /// nothing about whether `ClipboardManager` was found, whether the JNI
    /// signature matches, or whether the returned `CharSequence` survives being
    /// turned into a `String`.
    ///
    /// **Not at startup, and that is the whole reason this is a frame hook.**
    /// From API 29 Android returns an empty pasteboard to a *background* read,
    /// so a round trip run from `android_main` before the activity is in front
    /// would report a broken clipboard on a working one. `after_frame` runs
    /// after a frame has been presented, so the app is by definition on screen.
    ///
    /// **It puts back what it found.** A device suite that silently ate the
    /// user's pasteboard would be a bad citizen on a phone somebody also uses.
    /// The restore is best-effort: if the read failed there is nothing to put
    /// back, which is reported rather than hidden.
    ///
    /// What it still does not prove is that *another application* sees the
    /// write. That needs a second app and a person, and it is the one clipboard
    /// question left after this.
    /// `dead_code` as above — installed by `android.rs` only, for now. The iOS
    /// pasteboard is written and equally unrun, and this is the hook that will
    /// check it.
    #[allow(dead_code)]
    pub(crate) fn clipboard_round_trip(
        &self,
        clipboard: Option<Rc<dyn vieww_foundation::Clipboard>>,
    ) -> impl FnMut(&vieww_render::FrameDriver) + 'static {
        let results = Rc::clone(&self.results);
        let mut done = false;
        move |_driver| {
            if done {
                return;
            }
            done = true;

            let Some(clipboard) = clipboard.clone() else {
                results.borrow_mut().check(
                    "the_platform_clipboard_round_trips",
                    false,
                    "no Clipboard service is registered on this platform",
                );
                return;
            };

            // Distinctive, so that a stale value from a previous run cannot be
            // mistaken for this one's write succeeding.
            const MARKER: &str = "vieww device clipboard check 8f3a";

            let previous = clipboard.read_text();
            let written = clipboard.write_text(MARKER);
            let read_back = clipboard.read_text();

            // Best effort, and only when there was something to restore.
            if let Ok(Some(text)) = &previous {
                let _ = clipboard.write_text(text);
            }

            let detail = match (&written, &read_back) {
                (Err(error), _) => format!("write failed: {error}"),
                (Ok(()), Err(error)) => format!("wrote, but read failed: {error}"),
                (Ok(()), Ok(Some(text))) if text == MARKER => {
                    let restored: String = match &previous {
                        Ok(Some(_)) => "restored what was there".to_owned(),
                        Ok(None) => "the pasteboard had been empty".to_owned(),
                        Err(error) => format!("could not read the old value back ({error})"),
                    };
                    format!("wrote and read back {} bytes; {restored}", MARKER.len())
                }
                (Ok(()), Ok(Some(text))) => {
                    format!(
                        "read back {} bytes, and they are not the marker",
                        text.len()
                    )
                }
                (Ok(()), Ok(None)) => "wrote, and read back an empty pasteboard — the API 29 \
                     background-read rule looks like this"
                    .to_owned(),
            };

            let ok = matches!((&written, &read_back), (Ok(()), Ok(Some(text))) if text == MARKER);
            results
                .borrow_mut()
                .check("the_platform_clipboard_round_trips", ok, detail);
        }
    }

    /// The closure to hand to `App::on_frame`.
    pub(crate) fn hook(&self) -> impl FnMut(&FrameLog) + 'static {
        let results = Rc::clone(&self.results);
        move |log| {
            if log.frames() < FRAMES || results.borrow().reported {
                return;
            }
            let mut results = results.borrow_mut();
            frame_checks(&mut results, log);
            results.reported = true;
            print(&results);
        }
    }
}

/// What the frame log says after [`FRAMES`] frames.
fn frame_checks(results: &mut Results, log: &FrameLog) {
    let samples = log.work_samples();
    let budget = log.budget();

    results.check(
        "frames_were_presented",
        log.frames() >= FRAMES,
        format!("{} frames", log.frames()),
    );

    if samples.is_empty() {
        results.check("frame_times_were_recorded", false, "no samples");
        return;
    }

    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let worst = *sorted.last().unwrap_or(&Duration::ZERO);
    let median = sorted[sorted.len() / 2];

    // The check the raster fix made meaningful. Before it, `work` contained the
    // vsync wait and this was pinned at the budget on a perfectly healthy app —
    // an assertion that could only ever have been written to pass.
    //
    // Measured over a window that excludes the warm-up, by construction: see
    // `FRAMES`. The occasional late frame is still allowed, because a phone
    // schedules other things and a suite that fails on one 20ms frame in six
    // hundred is a suite that gets muted. What it will not tolerate is a
    // *median* over budget, which is what "this app is slow" actually looks
    // like, or a single frame so late it drops several vsyncs.
    let over = sorted.iter().filter(|work| **work > budget).count();
    let stall = budget * 4;
    results.check(
        "the_typical_frame_is_inside_its_budget",
        median <= budget,
        format!("median {median:?} against a budget of {budget:?}"),
    );
    results.check(
        "no_frame_stalled",
        worst <= stall,
        format!("worst {worst:?}, {over} of {} over budget", sorted.len()),
    );

    // Deliberately not a check: how much headroom a device has is a fact about
    // the device, and a threshold here would be a guess about hardware nobody
    // has seen yet.
    results.observe(
        "frame_time",
        format!(
            "median {median:?}, worst {worst:?}, budget {budget:?} ({} samples)",
            sorted.len()
        ),
    );

    // The one performance claim that *can* be gated on a single run, and the
    // reason is that it is not a time.
    //
    // Every millisecond above moves with the handset and with how hot it is —
    // four runs of this suite on one phone produced medians of 4.09, 6.86, 6.56
    // and 10.84ms with no code change between them. A 2.65x spread is larger
    // than most regressions worth catching, so a threshold either passes
    // everything (which is what a 16.7ms ceiling does) or flakes.
    //
    // Rasterised pixels do not throttle. `full_repaints` is pixels vello
    // actually touched over pixels in the surface, so per frame it is a pure
    // property of damage tracking — independent of the device, the resolution
    // and the temperature.
    //
    // **The bound was 1.0 for weeks and it was measuring nothing.** That number
    // was chosen because more than one full repaint per frame is the one outcome
    // that makes damage tracking worse than not having it — true, and far too
    // loose, because the note beside it claimed this screen already sat at 1.0
    // (`rasterised 190.00 full repaints` over 190 frames) and so every frame was
    // repainting in full.
    //
    // **That claim does not reproduce.** Measured 2026-08-14 on the Redmi Note 7
    // Pro, over 600 frames: **0.066 full repaints per frame**, with only 2
    // individual frames costing a whole surface. Damage tracking was working the
    // whole time. The old figure has no run behind it that anyone can point at,
    // and it is the fourth item on a "what is left" list to dissolve on contact.
    //
    // So the bound is 0.25 — **fifteen times below the old one and four times
    // above what was measured.** The headroom is for the parts of this suite
    // that legitimately damage everything: the keyboard opening, a tap changing
    // a whole row, the scroll. What it will not tolerate is the failure the old
    // note described: a screen that repaints in full every frame lands at 1.0
    // and fails this by a factor of four.
    //
    // Safe to state tightly because it is **not a time**. Rasterised pixels do
    // not throttle, the value is a ratio so resolution divides out, and the
    // suite drives the same scripted sequence everywhere — unlike the
    // milliseconds above, which spread 2.65x across four runs on one handset.
    let report = log.report();
    let per_frame = if log.frames() == 0 {
        0.0
    } else {
        report.full_repaints() / log.frames() as f64
    };
    results.check(
        "damage_tracking_costs_a_fraction_of_a_full_repaint",
        report.surface_pixels > 0 && per_frame <= 0.25,
        format!(
            "{per_frame:.3} full repaints per frame over {} frames, \
             against a {}px surface",
            log.frames(),
            report.surface_pixels,
        ),
    );

    // Which frames those repaints were, which the ratio above cannot say.
    //
    // A ratio of 1.0 over 190 frames is equally consistent with every frame
    // repainting in full and with a third of them repainting three times over
    // while the rest cost nothing. Those are different bugs and one of them is
    // not a bug at all, so the ratio alone was never something to act on.
    //
    // `continuous` already draws the line that matters. A frame the loop *owed*
    // — mid-scroll, mid-animation — repaints everything because the viewport
    // moved everything, and that is correct. A frame the loop **slept before**
    // was one tap or one caret blink on a still screen, and rasterising the
    // whole surface for it is precisely what damage tracking exists to avoid.
    results.observe(
        "full_repaints",
        format!(
            "{} of {} frames cost a whole surface, {} of those after the loop slept",
            report.full_repaint_frames, report.frames, report.idle_full_repaint_frames,
        ),
    );

    // Gated now, because the number has been read: **1 idle full repaint in 600
    // frames** on 2026-08-14. The question this was built to answer — "startup
    // and idle frames repaint fully and nobody has worked out why" — turns out
    // to have had a false premise. They do not.
    //
    // 2% rather than the 0.17% measured, because one legitimate cause exists and
    // is rare: the surface being invalidated after a sleep, which forces a full
    // repaint by design. A handful of those in a session is correct behaviour.
    // Hundreds is the defect, and on a 600-frame run the two are three orders of
    // magnitude apart — there is no threshold between them worth arguing about.
    //
    // The floor keeps a short run honest: 2% of 30 frames is a bound of zero,
    // which the *first* frame would fail on its own if it happened to follow a
    // sleep.
    let idle_budget = 4.max(report.frames / 50);
    results.check(
        "idle_frames_do_not_repaint_the_whole_surface",
        report.idle_full_repaint_frames <= idle_budget,
        format!(
            "{} idle full repaint(s) over {} frames, against a budget of {idle_budget}",
            report.idle_full_repaint_frames, report.frames,
        ),
    );

    // A check on the *instrument*, not on the result. The first presented frame
    // has no previous contents to keep, so it is a full repaint by construction
    // — on every device, at every resolution. Zero here therefore does not mean
    // a clean run, it means the counter never saw a surface size and is
    // reporting silence as good news. That failure mode has cost this project a
    // day before; see `uiautomator dump` in docs/HANDOFF.md.
    //
    // It matters more now that the two bounds above are tight: both are
    // satisfied trivially by a counter that never counts.
    results.check(
        "the_full_repaint_counter_saw_the_first_frame",
        report.frames == 0 || report.full_repaint_frames >= 1,
        format!(
            "{} full repaint(s) over {} frames against a {}px surface",
            report.full_repaint_frames, report.frames, report.surface_pixels,
        ),
    );

    results.observe("frame_report", format!("{report}"));
}

/// Write the report out, ending with the sentinel the script greps for.
fn print(results: &Results) {
    report!("__VIEWW_SUITE__ begin");
    for line in &results.lines {
        let mark = match line.ok {
            Some(true) => "ok  ",
            Some(false) => "FAIL",
            None => "--  ",
        };
        report!("__VIEWW_SUITE__ {} {}: {}", mark, line.name, line.detail);
    }
    let failures = results.failures();
    report!(
        "__VIEWW_SUITE__ end {} checks, {} failed",
        results.checks(),
        failures
    );
    // A separate, greppable verdict. The script keys on this rather than
    // counting lines itself, so adding a check never changes the script.
    report!(
        "__VIEWW_SUITE__ result {}",
        if failures == 0 { "PASS" } else { "FAIL" }
    );
}

/// Reads what only a live window knows, from inside the tree.
///
/// Draws nothing. It is a `Composed` widget returning an empty box, so it costs
/// one element and no pixels — a probe that changed the layout would be
/// measuring a tree that only exists while it is being measured.
#[derive(Debug)]
struct Probe {
    results: Rc<RefCell<Results>>,
    taps: Signal<u32>,
}

impl Widget for Probe {
    fn debug_name(&self) -> &'static str {
        "Probe"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let metrics = ctx.inherit_or(ViewMetrics::default());
        let taps = self.taps.get();
        let mut results = self.results.borrow_mut();

        // The metrics reached the tree at all. `ViewMetrics::default` is a zero
        // surface, so a real size proves the publish happened — which is exactly
        // what the `set_root` footgun silently broke, and it broke it in a way
        // that looked like a layout bug rather than a plumbing one.
        let size = metrics.size;
        results.check(
            "view_metrics_reached_the_tree",
            size.width > 0.0 && size.height > 0.0,
            format!("{}x{} logical", size.width, size.height),
        );

        let ratio = metrics.device_pixel_ratio;
        results.check(
            "the_display_density_is_plausible",
            ratio.is_finite() && (0.5..=8.0).contains(&ratio),
            format!("{ratio}x"),
        );

        // Sane, not non-zero. A phone with no cutout reports zeros and is not
        // broken; what would be broken is an inset that is negative, or one that
        // eats the whole screen. See this module's header on why the "is there
        // actually a notch" question belongs to the script.
        let safe = metrics.safe_area;
        let sane = [safe.left, safe.top, safe.right, safe.bottom]
            .iter()
            .all(|inset| inset.is_finite() && *inset >= 0.0)
            && safe.top + safe.bottom < size.height
            && safe.left + safe.right < size.width;
        results.check(
            "the_safe_area_fits_inside_the_surface",
            sane,
            format!(
                "l{} t{} r{} b{}",
                safe.left, safe.top, safe.right, safe.bottom
            ),
        );

        // The numbers themselves, for the script and for a human. This is the
        // line that answers "did the cutout reach us", and it is an observation
        // because only the operator knows what the device has.
        results.observe(
            "safe_area",
            format!(
                "l{} t{} r{} b{} on a {}x{} surface at {}x",
                safe.left, safe.top, safe.right, safe.bottom, size.width, size.height, ratio
            ),
        );
        // A keyboard is a *bottom* inset and nothing else, and it can never be
        // taller than the surface it is covering. Both hold on any device with
        // or without one up, so this is a check rather than an observation —
        // and both are exactly what goes wrong if the platform hands back a
        // rectangle in the wrong coordinate space.
        let keyboard = metrics.view_insets;
        let plausible = keyboard.bottom.is_finite()
            && keyboard.bottom >= 0.0
            && keyboard.bottom < size.height
            && keyboard.left == 0.0
            && keyboard.top == 0.0
            && keyboard.right == 0.0;
        results.check(
            "the_keyboard_inset_is_a_plausible_bottom_inset",
            plausible,
            format!("{keyboard:?} on a {}-tall surface", size.height),
        );

        // Latched at the first non-zero reading, so it survives the keyboard
        // being dismissed again before the report goes out. Its *absence* from
        // a run is the interesting outcome, and it has already been interesting
        // once: it was missing because `sync_ime` was never called on a tap, so
        // no keyboard was ever raised to have an inset. Seen at **290.55
        // logical pixels** on a Redmi Note 7 Pro once that was fixed.
        if keyboard.bottom > 0.0 {
            results.latch(
                "the_keyboard_inset_appears_when_the_keyboard_does",
                format!("{} logical pixels", keyboard.bottom),
            );
        }

        results.observe(
            "keyboard_inset",
            format!("{keyboard:?} at the moment of the report"),
        );

        // Recorded on the *rebuild* a tap causes, which is the point: reaching
        // this line at all means a MotionEvent crossed winit, the translator,
        // the router, the arena, a recogniser and a handler, and then marked
        // this element pending. `ci/mobile/device-suite.sh` sends the tap.
        if taps > 0 {
            // Latched at the count it first saw, which is normally 1 even though
            // the script sends several — it cannot know the app felt one until
            // the report, so it keeps tapping. The number is not the point; that
            // this line exists at all is.
            let plural = if taps == 1 { "tap" } else { "taps" };
            results.latch("a_tap_reached_a_handler", format!("{taps} {plural}"));
        }

        SizedBox::shrink().into()
    }
}

widget_node_from!(Probe);
