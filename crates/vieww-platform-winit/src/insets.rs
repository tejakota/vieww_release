//! Where the system furniture is, and what that leaves for content.
//!
//! Two kinds of furniture, and the difference is the whole reason this module
//! reports them separately. A cutout and a status bar are permanent, so content
//! is *inset* away from them. A soft keyboard is temporary and content is
//! expected to *scroll out from under* it — see [`vieww_foundation::ViewMetrics`]
//! for why collapsing the two is the bug where the keyboard opening pushes the
//! field being typed into off the screen.
//!
//! The JNI that asks the platform is Android-only. The arithmetic it feeds, and
//! the [`InsetWatch`] that decides when to ask again, are not — so they live
//! here beside it and are tested on every machine. A phone is needed to *get*
//! the numbers, not to check what is done with them.

use vieww_foundation::EdgeInsets;

/// Both kinds of obstruction, as one platform answer.
///
/// One value rather than two calls because on Android they come out of the same
/// `WindowInsets` object: asking separately would mean two JNI round trips, two
/// thread attaches and two chances for the two halves to describe different
/// moments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SystemInsets {
    /// Status bar, navigation bar, cutout, home indicator. Permanent.
    pub safe_area: EdgeInsets,
    /// The soft keyboard, and nothing else. Temporary.
    pub keyboard: EdgeInsets,
}

impl SystemInsets {
    /// What a window with nothing over it reports. Every desktop window.
    pub(crate) const NONE: Self = Self {
        safe_area: EdgeInsets::ZERO,
        keyboard: EdgeInsets::ZERO,
    };
}

#[cfg(target_os = "android")]
thread_local! {
    /// The last insets written to the log, so a repeat can be quiet.
    ///
    /// Thread-local rather than threaded through the caller: the raw physical
    /// numbers in the message are local to this function, and handing them
    /// upwards purely so somebody else can decide whether to print them would
    /// trade a `Cell` for an argument on every call.
    ///
    /// Only ever read and written from the frame that queries, which is one
    /// thread on Android — the local is for the borrow checker rather than for
    /// concurrency.
    static LAST_LOGGED: std::cell::Cell<Option<SystemInsets>> = const { std::cell::Cell::new(None) };
}

/// The first API level with `WindowInsets.getInsets(int)` and
/// `WindowManager.getCurrentWindowMetrics()`.
///
/// Below it this reports nothing and the safe area stays zero, which is what
/// every Android device did before this existed — so an older phone is no worse
/// off than it was. The alternative was the deprecated `getSystemWindowInset*`
/// family, which cannot report a cutout separately at all: a second code path
/// that would still get notches wrong.
#[cfg(target_os = "android")]
const INSETS_API: i32 = 30;

