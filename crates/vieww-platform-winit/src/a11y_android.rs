//! The semantics tree, on Android, without becoming a `GameActivity`.
//!
//! Every other platform reaches the screen reader through
//! [`accesskit_winit::Adapter`]. Android cannot, and the reason is one hardcoded
//! field lookup rather than anything about this framework:
//!
//! ```text
//! // accesskit_winit-0.33.2/src/platform_impl/android.rs
//! env.get_field(&activity, "mSurfaceView",
//!     "Lcom/google/androidgamesdk/GameActivity$InputEnabledSurfaceView;")
//! ```
//!
//! That field exists on `GameActivity` and on nothing else. We run a
//! `NativeActivity` on purpose — see `Cargo.toml`, *"the one that needs no Java
//! of our own"* — so enabling `accesskit_winit`'s `accesskit_android` feature
//! would not bridge accessibility. It would `.unwrap()` on a missing field and
//! take the app down before its first frame.
//!
//! So this module skips the shim and drives [`accesskit_android`] itself. The
//! only thing `accesskit_winit`'s version does that we do differently is *find
//! the view*: [`InjectingAdapter::new`] takes any `android.view.View`, and the
//! one a `NativeActivity` draws into is reachable through public `Activity` API.
//!
//! # Why this needs no Gradle, no AAR and no Java
//!
//! `accesskit_android` injects a Java `Delegate` class, and with the
//! `embedded-dex` feature it loads that class out of a `classes.dex` embedded in
//! the crate via `InMemoryDexClassLoader`. Nothing has to be on the compile
//! classpath, which is what makes this reachable from `cargo apk` at all.
//!
//! # Threading
//!
//! `android_main` is not the UI thread, and every call here is made from it.
//! That is allowed, and stated by the crate: *"None of this type's public
//! functions make assumptions about whether they're called from the Android UI
//! thread. As such, some requests are posted to the UI thread and handled
//! asynchronously."* It marshals through `View.post`. The one thing we owe it is
//! a thread that stays attached to the JVM, because `update_if_active` calls
//! `JavaVM::get_env` and unwraps it — hence `attach_current_thread_permanently`
//! rather than the scoped attach `insets.rs` uses.
//!
//! # Two `jni` crates in one binary, on purpose
//!
//! `insets.rs` uses `jni` 0.22, pinned to the copy `ndk` resolves.
//! `accesskit_android` 0.7.5 depends on `jni` 0.21, so both are linked. That is
//! the hazard `Cargo.toml` warns about — *"a second copy would give us a
//! `JavaVM` type that looks identical and is not"* — and it is safe here only
//! because the two never exchange a value. This module imports **everything**
//! JNI from [`accesskit_android::jni`], and the only thing crossing the boundary
//! is a raw pointer out of `AndroidApp`, which belongs to neither.

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use accesskit_android::jni::objects::{GlobalRef, JObject, JValue};
use accesskit_android::jni::{JNIEnv, JavaVM};
use accesskit_android::InjectingAdapter;
use winit::event_loop::EventLoopProxy;
use winit::platform::android::activity::AndroidApp;

/// What a screen reader asks of the loop, on the loop's own thread.
///
/// The handlers below run on whatever thread Android calls them from — the UI
/// thread, in practice — and the element tree belongs to the loop. So nothing is
/// answered in place; it is posted, exactly as `accesskit_winit` posts its own
/// [`accesskit_winit::Event`], and `App::user_event` does the work.
#[derive(Debug)]
pub(crate) enum Request {
    /// A screen reader attached and wants the tree. There is no frame coming,
    /// because nothing changed.
    InitialTree,
    /// A screen reader asked for something to be carried out.
    Action(ActionRequest),
}

/// The API level this binding needs, and it is higher than the app's floor.
///
/// Two independent requirements, both inside `accesskit_android` and both
/// `.unwrap()`ed rather than reported:
///
/// - `InMemoryDexClassLoader`, which the `embedded-dex` feature uses to load the
///   `Delegate` class, is **API 26**.
/// - `View.getAccessibilityDelegate()`, which its injection reads before
///   installing over it, is **API 29**. There is no public getter before that.
///
/// So 29 is the real floor, and `min_sdk_version` is 24. A device between the
/// two is a device where accessibility is declined here, deliberately and with a
/// log line, rather than one where the app dies on launch inside a JNI unwrap.
const A11Y_API: i32 = 29;

