//! A real `ControlFlow::Wait` loop, in a real window, asserted on.
//!
//! ```console
//! cargo test -p vieww-platform-winit --test wait_loop -- --nocapture
//! ```
//!
//! # Why this file exists
//!
//! `docs/PRODUCTION-GAPS.md` §6 argues that this is the highest-leverage
//! testing work in the repository, and the evidence it gives is hard to argue
//! with: **one real window found three bugs that 1,066 tests could not reach.**
//! Every one of them lived in the seam between winit's event delivery and this
//! crate's frame scheduling — a seam that has no representation in a harness,
//! because a harness *is* the thing being replaced.
//!
//! Three claims in this crate are, as of this file, comments rather than tests:
//!
//! * `windows.rs`: "An idle vieww application sleeps in `ControlFlow::Wait` and
//!   costs no CPU."
//! * `app.rs`, at the `set_control_flow` decision: that `Poll` is entered only
//!   while something is drawing, and the loop returns to `Wait` after.
//! * `stats.rs`, on `worst_interval`: that idle time is excluded, because
//!   counting it made an untouched 216-second session report a worst frame of
//!   1979.50ms.
//!
//! All three are about what the loop does when *nothing happens*, which is
//! precisely what no unit test can stage. `window_to_gesture.rs`, the closest
//! existing test, says so in its own header: it drives the whole bridge "with no
//! window, because everything that needed one has already been separated out".
//! What is left over is what is here.
//!
//! # It needs a display, and says so rather than lying
//!
//! There is no display in most CI containers and none in the sandbox this was
//! written in. Every scenario below gates on **both**
//! [`vieww_hardware::Capability::Display`] and
//! [`vieww_hardware::Capability::Gpu`], and returns early with a printed reason.
//!
//! Both, because a display alone is not enough and that is not obvious: under
//! Xvfb the window opens and the event loop runs, and then `App::run` returns
//! `Gpu(NoAdapter)` — there is a screen but nothing that can draw to it. The
//! adapter probe asks the same question the renderer's own Vulkan device
//! creation does — a working `VK_KHR_surface`/`VK_KHR_swapchain`-capable
//! physical device — so the gate and the renderer cannot disagree about what
//! counts as a GPU.
//!
//! **A test that always skips is indistinguishable from a test that passes**,
//! which is the failure mode that made these three claims comments for as long
//! as they were. The counter-measure is not in this file: set
//! `VIEWW_REQUIRE_HARDWARE=display` and a missing display becomes a panic
//! instead of a skip. A machine that is *supposed* to have a display — a
//! developer's laptop, a CI job with Xvfb — should set it, and then this file
//! cannot quietly stop testing anything.
//!
//! ```console
//! VIEWW_REQUIRE_HARDWARE=display,gpu cargo test -p vieww-platform-winit --test wait_loop
//! ```
//!
//! # Why this is not a `#[test]`
//!
//! It cannot be. Rust's test harness runs every `#[test]` on a spawned thread,
//! and winit refuses to build an event loop off the main thread on Linux:
//!
//! ```text
//! Initializing the event loop outside of the main thread is a significant
//! cross-platform compatibility hazard.
//! ```
//!
//! That refusal is correct — on macOS it is not a hazard but a hard platform
//! rule — and it is the structural reason this file did not exist before. A
//! `#[test]` **cannot** open a window, so any test that wanted one had to
//! settle for a harness, which is exactly how the three claims above came to be
//! comments. `harness = false` in `Cargo.toml` is what buys the main thread
//! back; the escape hatches winit offers instead (`EventLoopBuilderExtX11::
//! any_thread`) were deliberately not used, because a test that runs the loop
//! somewhere the shipped code never runs it is testing a different program.
//!
//! # And why each scenario is its own process
//!
//! One `EventLoop` per process is all several platforms allow, and `App::run`
//! consumes the app and returns only when the last window closes. Running the
//! three scenarios in sequence in one process would therefore test the second
//! and third under a condition — a loop already built and torn down — that no
//! application is ever in.
//!
//! So the binary re-executes *itself*, once per scenario, with the scenario's
//! name as its argument. Each child gets a clean process, a real main thread and
//! one event loop; the parent runs them in order and reports. The cost is three
//! process spawns and the benefit is that each scenario sees the same world a
//! real application sees on startup.
//!
//! # Every scenario judges itself from inside `on_frame`, and exits
//!
//! Not the shape anyone would choose first, and the reason is a finding rather
//! than a preference. Three attempts failed in three different ways, each of
//! which is a fact about the framework worth writing down:
//!
//! 1. **Closing from `after_frame` cannot work for an idle tree.** That hook
//!    runs after a *presented* frame, and an unchanging tree damages nothing, so
//!    the woken frame is built and never presented. Correct damage tracking, and
//!    it makes the hook unreachable for exactly the tree these scenarios need.
//! 2. **`on_frame` does run** — the wake produces a frame and the hook sees it.
//! 3. **`Windows::close(WindowKey::PRIMARY)` from inside a frame does not end an
//!    idle loop.** `service_requests` is called from `about_to_wait`, so the
//!    close is queued and serviced on the *next* turn — and an idle loop has no
//!    next turn. A second `Waker::wake` afterwards did not produce one either.
//!    This is left as a finding rather than worked around silently; see
//!    `NEXT.md`.
//!
//! What is left is to assert inside `on_frame`, where
//! [`FrameLog::report`](vieww_platform_winit::FrameLog::report) gives the same
//! statistics `App::run` would have returned, and to exit the process on
//! success. A failing assertion panics and the child's non-zero status is what
//! the parent reports. `App::run` therefore never returns in a passing scenario,
//! which is exactly what item 3 above says would happen anyway.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use vieww_foundation::task::FrameWaker;
use vieww_foundation::Color;
use vieww_platform_winit::App;
use vieww_render::{WindowKey, Windows};
use vieww_widget::prelude::*;