/// Real window insets, straight from the platform, in logical pixels.
///
/// `None` rather than zero when anything goes wrong, so a caller can tell "the
/// platform says there are no insets" from "we failed to ask".
///
/// # Why not `AndroidApp::content_rect`
///
/// It was tried first, because it needs no JNI. **It is the whole window.** A
/// `NativeActivity` never opts into insets, so its content rect covers the
/// display furniture rather than avoiding it, and the subtraction is always
/// zero. Measured on a device that simultaneously reported:
///
/// ```text
/// app=1080x2340                                     <- on a 1080x2340 screen
/// mDisplayCutout=DisplayCutout{insets=Rect(0, 80 - 0, 0)}
/// ```
///
/// The system knew about the 80px cutout. The content rect did not carry it.
///
/// # Why `getCurrentWindowMetrics` and not `getRootWindowInsets`
///
/// `getRootWindowInsets` is a `View` method and `android_main` does not run on
/// the UI thread. `WindowManager.getCurrentWindowMetrics()` reads window state
/// without touching the view hierarchy, so it is the safer of the two from here.
///
/// # The keyboard, and why it comes out of the same object
///
/// `WindowInsets.Type.ime()` is asked for through the same object as the bars,
/// because the IME is a window on the display and its inset source is part of
/// the display's state — which `getCurrentWindowMetrics` reads.
///
/// **Measured, not assumed.** That was the one open question in this file, and
/// a Redmi Note 7 Pro on Android 16 answered it: **290.55 logical pixels of
/// keyboard — 799 physical at 2.75x — on a 2340px screen**, through this call,
/// with the bars and the cutout coming back correct in the same round trip.
/// `ci/mobile/device-suite.sh --expect-keyboard` is what asks, and
/// `the_keyboard_inset_appears_when_the_keyboard_does` is the line to look for.
///
/// The fallback if a device ever disagrees is a `View`-level query marshalled
/// onto the UI thread — a great deal more machinery for the same number, and
/// worth avoiding until some phone makes it necessary.
#[cfg(target_os = "android")]
pub(crate) fn window_insets(
    android: &winit::platform::android::activity::AndroidApp,
    scale: f32,
) -> Option<SystemInsets> {
    use jni::objects::{JObject, JValue};
    use jni::{jni_sig, jni_str};

    if scale <= 0.0 {
        return None;
    }

    // Both pointers belong to the activity that is currently running, and
    // neither is kept past this call.
    let vm = unsafe { jni::JavaVM::from_raw(android.vm_as_ptr().cast()) };
    let activity_ptr = android.activity_as_ptr();

    type Raw = ([i32; 4], [i32; 4]);
    let queried = vm.attach_current_thread(|env| -> Result<Option<Raw>, jni::errors::Error> {
        let activity = unsafe { JObject::from_raw(env, activity_ptr.cast()) };

        let sdk = env
            .get_static_field(
                jni_str!("android/os/Build$VERSION"),
                jni_str!("SDK_INT"),
                jni_sig!("I"),
            )?
            .i()?;
        if sdk < INSETS_API {
            return Ok(None);
        }

        let manager = env
            .call_method(
                &activity,
                jni_str!("getWindowManager"),
                jni_sig!("()Landroid/view/WindowManager;"),
                &[],
            )?
            .l()?;
        let metrics = env
            .call_method(
                &manager,
                jni_str!("getCurrentWindowMetrics"),
                jni_sig!("()Landroid/view/WindowMetrics;"),
                &[],
            )?
            .l()?;
        let window_insets = env
            .call_method(
                &metrics,
                jni_str!("getWindowInsets"),
                jni_sig!("()Landroid/view/WindowInsets;"),
                &[],
            )?
            .l()?;

        // Both types, deliberately. The bars are what content must avoid to
        // be *usable*; the cutout is what it must avoid to be *visible*, and
        // on a phone drawing behind a transparent status bar the cutout can
        // be the larger of the two.
        let bars = env
            .call_static_method(
                jni_str!("android/view/WindowInsets$Type"),
                jni_str!("systemBars"),
                jni_sig!("()I"),
                &[],
            )?
            .i()?;
        let cutout = env
            .call_static_method(
                jni_str!("android/view/WindowInsets$Type"),
                jni_str!("displayCutout"),
                jni_sig!("()I"),
                &[],
            )?
            .i()?;
        // Asked for on its own rather than folded into the mask above. The
        // keyboard must not land in the safe area — that is the whole point of
        // reporting it separately — so it needs its own `getInsets` call.
        let ime = env
            .call_static_method(
                jni_str!("android/view/WindowInsets$Type"),
                jni_str!("ime"),
                jni_sig!("()I"),
                &[],
            )?
            .i()?;

        // Written out twice rather than through a closure over `env`: the
        // borrow a closure would take lasts as long as the closure does, and
        // this function has to keep using `env` afterwards.
        let bars_rect = env
            .call_method(
                &window_insets,
                jni_str!("getInsets"),
                jni_sig!("(I)Landroid/graphics/Insets;"),
                &[JValue::Int(bars | cutout)],
            )?
            .l()?;
        let bars_raw = [
            env.get_field(&bars_rect, jni_str!("left"), jni_sig!("I"))?
                .i()?,
            env.get_field(&bars_rect, jni_str!("top"), jni_sig!("I"))?
                .i()?,
            env.get_field(&bars_rect, jni_str!("right"), jni_sig!("I"))?
                .i()?,
            env.get_field(&bars_rect, jni_str!("bottom"), jni_sig!("I"))?
                .i()?,
        ];

        let ime_rect = env
            .call_method(
                &window_insets,
                jni_str!("getInsets"),
                jni_sig!("(I)Landroid/graphics/Insets;"),
                &[JValue::Int(ime)],
            )?
            .l()?;
        let ime_raw = [
            env.get_field(&ime_rect, jni_str!("left"), jni_sig!("I"))?
                .i()?,
            env.get_field(&ime_rect, jni_str!("top"), jni_sig!("I"))?
                .i()?,
            env.get_field(&ime_rect, jni_str!("right"), jni_sig!("I"))?
                .i()?,
            env.get_field(&ime_rect, jni_str!("bottom"), jni_sig!("I"))?
                .i()?,
        ];

        Ok(Some((bars_raw, ime_raw)))
    });

    // Logged rather than swallowed. A JNI call that quietly fails looks exactly
    // like a device with no furniture on its screen, and the first version of
    // this returned `None` in silence — which cost a whole build-and-look cycle
    // to distinguish from "the numbers really are zero".
    match queried {
        Ok(Some((bars, ime))) => {
            let insets = SystemInsets {
                safe_area: logical_insets(bars, scale),
                keyboard: keyboard_insets(ime, scale),
            };

            // **On change, not on every query.** `InsetWatch` queries every
            // frame of a 30-frame settling window, which is correct — the
            // keyboard animates, so the value at the moment of a toggle is the
            // old one — but it meant thirty identical `info` lines per
            // disturbance. A remount produced twenty-five in 130ms, and they
            // were the bulk of logcat during the hot-reload work: the noise
            // buried the lines somebody was actually reading.
            //
            // The diagnostic value is entirely in the *first* answer and in
            // *changes*; a repeat says only that the settle is still running.
            // Kept at `debug` rather than dropped, so a session chasing an inset
            // can still see every poll.
            let changed = LAST_LOGGED.with(|last| {
                let previous = last.get();
                if previous == Some(insets) {
                    false
                } else {
                    last.set(Some(insets));
                    true
                }
            });
            if changed {
                log::info!(
                    "window insets: bars {bars:?} ime {ime:?} physical at {scale}x -> {insets:?}"
                );
            } else {
                log::debug!("window insets: unchanged at {insets:?}");
            }
            Some(insets)
        }
        Ok(None) => {
            log::info!("window insets: device is below API {INSETS_API}, reporting none");
            None
        }
        Err(error) => {
            log::warn!("window insets: the JNI call failed: {error}");
            None
        }
    }
}