/// `android.R.id.content` — the framework id of the content frame every
/// activity puts its view inside.
///
/// Hardcoded because it is a public platform constant and has had this value
/// since API 1; reading it back through `Resources.getIdentifier` would be a
/// round trip to be told what the SDK documents.
const ANDROID_R_ID_CONTENT: i32 = 0x0102_0002;

/// `MotionEvent.ACTION_HOVER_MOVE`.
///
/// Hardcoded for the reason [`A11Y_API`] is: it is a public platform constant
/// with a fixed value, and reading it back through JNI would be three calls per
/// hover event to learn a number that has never changed.
///
/// **`ACTION_HOVER_ENTER` is deliberately absent.** AccessKit handles enter and
/// move in one branch — see `Adapter::on_hover_event` — so sending `MOVE` for
/// both is not an approximation, it takes the identical path. One constant that
/// is always right beats two where the caller has to track which edge it is on.
pub(crate) const ACTION_HOVER_MOVE: i32 = 7;

/// `MotionEvent.ACTION_HOVER_EXIT`, which clears the hover target.
pub(crate) const ACTION_HOVER_EXIT: i32 = 10;

/// Whether this device can carry the injected delegate.
///
/// Split out and pure so the decision is testable on a machine with no phone
/// attached — the rest of this module cannot be, and this is the part with a
/// boundary in it.
#[must_use]
pub(crate) const fn supports_injection(sdk: i32) -> bool {
    sdk >= A11Y_API
}

/// What came of trying to install the delegate.
///
/// Three outcomes rather than an `Option`, because **"not yet" and "never" want
/// opposite things from the caller** and an `Option` makes them the same
/// answer. Retrying a permanent decline costs a handful of JNI calls every
/// frame for the life of the application; not retrying a temporary one leaves
/// accessibility off for ever on a device that would have worked a frame later.
#[derive(Debug)]
pub(crate) enum Injection {
    /// Installed. Drive it.
    Done(AndroidAdapter),
    /// The host view is not attached to a window yet — ask again next frame.
    ///
    /// This is the case that cost a device session. `InjectingAdapter::new`
    /// installs its delegate by **posting a Runnable to the host view**, and
    /// `View.post` returns `false` and *drops* the Runnable when the view is not
    /// attached. `accesskit_android` 0.7.5 does not check that boolean, so the
    /// install fails silently and the adapter then sits there accepting a full,
    /// well-formed tree every frame that Android never asks it for.
    NotYet,
    /// Will never work here: too old an API, no view to be found, or a delegate
    /// already installed. Asking again would only repeat the JNI calls.
    Declined,
}

/// The AccessKit adapter for Android, injected into the activity's own view.
///
/// Deliberately the same two methods `accesskit_winit::Adapter` exposes to
/// `app.rs`, so the call sites there differ only in how one is constructed.
#[derive(Debug)]
pub(crate) struct AndroidAdapter {
    inner: InjectingAdapter,
    /// The view the delegate was installed on, kept alive across frames.
    ///
    /// A `GlobalRef` rather than the `JObject` from construction: a local
    /// reference is only valid for the JNI call that produced it, and using one
    /// on a later frame is undefined behaviour that usually looks like it works.
    host: GlobalRef,
    /// The VM, as the pointer the activity handed us.
    ///
    /// Stored raw and reconstructed per call rather than held as a `JavaVM`,
    /// because `attach_current_thread_permanently` borrows the `JavaVM` for as
    /// long as its `JNIEnv` lives — so the one in `with_event_loop_proxy` cannot
    /// also be moved into this struct. The pointer is valid for the life of the
    /// process; `AndroidApp` hands out the same one.
    vm: *mut std::ffi::c_void,
    /// How far the host view sits inside the window. See [`host_offset`].
    ///
    /// Subtracted from every published bound and from every hover coordinate,
    /// so that `accesskit_android` adding `getLocationOnScreen` back produces
    /// the rectangle the control is actually at.
    offset: (i32, i32),
}