/// How long an idle test sits still before deciding the loop is asleep.
///
/// Long enough that a `Poll` loop at any plausible refresh rate would have
/// produced dozens of frames — at 60Hz this is about 36 — and short enough that
/// a person runs the suite anyway. The assertions below are all of the form
/// "many fewer than a spinning loop would have made", never an exact count,
/// because the exact count is a property of the compositor.
const IDLE: Duration = Duration::from_millis(600);

/// The ceiling for a whole test, after which it fails rather than hangs.
///
/// A wait-loop bug's most likely symptom is *not waking up*, and a test whose
/// failure mode is an un-killed process wedges CI instead of reporting. Every
/// test below arms this before it opens a window.
const DEADLINE: Duration = Duration::from_secs(20);

/// A tree that never changes and never animates.
///
/// The point of every test here is what the loop does when the application is
/// giving it nothing to do, so the application has to genuinely give it nothing
/// — no signal, no animation, no timer. A `ColoredBox` is the smallest thing
/// that still produces a real scene with real pixels.
#[derive(Debug)]
struct Still;

impl Widget for Still {
    fn debug_name(&self) -> &'static str {
        "Still"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        ColoredBox::new(Color::BLUE).into()
    }
}

vieww_widget::widget_node_from!(Still);

/// Fail the whole process if a test outlives [`DEADLINE`].
///
/// A watchdog thread rather than a timeout inside the loop, because the bug it
/// guards against is the loop not running: anything scheduled *by* the loop is
/// unreachable in exactly the case worth catching. `std::process::abort` rather
/// than `panic!` for the same reason — a panic on a watchdog thread does not
/// stop an event loop blocked in the compositor.
/// Print progress when `VIEWW_WAIT_LOOP_TRACE` is set.
///
/// A scenario that hangs here hangs *inside a real event loop on a real
/// compositor*, where a debugger is awkward and a stack trace says only that
/// the process is blocked in the platform. The trace is the cheapest tool that
/// works, and it is left in because the next person to touch this file will
/// need it for the same reason this one did.
fn trace(what: &str) {
    if std::env::var_os("VIEWW_WAIT_LOOP_TRACE").is_some() {
        eprintln!("wait_loop trace: {what}");
    }
}

fn arm_deadline(what: &'static str) {
    std::thread::spawn(move || {
        std::thread::sleep(DEADLINE);
        eprintln!("wait_loop: {what} did not finish within {DEADLINE:?}; aborting");
        std::process::abort();
    });
}