/// The display's current refresh rate in Hz, or `None` if Android will not say.
///
/// # Why this is here rather than from winit
///
/// **winit's Android backend returns `None` unconditionally** — its
/// `refresh_rate_millihertz` is a `FIXME no way to get real refresh rate for
/// now` and a hard-coded `None`. So on the one platform this framework actually
/// ships to, the frame budget silently stayed at the 60Hz default and every
/// jank verdict on a 90 or 120Hz phone was wrong. Confirmed by the log line
/// this module's sibling prints never appearing on a real device.
///
/// It lives in `insets.rs` because this is where the JNI machinery already is,
/// and a second module to attach a thread and make two calls would be the
/// tax rather than the work. The two are unrelated otherwise.
///
/// # `getRefreshRate`, not the mode list
///
/// `Display.getRefreshRate()` reports the rate the display is running at *now*,
/// which is the one a frame budget is about. `Display.getMode()` and the
/// supported-modes list describe what the panel *could* do, and on a phone that
/// switches modes to save battery the highest supported rate is the wrong
/// number — it would make a budget the device has no intention of meeting.
///
/// Queried once per window open, not per frame: unlike the insets, this does
/// not change while an app is on screen without the surface being recreated.
#[cfg(target_os = "android")]
pub(crate) fn display_refresh_hz(
    android: &winit::platform::android::activity::AndroidApp,
) -> Option<f32> {
    use jni::objects::JObject;
    use jni::{jni_sig, jni_str};

    if android.vm_as_ptr().is_null() || android.activity_as_ptr().is_null() {
        return None;
    }

    let vm = unsafe { jni::JavaVM::from_raw(android.vm_as_ptr().cast()) };
    let activity_ptr = android.activity_as_ptr();

    let queried = vm.attach_current_thread(|env| -> Result<Option<f32>, jni::errors::Error> {
        let activity = unsafe { JObject::from_raw(env, activity_ptr.cast()) };

        let sdk = env
            .get_static_field(
                jni_str!("android/os/Build$VERSION"),
                jni_str!("SDK_INT"),
                jni_sig!("I"),
            )?
            .i()?;
        if sdk < INSETS_API {
            return Ok(None);
        }

        // `Context.getDisplay()` is API 30, the same floor the insets already
        // require. The older route is `getWindowManager().getDefaultDisplay()`,
        // deprecated since 30 and not worth carrying for devices this module
        // already declines to serve.
        let display = env
            .call_method(
                &activity,
                jni_str!("getDisplay"),
                jni_sig!("()Landroid/view/Display;"),
                &[],
            )?
            .l()?;
        if display.is_null() {
            // A real answer: an activity not attached to a display yet.
            return Ok(None);
        }
        let hz = env
            .call_method(&display, jni_str!("getRefreshRate"), jni_sig!("()F"), &[])?
            .f()?;
        Ok(Some(hz))
    });

    match queried {
        Ok(Some(hz)) if hz.is_finite() && hz > 0.0 => {
            log::info!("display refresh: {hz:.1}Hz from Display.getRefreshRate()");
            Some(hz)
        }
        Ok(Some(hz)) => {
            log::warn!("display refresh: Android reported {hz}, ignoring it");
            None
        }
        Ok(None) => {
            log::info!("display refresh: no display, or below API {INSETS_API}");
            None
        }
        Err(error) => {
            log::warn!("display refresh: the JNI call failed: {error}");
            None
        }
    }
}