impl AndroidAdapter {
    /// Attach to the running activity, or report why not.
    ///
    /// `None` is a working app without a screen reader bridge, never a crash: an
    /// accessibility bridge that fails to attach must not be the reason an
    /// application will not start. Every path out logs what it found, because
    /// the failure this replaces — a silent no-op adapter — is exactly the one
    /// that cost this project a device session to diagnose.
    pub(crate) fn with_event_loop_proxy<T>(
        android: &AndroidApp,
        proxy: EventLoopProxy<T>,
    ) -> Injection
    where
        T: From<Request> + Send + 'static,
    {
        if android.vm_as_ptr().is_null() || android.activity_as_ptr().is_null() {
            log::warn!("a11y: no activity to attach to");
            return Injection::Declined;
        }

        // Permanent rather than scoped: `InjectingAdapter::update_if_active`
        // calls `JavaVM::get_env` and unwraps it, so this thread has to still be
        // attached on every later frame, not just this one.
        let Ok(vm) = unsafe { JavaVM::from_raw(android.vm_as_ptr().cast()) }
            .inspect_err(|error| log::warn!("a11y: could not reach the JVM: {error}"))
        else {
            return Injection::Declined;
        };
        let Ok(mut env) = vm
            .attach_current_thread_permanently()
            .inspect_err(|error| log::warn!("a11y: could not attach this thread: {error}"))
        else {
            return Injection::Declined;
        };

        let activity = unsafe { JObject::from_raw(android.activity_as_ptr().cast()) };

        let Ok(sdk) = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
            .and_then(|value| value.i())
            .inspect_err(|error| log::warn!("a11y: could not read SDK_INT: {error}"))
        else {
            return Injection::Declined;
        };
        if !supports_injection(sdk) {
            log::info!(
                "a11y: device is API {sdk}, and the injected delegate needs {A11Y_API}; \
                 TalkBack will not see this tree"
            );
            return Injection::Declined;
        }

        let Some(host) = host_view(&mut env, &activity) else {
            // Every candidate was missing, which before layout means the
            // activity has simply not put its view in yet.
            return Injection::NotYet;
        };

        // **The whole reason this function can answer `NotYet`.**
        //
        // `InjectingAdapter::new` installs its delegate through `View.post`,
        // which returns `false` and drops the Runnable when the view is not
        // attached to a window — and `accesskit_android` 0.7.5 does not check
        // that boolean. So injecting here would report success, publish a full
        // tree every frame, and be asked for none of it.
        match env
            .call_method(&host, "isAttachedToWindow", "()Z", &[])
            .and_then(|value| value.z())
        {
            Ok(true) => {}
            Ok(false) => {
                log::info!("a11y: the host view is not attached yet; will try again");
                return Injection::NotYet;
            }
            Err(error) => {
                log::warn!("a11y: could not ask whether the host is attached: {error}");
                return Injection::Declined;
            }
        }

        // **Attached is not laid out**, and the difference is the whole of touch
        // exploration. Android attaches a view before it measures it, so the
        // first frames report 0x0 — and a host with no area is invisible to hit
        // testing. TalkBack would still walk the tree on a swipe and find
        // nothing at all under a finger, which is exactly the bug reported on
        // 2026-08-15: every label read aloud, and touching the screen did
        // nothing, where Chrome highlights and speaks.
        //
        // Waiting is the fix. Rerouting to a laid-out ancestor is not: that
        // ancestor is not where input goes.
        let width = view_int(&mut env, &host, "getWidth");
        let height = view_int(&mut env, &host, "getHeight");
        if width == 0 || height == 0 {
            log::info!("a11y: the host view is attached but not laid out ({width}x{height}); will try again");
            return Injection::NotYet;
        }

        // Checked here rather than letting the crate find out: its own check runs
        // inside a `Runnable` posted to the UI thread and `panic!`s there, which
        // on Android is an abort with a stack nobody can attribute to this.
        match env
            .call_method(
                &host,
                "getAccessibilityDelegate",
                "()Landroid/view/View$AccessibilityDelegate;",
                &[],
            )
            .and_then(|value| value.l())
        {
            Ok(existing) if !existing.is_null() => {
                log::warn!(
                    "a11y: the host view already has an accessibility delegate; not injecting"
                );
                return Injection::Declined;
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!("a11y: could not read the host's delegate: {error}");
                return Injection::Declined;
            }
        }

        // Before `host` is consumed below, and global so it survives this call.
        // Without it there is nothing to hand `onHover` a view on later frames.
        let Ok(global_host) = env
            .new_global_ref(&host)
            .inspect_err(|error| log::warn!("a11y: could not pin the host view: {error}"))
        else {
            return Injection::Declined;
        };

        // Before the adapter, because it needs `env` and `host` and both are
        // consumed below.
        let offset = host_offset(&mut env, &activity, &host);
        if offset != (0, 0) {
            log::info!(
                "a11y: the host sits at {:?} inside the window, so bounds are \
                 corrected by that much",
                offset
            );
        }

        let inner = InjectingAdapter::new(
            &mut env,
            &host,
            Activation {
                proxy: proxy.clone(),
            },
            Action { proxy },
        );
        log::info!("a11y: injected an AccessKit delegate into the activity's view");
        Injection::Done(Self {
            inner,
            host: global_host,
            vm: android.vm_as_ptr(),
            offset,
        })
    }