/// Nanoseconds this process has spent on a CPU, if the platform will say.
///
/// **The frame count is not the measurement, and finding that out cost a
/// mutation test.** The claim in `windows.rs` is that an idle application
/// "costs no CPU" — and a loop wrongly left in `ControlFlow::Poll` with nothing
/// to draw still produces no frames. It spins in `about_to_wait`, decides there
/// is nothing to do, and goes round again, for ever, at 100% of a core. Counting
/// frames sees that as a perfectly quiet application. Forcing the loop into
/// `Poll` and watching the frame-count assertion pass is what proved the
/// assertion was decorative.
///
/// `/proc/self/schedstat`'s first field is time on the CPU in **nanoseconds**,
/// which needs no clock-tick conversion and no `libc` dependency for a crate
/// that is a test. Linux only, and the fallback is `None` rather than a guess —
/// on a platform that will not answer, the scenario says so and asserts what it
/// still can, which is honest in a way that a fabricated zero would not be.
fn cpu_nanos() -> Option<u64> {
    let raw = std::fs::read_to_string("/proc/self/schedstat").ok()?;
    raw.split_whitespace().next()?.parse().ok()
}

/// Assert, and end the process if the scenario is satisfied.
///
/// `std::process::exit` rather than returning, because there is no way back out
/// of `App::run` for an idle loop — see the module docs. A scenario that
/// reaches here has already made its assertions, so exiting zero is the honest
/// report; a scenario that fails panics on the way and never arrives.
fn pass(what: &str) -> ! {
    trace(&format!("pass: {what}"));
    std::process::exit(0)
}

/// An idle application stops producing frames.
///
/// The `windows.rs` claim, made checkable: after the first frames have settled
/// the loop reaches `ControlFlow::Wait` and nothing further is drawn, however
/// long it is left alone.
///
/// Asserted as a *bound* rather than an equality. The startup sequence is
/// allowed several frames — the surface is configured, the window is mapped, a
/// compositor may deliver a resize or a scale change — and pinning an exact
/// count would make this a record of one compositor's behaviour. What
/// distinguishes sleeping from spinning is orders of magnitude, not one frame: a
/// `Poll` loop over [`IDLE`] at 60Hz draws about 36 frames and at 120Hz about
/// 72, so a ceiling of 10 separates the two cases with room to spare and no
/// tuning.
fn an_idle_application_stops_drawing_rather_than_spinning() {
    vieww_hardware::skip_without!(display, gpu);
    arm_deadline("an_idle_application_stops_drawing_rather_than_spinning");

    let app = App::new().title("vieww wait-loop: idle");

    // The wake is what ends the idle period, and it has to arrive from off the
    // UI thread because that is the only place anything can arrive from while
    // the loop is asleep. It is also, conveniently, the mechanism the next
    // scenario is about.
    let waker = app.waker();
    let elapsed = Arc::new(AtomicU32::new(0));
    std::thread::spawn({
        let elapsed = Arc::clone(&elapsed);
        move || {
            std::thread::sleep(IDLE);
            elapsed.store(1, Ordering::SeqCst);
            trace("waking");
            waker.wake();
        }
    });

    let frames = Rc::new(Cell::new(0u32));
    let at_idle_start = Rc::new(Cell::new(0u32));
    let cpu_at_idle_start = Rc::new(Cell::new(None::<u64>));

    let result = app
        .on_frame({
            let frames = Rc::clone(&frames);
            let at_idle_start = Rc::clone(&at_idle_start);
            let cpu_at_idle_start = Rc::clone(&cpu_at_idle_start);
            let elapsed = Arc::clone(&elapsed);
            move |_log| {
                frames.set(frames.get() + 1);
                trace(&format!("frame {}", frames.get()));

                if elapsed.load(Ordering::SeqCst) == 0 {
                    // Still inside the idle window. Each frame here is one the
                    // loop should not have drawn, and the CPU reading taken with
                    // it is the baseline — both are overwritten every time, so
                    // what survives is the state the loop settled into.
                    at_idle_start.set(frames.get());
                    cpu_at_idle_start.set(cpu_nanos());
                    return;
                }

                // The wake landed. One frame is subtracted because the wake
                // itself drew that one, and it is this scenario's doing rather
                // than the loop spinning.
                let drawn = frames
                    .get()
                    .saturating_sub(at_idle_start.get())
                    .saturating_sub(1);
                assert!(
                    drawn <= 10,
                    "{drawn} frames across {IDLE:?} of an unchanging tree — the \
                     loop is drawing while nothing has changed."
                );

                // **This is the assertion that bites.** A `Poll` loop with
                // nothing to draw is invisible to the frame count and obvious
                // here: it spends the whole idle period on a CPU. A sleeping
                // loop spends approximately none of it, so a tenth of the
                // period is a threshold with two orders of magnitude of room
                // rather than a tuned constant.
                match (cpu_at_idle_start.get(), cpu_nanos()) {
                    (Some(before), Some(now)) => {
                        let spent = Duration::from_nanos(now.saturating_sub(before));
                        let ceiling = IDLE / 10;
                        trace(&format!("cpu during idle: {spent:?} (ceiling {ceiling:?})"));
                        assert!(
                            spent < ceiling,
                            "{spent:?} of CPU across {IDLE:?} of an idle \
                             application — the loop is spinning rather than \
                             sleeping. `windows.rs` claims an idle window costs \
                             no CPU; this is what that claim means."
                        );
                    }
                    _ => trace("no /proc/self/schedstat; the CPU claim is unchecked here"),
                }
                pass("idle");
            }
        })
        .run(|driver| driver.set_root(Still));

    // Only reachable if the loop ended without the wake ever landing, which
    // means the application exited on its own — there is nothing to assert
    // about sleeping, so say that rather than passing quietly.
    panic!("the loop ended before the idle period was over: {result:?}");
}