/// Physical insets to logical, clamped at zero on every edge.
///
/// Split from the JNI above because that part is a data fetch and this part is
/// arithmetic that can be wrong. Only Android calls it; every platform tests it.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[must_use]
pub(crate) fn logical_insets([left, top, right, bottom]: [i32; 4], scale: f32) -> EdgeInsets {
    if scale <= 0.0 {
        return EdgeInsets::ZERO;
    }
    // Clamped: a negative inset would push content *outwards*, off the screen.
    let logical = |physical: i32| (physical as f32 / scale).max(0.0);
    EdgeInsets {
        left: logical(left),
        top: logical(top),
        right: logical(right),
        bottom: logical(bottom),
    }
}

/// The keyboard's insets, with everything but the bottom edge discarded.
///
/// A soft keyboard comes up from the bottom on both platforms this targets, and
/// `WindowInsets.Type.ime()` is nonetheless a four-sided rectangle. Passing the
/// other three through would be inviting a future device — a side-docked IME on
/// a foldable, a hardware keyboard drawer — to inset content horizontally in a
/// direction nothing above here is prepared to scroll.
///
/// If that device shows up, this is the one function to change, and the change
/// is deliberate rather than accidental.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[must_use]
pub(crate) fn keyboard_insets(raw: [i32; 4], scale: f32) -> EdgeInsets {
    EdgeInsets::only(0.0, 0.0, 0.0, logical_insets(raw, scale).bottom)
}

/// How many frames to keep re-asking after something disturbed the insets.
///
/// Half a second at 60Hz. The keyboard *animates* in and out over roughly
/// 250ms on both platforms, and a single query at the moment the IME was
/// requested reads the height it had before it started moving — which is zero.
/// Sampling across the animation is also what makes content track the keyboard
/// on the way up rather than jumping once it arrives.
const SETTLE_FRAMES: u32 = 30;

/// How many frames between queries once the keyboard is up and still.
///
/// Four times a second. Not zero, because the keyboard can be dismissed by
/// something this process never sees — the back gesture, a hardware keyboard
/// being attached, another app taking focus — and an inset that stays up
/// forever after that is a permanently short screen. Not every frame, because
/// this costs a JNI round trip and nothing is changing.
const IDLE_STRIDE: u32 = 15;