    /// Hand a hover event to the delegate, as though the view had received it.
    ///
    /// # Why this exists at all
    ///
    /// With explore-by-touch on, a finger dragged over the screen is the only
    /// signal a screen reader has for "what is under the user", and it arrives
    /// as a hover `MotionEvent`. On a `NativeActivity` those never reach the
    /// View hierarchy — the activity owns the window's `InputQueue` — and
    /// **winit 0.30.13 then discards them**, so the delegate AccessKit installs
    /// as an `OnHoverListener` is never called. Measured on 2026-08-15: 1955 raw
    /// input lines during a run, and zero hover probes reaching Rust.
    ///
    /// `ci/tools/patch-winit.sh` is the half that makes the events arrive. This is the
    /// half that delivers them.
    ///
    /// # Why it synthesises a `MotionEvent` instead of calling AccessKit
    ///
    /// `accesskit_android::Adapter::on_hover_event` is exactly the right
    /// function and is **not reachable**: `InjectingAdapter` exposes no wrapper
    /// for it, and the only caller is a `static native` method on the Java
    /// delegate whose `adapter_handle` is private. So the delivery goes the way
    /// Android itself would have delivered it — `MotionEvent.obtain`, then
    /// `onHover(View, MotionEvent)` on the delegate reached through
    /// `View.getAccessibilityDelegate()`. That is public platform API on both
    /// sides and needs no change to AccessKit, which is what `docs/HANDOFF.md`
    /// predicted.
    ///
    /// # Everything here fails quietly
    ///
    /// A missing delegate, a refused `obtain`, a method that does not resolve —
    /// all of them mean "this device does not do touch exploration through this
    /// path", and none of them is a reason to take the application down. They
    /// are logged at debug rather than warn because this runs per hover event,
    /// which during exploration is per frame.
    pub(crate) fn on_hover(&mut self, action: i32, x: f32, y: f32) {
        // The same shift the bounds get. Both have to move together: the hit
        // test compares one against the other, so correcting only the bounds
        // would fix the rectangle and break the speech.
        #[expect(
            clippy::cast_precision_loss,
            reason = "a view offset is a small number of pixels"
        )]
        let (x, y) = (x - self.offset.0 as f32, y - self.offset.1 as f32);

        // SAFETY: the pointer came from `AndroidApp::vm_as_ptr` and the JVM
        // outlives the process's Rust code. Reconstructing a handle to it is
        // what `accesskit_android` and `ndk-context` both do.
        let Ok(vm) = (unsafe { JavaVM::from_raw(self.vm.cast()) }) else {
            return;
        };
        let Ok(mut env) = vm.get_env() else {
            return;
        };

        let host = self.host.as_obj();
        let Ok(delegate) = env
            .call_method(
                host,
                "getAccessibilityDelegate",
                "()Landroid/view/View$AccessibilityDelegate;",
                &[],
            )
            .and_then(|value| value.l())
        else {
            log::debug!("a11y: the host view has no accessibility delegate to hover");
            return;
        };
        if delegate.is_null() {
            return;
        }

        // The timestamps a real event would carry. `MotionEvent.obtain` rejects
        // nothing here, but a screen reader that reasons about event ordering
        // gets a monotonic clock rather than zeros.
        let now = env
            .call_static_method("android/os/SystemClock", "uptimeMillis", "()J", &[])
            .and_then(|value| value.j())
            .unwrap_or(0);

        let Ok(event) = env
            .call_static_method(
                "android/view/MotionEvent",
                "obtain",
                "(JJIFFI)Landroid/view/MotionEvent;",
                &[
                    JValue::Long(now),
                    JValue::Long(now),
                    JValue::Int(action),
                    JValue::Float(x),
                    JValue::Float(y),
                    JValue::Int(0),
                ],
            )
            .and_then(|value| value.l())
        else {
            log::debug!("a11y: MotionEvent.obtain refused a synthesised hover");
            return;
        };

        // Resolved against the delegate's *runtime* class, which is AccessKit's
        // and implements `View.OnHoverListener`. A delegate that does not is a
        // device where this path simply is not available.
        if let Err(error) = env.call_method(
            &delegate,
            "onHover",
            "(Landroid/view/View;Landroid/view/MotionEvent;)Z",
            &[JValue::Object(host), JValue::Object(&event)],
        ) {
            log::debug!("a11y: the delegate would not take a hover: {error}");
            // Cleared so the next call is not made against a pending exception,
            // which in JNI aborts rather than fails.
            let _ = env.exception_clear();
        }

        // Returned to the platform's pool. A synthesised event per hover frame
        // that is never recycled is a slow leak on the one code path that runs
        // continuously while somebody explores the screen.
        let _ = env.call_method(&event, "recycle", "()V", &[]);
    }

    /// Hand the current semantics tree over, if anything is listening.
    ///
    /// The closure is not called when no screen reader is attached, which is the
    /// property the desktop path already relies on: building a semantics tree is
    /// a walk of the whole render tree, and it must not be a per-frame tax on
    /// the phones where nothing will read it.
    pub(crate) fn update_if_active(&mut self, updater: impl FnOnce() -> TreeUpdate) {
        let (dx, dy) = self.offset;
        self.inner.update_if_active(move || {
            let mut update = updater();
            // Into the host view's coordinate space, because that is the space
            // `accesskit_android` will add `getLocationOnScreen` back to. See
            // `host_offset` for the measurement and for what it looked like
            // when this was missing.
            if (dx, dy) != (0, 0) {
                let (dx, dy) = (f64::from(dx), f64::from(dy));
                for (_, node) in &mut update.nodes {
                    if let Some(rect) = node.bounds() {
                        node.set_bounds(accesskit::Rect {
                            x0: rect.x0 - dx,
                            y0: rect.y0 - dy,
                            x1: rect.x1 - dx,
                            y1: rect.y1 - dy,
                        });
                    }
                }
            }
            update
        });
    }

    /// Present so that `app.rs` can call it unconditionally.
    ///
    /// `accesskit_winit` needs window events to notice focus and geometry
    /// changes. The injected delegate is driven by the view it lives on, so
    /// there is nothing to forward — and `accesskit_winit`'s own Android arm
    /// has an empty body here too.
    // `allow` rather than `expect`: this workspace does not turn on
    // `clippy::pedantic`, so an expectation for a pedantic lint is one clippy
    // may never fulfil, and an unfulfilled expectation is itself a warning.
    #[allow(
        clippy::unused_self,
        reason = "matches accesskit_winit::Adapter's shape, so app.rs needs no cfg"
    )]
    pub(crate) fn process_event(
        &mut self,
        _window: &winit::window::Window,
        _event: &winit::event::WindowEvent,
    ) {
    }
}