/// Waking a sleeping loop produces a frame.
///
/// The idle scenario cannot distinguish "correctly asleep" from "wedged and
/// unable to wake", and a framework that fails the second way fails silently and
/// completely: the screen simply stops responding. This is the test that tells
/// them apart.
///
/// [`App::waker`] is the mechanism precisely because it arrives from **off** the
/// UI thread. Its whole reason for existing, per its own docs, is that work
/// finishing on a worker thread must cause a frame rather than sitting in its
/// channel "until the user touches it" — so a wake that crosses the thread
/// boundary is the case worth proving, not a redraw asked for from inside a hook
/// the loop was already running.
fn a_waker_brings_a_sleeping_loop_back_and_it_settles_again() {
    vieww_hardware::skip_without!(display, gpu);
    arm_deadline("a_waker_brings_a_sleeping_loop_back_and_it_settles_again");

    let app = App::new().title("vieww wait-loop: wake");
    let waker = app.waker();
    let woke = Arc::new(AtomicU32::new(0));

    // Sleeping first is the point: a wake delivered while the loop is still
    // starting up proves nothing, because the loop was going to draw anyway.
    std::thread::spawn({
        let woke = Arc::clone(&woke);
        move || {
            std::thread::sleep(IDLE);
            woke.store(1, Ordering::SeqCst);
            trace("waking");
            waker.wake();
        }
    });

    let frames = Rc::new(Cell::new(0u32));
    let before_wake = Rc::new(Cell::new(0u32));

    let result = app
        .on_frame({
            let frames = Rc::clone(&frames);
            let before_wake = Rc::clone(&before_wake);
            let woke = Arc::clone(&woke);
            move |log| {
                frames.set(frames.get() + 1);
                trace(&format!("frame {}", frames.get()));

                if woke.load(Ordering::SeqCst) == 0 {
                    before_wake.set(frames.get());
                    return;
                }

                assert!(
                    frames.get() > before_wake.get(),
                    "the loop drew {} frames before the wake and {} after — a \
                     `Waker` from another thread did not produce a frame, which \
                     is the failure mode where an application stops responding \
                     to finished background work",
                    before_wake.get(),
                    frames.get()
                );

                // A wake must cost a frame, not a frame *rate*. If waking
                // flipped the loop into `Poll` and left it there, the total is
                // where it shows up — the whole session is one startup plus one
                // wake.
                let report = log.report();
                assert!(
                    report.frames <= 12,
                    "{} frames for one wake — the loop did not return to `Wait`",
                    report.frames
                );
                pass("wake");
            }
        })
        .run(|driver| driver.set_root(Still));

    panic!("the loop ended before the wake landed: {result:?}");
}