/// Decides which frames are worth asking the platform about its insets.
///
/// Pure, and separated from the asking for exactly that reason: *when* to query
/// is a policy with edge cases worth testing, and a phone is not needed to test
/// a policy.
///
/// # Why this is not just "query every frame"
///
/// A query is a JNI thread attach and six calls into the JVM on Android. Doing
/// it unconditionally puts that on the critical path of every frame the app
/// ever draws, to detect a change that happens a handful of times in a session.
///
/// # Why this is not just "query when the IME is toggled"
///
/// Two reasons, and both were the first design. The keyboard animates, so the
/// value at the moment of the toggle is the old one. And the keyboard can go
/// away without this process asking it to, so there has to be *some* polling
/// while it is up.
///
/// # What this deliberately does not do
///
/// It does not *request* frames. The stride only advances on frames that were
/// going to happen anyway, so an application that is completely idle with the
/// keyboard up will not notice a system-initiated dismissal until something
/// else wakes it. In practice a focused text field is blinking a caret and
/// therefore is not idle — but that is a happy accident, not a guarantee, and
/// the alternative is pinning a backgrounded-looking app at four full repaints
/// a second for the whole time a keyboard is open. The battery cost of being
/// wrong in that direction is much worse than one stale inset.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InsetWatch {
    /// Frames left in the settling window; queried every frame while non-zero.
    settling: u32,
    /// Frames since the last query, for the idle stride.
    since_query: u32,
    /// What the last query came back with, so a change can extend the settle.
    keyboard_up: bool,
}

impl InsetWatch {
    /// Nothing to watch yet.
    pub(crate) const fn new() -> Self {
        Self {
            settling: 0,
            since_query: 0,
            keyboard_up: false,
        }
    }

    /// Something happened that could move the insets: the IME was switched on
    /// or off, the app resumed, the window was resized.
    ///
    /// Opens a settling window rather than querying immediately, so the caller
    /// stays a single "query or do not" decision per frame.
    pub(crate) fn disturbed(&mut self) {
        self.settling = SETTLE_FRAMES;
    }

    /// Whether this frame should ask the platform. Advances the clock.
    ///
    /// Called once per frame, before the build phase — metrics have to be
    /// current for the tree that is about to be built, not the one after it.
    pub(crate) fn due(&mut self) -> bool {
        if self.settling > 0 {
            self.settling -= 1;
            self.since_query = 0;
            return true;
        }
        if !self.keyboard_up {
            return false;
        }
        self.since_query += 1;
        if self.since_query >= IDLE_STRIDE {
            self.since_query = 0;
            return true;
        }
        false
    }