/// Where a view sits on the screen, in physical pixels.
fn view_location(env: &mut JNIEnv<'_>, view: &JObject) -> Option<(i32, i32)> {
    let array = env.new_int_array(2).ok()?;
    env.call_method(view, "getLocationOnScreen", "([I)V", &[(&array).into()])
        .ok()?;
    let mut buf = [0_i32; 2];
    env.get_int_array_region(&array, 0, &mut buf).ok()?;
    Some((buf[0], buf[1]))
}

/// How far the host view sits inside the window vieww draws into.
///
/// # Why this is not always zero, and what it broke
///
/// `accesskit_android` computes the rectangle a screen reader draws as
/// **node bounds + `host.getLocationOnScreen()`** (`node.rs:213`). That is
/// correct if the node bounds are relative to the host view. vieww's are
/// relative to its **surface**, and on a `NativeActivity` those are two
/// different origins:
///
/// ```text
/// frame=[0,0][1080,2340]                <- the window, and vieww's surface
/// mAppBounds=Rect(0, 80 - 1080, 2274)   <- android.R.id.content, the host
/// ```
///
/// So every node was published 80px too low. Measured on a Redmi Note 7 Pro on
/// 2026-08-17: vieww put the demo's button at y=538, and `uiautomator dump`
/// reported it at y=617 — a constant 79px, x unaffected, which is the status
/// bar exactly.
///
/// **Hit testing hid it.** `virtual_view_at_point` compares the hover
/// coordinates against the same bounds, and both were surface-relative, so
/// touch exploration spoke the *right* control while TalkBack drew its green
/// rectangle 79px below it. Speech and highlight disagreeing is the only symptom
/// this produces, and it is why the bug survived the tree being called correct.
///
/// The decor view is the one whose origin *is* the window origin, so it is the
/// reference rather than an arbitrary choice. Returning the difference lets the
/// host stay whatever [`host_view`] picked — including the render surface, when
/// it has size — instead of forcing a host for the sake of the arithmetic.
fn host_offset(env: &mut JNIEnv<'_>, activity: &JObject, host: &JObject) -> (i32, i32) {
    let Some((host_x, host_y)) = view_location(env, host) else {
        return (0, 0);
    };
    let window = env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])
        .and_then(|value| value.l());
    let decor = window
        .as_ref()
        .ok()
        .and_then(|window| {
            env.call_method(window, "getDecorView", "()Landroid/view/View;", &[])
                .and_then(|value| value.l())
                .ok()
        })
        .and_then(|decor| view_location(env, &decor));

    // No decor to measure against means no correction rather than a guess: a
    // wrong offset moves every rectangle, where none leaves them where they
    // already were.
    let Some((decor_x, decor_y)) = decor else {
        return (0, 0);
    };
    (host_x - decor_x, host_y - decor_y)
}