/// Idle time is not reported as a slow frame.
///
/// The `stats.rs` claim. An untouched 216-second session once reported a worst
/// frame of 1979.50ms, and the run that verified the layout-write fix reported
/// 48302.83ms — numbers that say the framework is broken when what happened is
/// that it was doing nothing, correctly and at no cost. `Frame::continuous` is
/// the distinction and the loop is what draws it, so the loop is where it has to
/// be checked.
///
/// This scenario is the reason the file is worth its weight: the accounting it
/// checks is *only* wrong in the presence of real idle time, and real idle time
/// is exactly what a harness fabricates rather than experiences.
fn idle_time_is_not_counted_as_the_worst_frame() {
    vieww_hardware::skip_without!(display, gpu);
    arm_deadline("idle_time_is_not_counted_as_the_worst_frame");

    let app = App::new().title("vieww wait-loop: accounting");
    let waker = app.waker();
    let elapsed = Arc::new(AtomicU32::new(0));
    std::thread::spawn({
        let elapsed = Arc::clone(&elapsed);
        move || {
            std::thread::sleep(IDLE);
            elapsed.store(1, Ordering::SeqCst);
            trace("waking");
            waker.wake();
        }
    });

    let result = app
        .on_frame({
            let elapsed = Arc::clone(&elapsed);
            move |log| {
                trace("frame");
                if elapsed.load(Ordering::SeqCst) == 0 {
                    return;
                }

                // The frame this hook is in has not been recorded yet, so the
                // report describes the session up to the sleep — which is the
                // session this scenario is about.
                let report = log.report();
                trace(&format!("{report:?}"));

                // If the idle period leaked into the interval statistics it
                // lands here, and it lands *enormous*: hundreds of milliseconds
                // against a budget of about sixteen.
                assert!(
                    report.worst_interval < IDLE / 2,
                    "worst interval {:?} over a session that was idle for \
                     {IDLE:?} — idle time is being reported as a slow frame, \
                     which is the 1979.50ms bug",
                    report.worst_interval
                );

                // And the honest-answer clause of the same doc: a session with
                // no continuous intervals reports zero rather than inventing a
                // median.
                if report.continuous_intervals == 0 {
                    assert_eq!(
                        report.worst_interval,
                        Duration::ZERO,
                        "no continuous intervals, so there is nothing to report \
                         — but a non-zero worst interval was reported anyway"
                    );
                }
                pass("accounting");
            }
        })
        .run(|driver| driver.set_root(Still));

    panic!("the loop ended before the idle period was over: {result:?}");
}

/// An application can close its own last window, and gets its report back.
///
/// The regression test for the two defects this file found on the day it was
/// written, both of which needed a real window and neither of which any of the
/// 2,057 other tests could reach.
///
/// **The close.** `Windows::close` from a background task queues the close and
/// wakes the loop; `service_requests` destroyed the window and never asked
/// whether it was the last one, because that rule lived inline in the
/// `WindowEvent::CloseRequested` arm — the path where the *user* clicks the
/// button. So an application that closed itself — a sync finishing, a sign-out
/// — destroyed its window and kept running, invisible, for ever. Without this
/// scenario the assertion is a hang, which is why it arms a watchdog.
///
/// **The report.** Fixing the first exposed the second: `App::run` answered
/// `frames: 0` for a session that had visibly drawn, because `primary()` is
/// `None` once the window is destroyed and the fallback invents an empty
/// report. That hole was not specific to the programmatic close — it swallowed
/// the measurement of *every* application that closed normally, and had done so
/// for longer. `frames >= 1` is the assertion, and it fails against either bug.
fn an_application_can_close_its_own_window_and_still_gets_its_report() {
    vieww_hardware::skip_without!(display, gpu);
    arm_deadline("an_application_can_close_its_own_window_and_still_gets_its_report");

    let app = App::new().title("vieww wait-loop: self-close");
    let windows = app.windows();
    let waker = app.waker();
    let elapsed = Arc::new(AtomicU32::new(0));

    // From off the UI thread and after the loop has settled, which is the case
    // that was broken: a close asked for while the loop is already busy would
    // have been serviced by the next turn it was going to take anyway.
    std::thread::spawn({
        let elapsed = Arc::clone(&elapsed);
        move || {
            std::thread::sleep(IDLE);
            elapsed.store(1, Ordering::SeqCst);
            trace("waking");
            waker.wake();
        }
    });

    let asked = Rc::new(Cell::new(false));
    let report = app
        .on_frame({
            let elapsed = Arc::clone(&elapsed);
            let asked = Rc::clone(&asked);
            move |_log| {
                trace("frame");
                if elapsed.load(Ordering::SeqCst) == 1 && !asked.get() {
                    asked.set(true);
                    trace("closing");
                    let _ = Windows::close(&*windows, WindowKey::PRIMARY);
                }
            }
        })
        .run(|driver| driver.set_root(Still))
        // Reaching this line at all is the first assertion. Before the fix the
        // loop ran on with no window and the watchdog aborted the process.
        .expect("the application closed its own window; that is not a failure");

    assert!(
        asked.get(),
        "the close was never asked for, so nothing was tested"
    );
    trace(&format!("run returned: {report:?}"));

    assert!(
        report.frames >= 1,
        "the session drew and the report says {} frames — the measurement was \
         discarded when the window was destroyed, which is what happens to \
         every application that closes normally",
        report.frames
    );
    assert!(
        report.rasterised > 0,
        "a report with no rasterised pixels for a session that presented a \
         frame is the invented empty report, not a measurement"
    );
    pass("self-close");
}