    /// What the query came back with.
    ///
    /// A change extends the settling window: the keyboard is still moving, and
    /// stopping mid-animation would freeze content halfway up.
    pub(crate) fn observed(&mut self, keyboard: EdgeInsets) {
        let up = keyboard.bottom > 0.0;
        if up != self.keyboard_up {
            self.settling = SETTLE_FRAMES;
        }
        self.keyboard_up = up;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_bar_and_a_gesture_bar_become_top_and_bottom_insets() {
        // 90px of status bar and 60px of navigation, at 3x.
        let insets = logical_insets([0, 90, 0, 60], 3.0);
        assert_eq!(insets.top, 30.0, "90 physical is 30 logical");
        assert_eq!(insets.bottom, 20.0);
        assert_eq!((insets.left, insets.right), (0.0, 0.0));
    }

    #[test]
    fn a_cutout_on_one_side_insets_only_that_side() {
        let insets = logical_insets([120, 0, 0, 0], 3.0);
        assert_eq!(insets.left, 40.0);
        assert_eq!((insets.top, insets.right, insets.bottom), (0.0, 0.0, 0.0));
    }

    #[test]
    fn a_window_with_no_furniture_over_it_is_no_inset_at_all() {
        assert_eq!(logical_insets([0, 0, 0, 0], 3.0), EdgeInsets::ZERO);
    }

    #[test]
    fn a_negative_inset_is_clamped_rather_than_pushing_content_off_screen() {
        assert_eq!(logical_insets([-10, 0, 0, 0], 3.0).left, 0.0);
    }

    #[test]
    fn a_nonsense_scale_reports_nothing_rather_than_infinities() {
        assert_eq!(logical_insets([0, 90, 0, 60], 0.0), EdgeInsets::ZERO);
    }

    #[test]
    fn the_insets_are_logical_so_a_denser_screen_reports_the_same_numbers() {
        // The same physical furniture on a 2x and a 3x screen is the same number
        // of logical pixels, which is the whole point of the division.
        assert_eq!(logical_insets([0, 90, 0, 0], 3.0).top, 30.0);
        assert_eq!(logical_insets([0, 60, 0, 0], 2.0).top, 30.0);
    }

    #[test]
    fn a_keyboard_becomes_a_bottom_inset_and_nothing_else() {
        // 900px of keyboard at 3x, on a device that also reports side insets
        // for the IME — which some do, and which nothing above here can scroll.
        let keyboard = keyboard_insets([12, 0, 12, 900], 3.0);
        assert_eq!(keyboard.bottom, 300.0);
        assert_eq!(
            (keyboard.left, keyboard.top, keyboard.right),
            (0.0, 0.0, 0.0),
            "a horizontal keyboard inset would inset content in a direction \
             nothing is prepared to scroll"
        );
    }

    #[test]
    fn no_keyboard_is_no_inset() {
        assert_eq!(keyboard_insets([0, 0, 0, 0], 3.0), EdgeInsets::ZERO);
    }

    #[test]
    fn an_undisturbed_window_with_no_keyboard_never_queries() {
        let mut watch = InsetWatch::new();
        for frame in 0..600 {
            assert!(
                !watch.due(),
                "frame {frame} asked the platform for nothing: a JNI round trip \
                 on every idle frame is the cost this type exists to avoid"
            );
        }
    }

    #[test]
    fn a_disturbance_queries_every_frame_across_the_keyboard_animation() {
        let mut watch = InsetWatch::new();
        watch.disturbed();
        for frame in 0..SETTLE_FRAMES {
            assert!(watch.due(), "frame {frame} is inside the settling window");
            watch.observed(EdgeInsets::ZERO);
        }
        assert!(
            !watch.due(),
            "the window closes once the keyboard has had time to arrive"
        );
    }

    #[test]
    fn the_keyboard_appearing_extends_the_settle_so_the_animation_is_tracked() {
        let mut watch = InsetWatch::new();
        watch.disturbed();

        // The animation is halfway through the first window when the height
        // first comes back non-zero.
        for _ in 0..SETTLE_FRAMES / 2 {
            assert!(watch.due());
            watch.observed(EdgeInsets::ZERO);
        }
        assert!(watch.due());
        watch.observed(EdgeInsets::only(0.0, 0.0, 0.0, 100.0));

        // A fresh full window from *that* frame, not the remainder of the old
        // one: content that stopped tracking here would freeze halfway up.
        for frame in 0..SETTLE_FRAMES {
            assert!(
                watch.due(),
                "frame {frame} after the keyboard appeared must still sample"
            );
            watch.observed(EdgeInsets::only(0.0, 0.0, 0.0, 300.0));
        }
    }

    #[test]
    fn an_open_keyboard_is_polled_slowly_rather_than_not_at_all() {
        let mut watch = InsetWatch::new();
        watch.disturbed();
        // Settle with the keyboard up.
        for _ in 0..=SETTLE_FRAMES {
            if watch.due() {
                watch.observed(EdgeInsets::only(0.0, 0.0, 0.0, 300.0));
            }
        }

        let queries = (0..IDLE_STRIDE * 4)
            .filter(|_| {
                let due = watch.due();
                if due {
                    watch.observed(EdgeInsets::only(0.0, 0.0, 0.0, 300.0));
                }
                due
            })
            .count();
        assert_eq!(
            queries, 4,
            "a keyboard dismissed by the system back gesture is never reported \
             to this process, so a still keyboard still needs polling"
        );
    }

    #[test]
    fn a_keyboard_that_goes_away_stops_the_polling_again() {
        let mut watch = InsetWatch::new();
        watch.disturbed();
        for _ in 0..=SETTLE_FRAMES {
            if watch.due() {
                watch.observed(EdgeInsets::only(0.0, 0.0, 0.0, 300.0));
            }
        }

        // The next poll finds it gone.
        while !watch.due() {}
        watch.observed(EdgeInsets::ZERO);

        // That is a change, so it settles again, and then goes quiet for good.
        for _ in 0..=SETTLE_FRAMES {
            if watch.due() {
                watch.observed(EdgeInsets::ZERO);
            }
        }
        for frame in 0..IDLE_STRIDE * 4 {
            assert!(
                !watch.due(),
                "frame {frame} is back to an idle window and must cost nothing"
            );
        }
    }
}