/// The view to hang the delegate on, in descending order of how specific it is.
///
/// A `NativeActivity` builds a view of its own inside the standard content
/// frame, and that view is the one covering the screen the user is touching — so
/// it is the right host. The two fallbacks exist because none of this is
/// contractual: `getChildAt` can legitimately return null before the activity
/// has finished laying out, and the content frame and the decor view are both
/// still views a delegate works on.
///
/// Logged at each step, because "which view did it land on" is the first
/// question to ask if TalkBack reads nothing, and it is not answerable
/// afterwards.
fn host_view<'local>(env: &mut JNIEnv<'local>, activity: &JObject) -> Option<JObject<'local>> {
    let window = env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])
        .and_then(|value| value.l())
        .inspect_err(|error| log::warn!("a11y: no window on the activity: {error}"))
        .ok()?;
    let decor = env
        .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])
        .and_then(|value| value.l())
        .inspect_err(|error| log::warn!("a11y: no decor view: {error}"))
        .ok()?;

    let Some(content) = view_call(
        env,
        &decor,
        "findViewById",
        "(I)Landroid/view/View;",
        &[ANDROID_R_ID_CONTENT.into()],
    ) else {
        log::info!("a11y: no content frame; host is the decor view");
        return Some(decor);
    };

    // `getChildAt` is a `ViewGroup` method, and JNI resolves against the
    // object's real class — so this needs no cast, the content frame being a
    // `FrameLayout`. Null here is ordinary rather than an error: it means the
    // activity has not put its view in yet.
    let Some(child) = view_call(
        env,
        &content,
        "getChildAt",
        "(I)Landroid/view/View;",
        &[0.into()],
    ) else {
        log::info!("a11y: host is android.R.id.content");
        return Some(content);
    };

    // **This is the right host even when it is momentarily 0x0**, and getting
    // that wrong cost a round trip. `NativeActivity` does
    // `setContentView(mNativeContentView)`, so child 0 *is* the surface the app
    // draws into and *is* the view Android dispatches input to — including the
    // hover events that touch exploration is made of.
    //
    // It reports no size for the first frames because Android attaches a view
    // before it measures one. An earlier version read that as "this view is a
    // dummy" and rerouted to `android.R.id.content`, which is laid out but
    // receives no input: that made the tree visible to `uiautomator` and left
    // touch exploration exactly as broken, because hover never reaches a parent
    // frame that draws nothing.
    //
    // The size is therefore checked by the caller, which can answer
    // `Injection::NotYet` and try again — see `inject`.
    log::info!("a11y: host is the activity's content view");
    Some(child)
}