// A former scenario here, `two_windows_share_one_gpu_device`, counted
// `vieww_paint::gpu::GpuContext`'s shared-device bookkeeping across two
// windows. That type is gone with vello — the new Vulkan backend
// (`vieww_platform_winit::native`) builds every window its own
// `vieww_hal::vulkan::VulkanDevice` with no shared-instance equivalent yet;
// see that module's own doc comment. Re-add a scenario here once device
// sharing exists to measure.

/// The scenarios, by the name a child process is given.
///
/// One table rather than three `if` arms so that the parent's list and the
/// child's dispatch cannot drift apart — adding a scenario is one line, and a
/// name the parent spawns that the child does not know is impossible rather
/// than merely unlikely.
const SCENARIOS: &[(&str, fn())] = &[
    (
        "idle",
        an_idle_application_stops_drawing_rather_than_spinning,
    ),
    (
        "wake",
        a_waker_brings_a_sleeping_loop_back_and_it_settles_again,
    ),
    ("accounting", idle_time_is_not_counted_as_the_worst_frame),
    (
        "self_close",
        an_application_can_close_its_own_window_and_still_gets_its_report,
    ),
];

/// Parent: run each scenario in a child. Child: run the one it was named.
///
/// The gate is checked in *both* roles and for different reasons. In the child
/// it is what makes a scenario skip rather than fail. In the parent it is what
/// stops three processes being spawned only to skip — and, more importantly, it
/// is where `VIEWW_REQUIRE_HARDWARE=display` turns the whole run into a failure
/// rather than a silent success. The environment variable is inherited, so a
/// child reaches the same verdict the parent did.
fn main() {
    let mut args = std::env::args().skip(1);
    if let Some(name) = args.next() {
        let (_, scenario) = SCENARIOS
            .iter()
            .find(|(known, _)| *known == name)
            .unwrap_or_else(|| panic!("no scenario named {name:?}"));
        scenario();
        return;
    }

    vieww_hardware::skip_without!(display, gpu);

    let binary = std::env::current_exe().expect("a test binary knows its own path");
    let mut failed = Vec::new();
    for (name, _) in SCENARIOS {
        eprintln!("wait_loop: {name}");
        let status = std::process::Command::new(&binary)
            .arg(name)
            .status()
            .unwrap_or_else(|error| panic!("could not re-execute {binary:?}: {error}"));
        if status.success() {
            eprintln!("wait_loop: {name} ok");
        } else {
            eprintln!("wait_loop: {name} FAILED ({status})");
            failed.push(*name);
        }
    }

    assert!(failed.is_empty(), "wait-loop scenarios failed: {failed:?}");
    eprintln!("wait_loop: {} scenarios ok", SCENARIOS.len());
}