/// One `View` method returning an `int`, with any failure folded into `0`.
///
/// Zero is the safe answer for the sizes this reads: it means "not a usable
/// host", which is the conservative branch.
fn view_int(env: &mut JNIEnv<'_>, view: &JObject, method: &str) -> i32 {
    env.call_method(view, method, "()I", &[])
        .and_then(|value| value.i())
        .unwrap_or(0)
}

/// One `View`-returning call, with null folded into `None`.
///
/// Android returns null for "there is nothing there" across all of these, and
/// every caller above treats that the same as a failed call — so collapsing the
/// two keeps the fallback chain readable rather than four levels deep.
fn view_call<'local>(
    env: &mut JNIEnv<'local>,
    receiver: &JObject,
    method: &str,
    signature: &str,
    args: &[JValue<'_, '_>],
) -> Option<JObject<'local>> {
    let value = env
        .call_method(receiver, method, signature, args)
        .and_then(|value| value.l())
        .inspect_err(|error| log::warn!("a11y: {method} failed: {error}"))
        .ok()?;
    (!value.is_null()).then_some(value)
}

/// Answers "give me the tree" by asking the loop for it.
///
/// Returning `None` is the documented way to say *not synchronously*: the
/// adapter then treats the next [`AndroidAdapter::update_if_active`] as the
/// initial full tree, which is what `App::publish_semantics` produces. Building
/// it here instead is not an option — the element tree lives on the loop's
/// thread and this runs on Android's.
struct Activation<T: 'static> {
    proxy: EventLoopProxy<T>,
}

impl<T: From<Request> + Send + 'static> ActivationHandler for Activation<T> {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // A closed loop is a shutting-down app, and a screen reader asking for a
        // tree on the way out is not an error worth reporting.
        let _ = self.proxy.send_event(Request::InitialTree.into());
        None
    }
}

/// Forwards what a screen reader asked for to the thread that can carry it out.
struct Action<T: 'static> {
    proxy: EventLoopProxy<T>,
}

impl<T: From<Request> + Send + 'static> ActionHandler for Action<T> {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = self.proxy.send_event(Request::Action(request).into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_devices_this_app_supports_are_not_all_devices_this_bridge_supports() {
        // `min_sdk_version` is 24 and the injected delegate needs 29, so there
        // is a real band of devices that run this app and get no screen reader.
        // Asserted rather than assumed, because the alternative to declining is
        // a JNI unwrap inside `accesskit_android` on launch.
        assert!(
            !supports_injection(24),
            "the app's own min_sdk is below what the bridge needs"
        );
        assert!(
            !supports_injection(26),
            "InMemoryDexClassLoader is reachable at 26, getAccessibilityDelegate is not"
        );
        assert!(!supports_injection(28));
    }

    #[test]
    fn api_29_and_above_can_carry_the_delegate() {
        assert!(
            supports_injection(29),
            "getAccessibilityDelegate lands in 29"
        );
        assert!(supports_injection(30));
        assert!(supports_injection(34));
    }

    #[test]
    fn a_nonsense_api_level_is_declined_rather_than_trusted() {
        // `SDK_INT` comes back through JNI and a failure to read it must not
        // read as a modern device.
        assert!(!supports_injection(0));
        assert!(!supports_injection(-1));
    }
}
